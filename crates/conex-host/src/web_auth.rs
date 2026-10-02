use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Extension;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use conex_core::CallError;
use conex_proto;
use serde_json::json;
use tokio::sync::Notify;

use crate::agent::{TicketRegistry, now_ms};
use crate::auth::InboundCaller;
use crate::http::HttpState;
use crate::ui_links::UiLinkRegistry;

pub const SESSION_COOKIE: &str = "conex_web_session";
pub const CSRF_HEADER: &str = "x-csrf-token";
const SESSION_TTL_MS: u64 = 8 * 60 * 60 * 1000;
/// Authenticated users: sessions per principal (plan M1.1 keeps this
/// independent from anonymous visitor capacity).
const MAX_SESSIONS_PER_PRINCIPAL: usize = 8;
const MAX_SESSIONS_GLOBAL: usize = 1024;
/// How long a removed session id is remembered for the guest-cookie reissue
/// decision (guest cookies may reissue; authenticated cookies must not
/// silently downgrade).
const TOMBSTONE_TTL_MS: u64 = 60 * 60 * 1000;
const TOMBSTONE_CAPACITY: usize = 4096;

/// Anonymous visitor policy resolved from `WebGuestConfig` (M1.1): the
/// principal/tenant come from configuration, never hardcoded.
#[derive(Debug, Clone)]
pub struct GuestPolicy {
    pub principal_id: String,
    pub tenant_id: String,
    pub max_sessions: usize,
    pub idle_ttl_ms: u64,
    pub max_issue_per_minute: u32,
}

impl GuestPolicy {
    pub fn from_config(config: &crate::config::WebGuestConfig) -> Self {
        Self {
            principal_id: config.principal_id.clone(),
            tenant_id: config.tenant_id.clone(),
            max_sessions: config.max_sessions,
            idle_ttl_ms: config.idle_ttl_ms,
            max_issue_per_minute: config.max_issue_per_minute,
        }
    }
}

#[derive(Debug)]
pub struct WebSession {
    pub id: String,
    pub csrf: String,
    pub caller: conex_core::Caller,
    pub role: String,
    pub origin: String,
    expires_at_ms: std::sync::atomic::AtomicU64,
    /// Sliding idle expiry; `None` for authenticated sessions (fixed TTL).
    idle_ttl_ms: Option<u64>,
    pub link_id: String,
    revoked: std::sync::atomic::AtomicBool,
    pub revoked_signal: Notify,
}

impl WebSession {
    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn is_active(&self) -> bool {
        !self.revoked.load(std::sync::atomic::Ordering::Acquire) && self.expires_at_ms() > now_ms()
    }

    /// Guest sessions slide: every authenticated use extends the expiry.
    pub fn touch(&self) {
        if let Some(idle) = self.idle_ttl_ms {
            self.expires_at_ms
                .store(now_ms() + idle, std::sync::atomic::Ordering::Release);
        }
    }

    fn revoke(&self) {
        self.revoked
            .store(true, std::sync::atomic::Ordering::Release);
        self.revoked_signal.notify_waiters();
    }
}

#[derive(Debug, Clone, Copy)]
struct Tombstone {
    was_guest: bool,
    at_ms: u64,
}

pub struct WebAuth {
    origin: String,
    secure_cookie: bool,
    tickets: Arc<TicketRegistry>,
    ui_links: Arc<UiLinkRegistry>,
    sessions: Mutex<HashMap<String, Arc<WebSession>>>,
    guest: Option<GuestPolicy>,
    /// Rolling-minute guest issuance log for the rate limit.
    guest_issued: Mutex<VecDeque<u64>>,
    /// Removed session ids: guest cookies may reissue, authenticated ones
    /// must surface the auth error instead of silently downgrading.
    tombstones: Mutex<HashMap<String, Tombstone>>,
}

impl WebAuth {
    pub fn new(
        origin: impl Into<String>,
        secure_cookie: bool,
        tickets: Arc<TicketRegistry>,
        ui_links: Arc<UiLinkRegistry>,
    ) -> Self {
        Self::with_guest(origin, secure_cookie, tickets, ui_links, None)
    }

    pub fn with_guest(
        origin: impl Into<String>,
        secure_cookie: bool,
        tickets: Arc<TicketRegistry>,
        ui_links: Arc<UiLinkRegistry>,
        guest: Option<GuestPolicy>,
    ) -> Self {
        Self {
            origin: origin.into(),
            secure_cookie,
            tickets,
            ui_links,
            sessions: Mutex::new(HashMap::new()),
            guest,
            guest_issued: Mutex::new(VecDeque::new()),
            tombstones: Mutex::new(HashMap::new()),
        }
    }

    pub fn guest_enabled(&self) -> bool {
        self.guest.is_some()
    }

    pub fn guest_principal(&self) -> Option<&str> {
        self.guest.as_ref().map(|guest| guest.principal_id.as_str())
    }

    /// Issue a read-only anonymous session from the configured policy.
    /// Capacity and issuance rate are independent from the authenticated
    /// per-principal cap (plan M1.1).
    pub fn login_guest(&self) -> Result<Arc<WebSession>, CallError> {
        let guest = self
            .guest
            .as_ref()
            .ok_or_else(|| unauthorized("guest sessions are not enabled"))?;
        let now = now_ms();
        {
            let mut issued = self
                .guest_issued
                .lock()
                .expect("guest issuance log poisoned");
            issued.retain(|issued_at| now.saturating_sub(*issued_at) < 60_000);
            if issued.len() >= guest.max_issue_per_minute as usize {
                return Err(CallError::new(
                    conex_proto::ErrorCode::QuotaExceeded,
                    "guest session issuance rate exceeded",
                ));
            }
            issued.push_back(now);
        }
        let mut sessions = self.sessions.lock().expect("web session registry poisoned");
        let guest_count = sessions
            .values()
            .filter(|session| session.caller.principal_id == guest.principal_id)
            .count();
        if guest_count >= guest.max_sessions || sessions.len() >= MAX_SESSIONS_GLOBAL {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "anonymous session capacity exceeded",
            ));
        }
        let link = self
            .ui_links
            .register(&guest.principal_id, &guest.tenant_id);
        let session = Arc::new(WebSession {
            id: random_token(),
            csrf: random_token(),
            caller: conex_core::Caller {
                principal_id: guest.principal_id.clone(),
                tenant_id: guest.tenant_id.clone(),
                actor_peer_id: String::new(),
            },
            role: "ui".into(),
            origin: self.origin.clone(),
            expires_at_ms: std::sync::atomic::AtomicU64::new(now + guest.idle_ttl_ms),
            idle_ttl_ms: Some(guest.idle_ttl_ms),
            link_id: link.link_id.clone(),
            revoked: std::sync::atomic::AtomicBool::new(false),
            revoked_signal: Notify::new(),
        });
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    pub fn ui_links(&self) -> &Arc<UiLinkRegistry> {
        &self.ui_links
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn login(&self, inbound: InboundCaller) -> Result<Arc<WebSession>, CallError> {
        if inbound.role != "ui" {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "UI role required",
            ));
        }
        let mut sessions = self.sessions.lock().expect("web session registry poisoned");
        let now = now_ms();
        let mut expired = Vec::new();
        sessions.retain(|id, session| {
            let keep = session.is_active() && session.expires_at_ms() > now;
            if !keep {
                expired.push((id.clone(), session.caller.principal_id.clone()));
            }
            keep
        });
        for (id, principal) in expired {
            self.record_tombstone(&id, principal);
        }
        let principal_count = sessions
            .values()
            .filter(|session| session.caller.principal_id == inbound.caller.principal_id)
            .count();
        if principal_count >= MAX_SESSIONS_PER_PRINCIPAL || sessions.len() >= MAX_SESSIONS_GLOBAL {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "web session capacity exceeded",
            ));
        }
        let link = self
            .ui_links
            .register(&inbound.caller.principal_id, &inbound.caller.tenant_id);
        let session = Arc::new(WebSession {
            id: random_token(),
            csrf: random_token(),
            caller: inbound.caller,
            role: inbound.role,
            origin: self.origin.clone(),
            expires_at_ms: std::sync::atomic::AtomicU64::new(now + SESSION_TTL_MS),
            idle_ttl_ms: None,
            link_id: link.link_id.clone(),
            revoked: std::sync::atomic::AtomicBool::new(false),
            revoked_signal: Notify::new(),
        });
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    fn record_tombstone(&self, id: &str, principal_id: String) {
        let was_guest = self
            .guest
            .as_ref()
            .is_some_and(|guest| guest.principal_id == principal_id);
        let mut tombstones = self.tombstones.lock().expect("session tombstones poisoned");
        let now = now_ms();
        tombstones.retain(|_, tombstone| now.saturating_sub(tombstone.at_ms) < TOMBSTONE_TTL_MS);
        if tombstones.len() >= TOMBSTONE_CAPACITY {
            tombstones.clear();
        }
        tombstones.insert(
            id.to_string(),
            Tombstone {
                was_guest,
                at_ms: now,
            },
        );
    }

    /// A stale cookie may silently become a fresh anonymous session only when
    /// it belonged to a guest session and guest access is configured.
    pub fn can_reissue_guest(&self, id: &str) -> bool {
        self.guest.is_some()
            && self
                .tombstones
                .lock()
                .expect("session tombstones poisoned")
                .get(id)
                .is_some_and(|tombstone| tombstone.was_guest)
    }

    pub fn get(&self, id: &str) -> Option<Arc<WebSession>> {
        let mut sessions = self.sessions.lock().expect("web session registry poisoned");
        let session = sessions.get(id).cloned();
        if session.as_ref().is_some_and(|session| !session.is_active()) {
            sessions.remove(id);
            if let Some(session) = session.as_ref() {
                let principal = session.caller.principal_id.clone();
                drop(sessions);
                self.record_tombstone(id, principal);
            }
            return None;
        }
        session
    }

    /// Revocation signals existing UI WS loops, which close before accepting
    /// another business frame, and removes all unconsumed tickets.
    pub fn revoke(&self, id: &str) -> bool {
        let session = self
            .sessions
            .lock()
            .expect("web session registry poisoned")
            .remove(id);
        if let Some(session) = session {
            let principal = session.caller.principal_id.clone();
            session.revoke();
            self.tickets.revoke_session(id);
            self.ui_links.remove(&session.link_id);
            self.record_tombstone(id, principal);
            true
        } else {
            self.tickets.revoke_session(id);
            false
        }
    }

    pub fn session_from_headers(&self, headers: &HeaderMap) -> Result<Arc<WebSession>, CallError> {
        let id = cookie_value(headers, SESSION_COOKIE)
            .ok_or_else(|| unauthorized("web session cookie required"))?;
        let session = self
            .get(id)
            .ok_or_else(|| unauthorized("web session is invalid or expired"))?;
        session.touch();
        Ok(session)
    }

    pub fn check_origin(&self, headers: &HeaderMap) -> Result<(), CallError> {
        let origin = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        if origin != Some(self.origin.as_str()) {
            return Err(unauthorized("Origin does not match configured web origin"));
        }
        Ok(())
    }

    pub fn check_mutation(&self, headers: &HeaderMap) -> Result<Arc<WebSession>, CallError> {
        self.check_origin(headers)?;
        let session = self.session_from_headers(headers)?;
        let csrf = headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok());
        if csrf != Some(session.csrf.as_str()) {
            return Err(unauthorized("CSRF token is invalid"));
        }
        Ok(session)
    }

    pub fn set_cookie(&self, session: &WebSession) -> HeaderValue {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict{}",
            SESSION_COOKIE, session.id, secure
        ))
        .expect("valid cookie")
    }

    pub fn clear_cookie(&self) -> HeaderValue {
        HeaderValue::from_static("conex_web_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict")
    }
}

pub async fn login(Extension(state): Extension<Arc<HttpState>>, headers: HeaderMap) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return error_response(unavailable("web auth is not configured"));
    };
    if let Err(error) = web.check_origin(&headers) {
        return error_response(error);
    }
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let inbound = match state.auth.authenticate(authorization) {
        Ok(inbound) => inbound,
        Err(error) => return error_response(error),
    };
    let session = match web.login(inbound) {
        Ok(session) => session,
        Err(error) => return error_response(error),
    };
    let mut response = (
        StatusCode::OK,
        axum::Json(json!({
            "principalId": session.caller.principal_id,
            "tenantId": session.caller.tenant_id,
            "role": session.role,
            "expiresAtMs": session.expires_at_ms().to_string(),
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, web.set_cookie(&session));
    response
}

pub async fn session(Extension(state): Extension<Arc<HttpState>>, headers: HeaderMap) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return error_response(unavailable("web auth is not configured"));
    };
    let cookie_present = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|raw| {
            raw.split(';').any(|part| {
                part.trim_start()
                    .starts_with(&format!("{}=", SESSION_COOKIE))
            })
        });
    if !cookie_present {
        if !web.guest_enabled() {
            return error_response(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "web session cookie required",
            ));
        }
        let session = match web.login_guest() {
            Ok(session) => session,
            Err(error) => return error_response(error),
        };
        let principal_id = session.caller.principal_id.clone();
        let tenant_id = session.caller.tenant_id.clone();
        let csrf = session.csrf.clone();
        let expires_at_ms = session.expires_at_ms().to_string();
        let mut response = (
            StatusCode::OK,
            axum::Json(json!({
                "principalId": principal_id,
                "tenantId": tenant_id,
                "role": session.role,
                "csrf": csrf,
                "expiresAtMs": expires_at_ms,
            })),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::SET_COOKIE, web.set_cookie(&session));
        return response;
    }
    let session = match web.session_from_headers(&headers) {
        Ok(session) => session,
        Err(error) => {
            // A stale guest cookie silently becomes a fresh anonymous
            // session; an authenticated user's dead session surfaces the
            // auth error instead of silently downgrading (plan M1.1).
            let reissue =
                cookie_value(&headers, SESSION_COOKIE).is_some_and(|id| web.can_reissue_guest(id));
            if !reissue {
                return error_response(error);
            }
            match web.login_guest() {
                Ok(session) => session,
                Err(error) => return error_response(error),
            }
        }
    };
    let principal_id = session.caller.principal_id.clone();
    let tenant_id = session.caller.tenant_id.clone();
    let csrf = session.csrf.clone();
    let expires_at_ms = session.expires_at_ms().to_string();
    let mut response = (
        StatusCode::OK,
        axum::Json(json!({
            "principalId": principal_id,
            "tenantId": tenant_id,
            "role": session.role,
            "csrf": csrf,
            "expiresAtMs": expires_at_ms,
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, web.set_cookie(&session));
    response
}

pub async fn logout(Extension(state): Extension<Arc<HttpState>>, headers: HeaderMap) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return error_response(unavailable("web auth is not configured"));
    };
    let session = match web.check_mutation(&headers) {
        Ok(session) => session,
        Err(error) => return error_response(error),
    };
    web.revoke(&session.id);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, web.clear_cookie());
    response
}

pub fn allowed_ui_method(method: &str) -> bool {
    matches!(
        method,
        "endpoint/list" | "connection/list" | "source/list" | "source/read" | "source/search"
    )
}

pub fn error_response(error: CallError) -> Response {
    let status = match error.code() {
        code if code == conex_proto::ErrorCode::Unauthorized as i32 => StatusCode::UNAUTHORIZED,
        code if code == conex_proto::ErrorCode::Forbidden as i32 => StatusCode::FORBIDDEN,
        code if code == conex_proto::ErrorCode::QuotaExceeded as i32 => {
            StatusCode::TOO_MANY_REQUESTS
        }
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        axum::Json(json!({ "code": error.code(), "message": error.message() })),
    )
        .into_response()
}

fn unauthorized(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::Unauthorized, message)
}
fn unavailable(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::Unavailable, message)
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let value = headers.get(header::COOKIE)?.to_str().ok()?;
    value.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

fn random_token() -> String {
    use base64::Engine as _;
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn session_deadline() -> Duration {
    Duration::from_millis(SESSION_TTL_MS)
}
