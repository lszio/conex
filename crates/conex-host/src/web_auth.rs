use std::collections::HashMap;
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

#[derive(Debug)]
pub struct WebSession {
    pub id: String,
    pub csrf: String,
    pub caller: conex_core::Caller,
    pub role: String,
    pub origin: String,
    pub expires_at_ms: u64,
    pub link_id: String,
    revoked: std::sync::atomic::AtomicBool,
    pub revoked_signal: Notify,
}

impl WebSession {
    pub fn is_active(&self) -> bool {
        !self.revoked.load(std::sync::atomic::Ordering::Acquire) && self.expires_at_ms > now_ms()
    }

    fn revoke(&self) {
        self.revoked.store(true, std::sync::atomic::Ordering::Release);
        self.revoked_signal.notify_waiters();
    }
}

pub struct WebAuth {
    origin: String,
    secure_cookie: bool,
    tickets: Arc<TicketRegistry>,
    ui_links: Arc<UiLinkRegistry>,
    sessions: Mutex<HashMap<String, Arc<WebSession>>>,
}

impl WebAuth {
    pub fn new(
        origin: impl Into<String>,
        secure_cookie: bool,
        tickets: Arc<TicketRegistry>,
        ui_links: Arc<UiLinkRegistry>,
    ) -> Self {
        Self {
            origin: origin.into(),
            secure_cookie,
            tickets,
            ui_links,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn ui_links(&self) -> &Arc<UiLinkRegistry> {
        &self.ui_links
    }

    pub fn origin(&self) -> &str { &self.origin }

    pub fn login(&self, inbound: InboundCaller) -> Result<Arc<WebSession>, CallError> {
        if inbound.role != "ui" {
            return Err(CallError::new(conex_proto::ErrorCode::Unauthorized, "UI role required"));
        }
        let mut sessions = self.sessions.lock().expect("web session registry poisoned");
        let now = now_ms();
        sessions.retain(|_, session| session.is_active() && session.expires_at_ms > now);
        let principal_count = sessions.values().filter(|session| session.caller.principal_id == inbound.caller.principal_id).count();
        if principal_count >= 8 || sessions.len() >= 1024 {
            return Err(CallError::new(conex_proto::ErrorCode::QuotaExceeded, "web session capacity exceeded"));
        }
        let link = self.ui_links.register(&inbound.caller.principal_id, &inbound.caller.tenant_id);
        let session = Arc::new(WebSession {
            id: random_token(),
            csrf: random_token(),
            caller: inbound.caller,
            role: inbound.role,
            origin: self.origin.clone(),
            expires_at_ms: now + SESSION_TTL_MS,
            link_id: link.link_id.clone(),
            revoked: std::sync::atomic::AtomicBool::new(false),
            revoked_signal: Notify::new(),
        });
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    pub fn get(&self, id: &str) -> Option<Arc<WebSession>> {
        let mut sessions = self.sessions.lock().expect("web session registry poisoned");
        let session = sessions.get(id).cloned();
        if session.as_ref().is_some_and(|session| !session.is_active()) {
            sessions.remove(id);
            return None;
        }
        session
    }

    /// Revocation signals existing UI WS loops, which close before accepting
    /// another business frame, and removes all unconsumed tickets.
    pub fn revoke(&self, id: &str) -> bool {
        let session = self.sessions.lock().expect("web session registry poisoned").remove(id);
        if let Some(session) = session {
            session.revoke();
            self.tickets.revoke_session(id);
            self.ui_links.remove(&session.link_id);
            true
        } else {
            self.tickets.revoke_session(id);
            false
        }
    }

    pub fn session_from_headers(&self, headers: &HeaderMap) -> Result<Arc<WebSession>, CallError> {
        let id = cookie_value(headers, SESSION_COOKIE).ok_or_else(|| unauthorized("web session cookie required"))?;
        self.get(id).ok_or_else(|| unauthorized("web session is invalid or expired"))
    }

    pub fn check_origin(&self, headers: &HeaderMap) -> Result<(), CallError> {
        let origin = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok());
        if origin != Some(self.origin.as_str()) {
            return Err(unauthorized("Origin does not match configured web origin"));
        }
        Ok(())
    }

    pub fn check_mutation(&self, headers: &HeaderMap) -> Result<Arc<WebSession>, CallError> {
        self.check_origin(headers)?;
        let session = self.session_from_headers(headers)?;
        let csrf = headers.get(CSRF_HEADER).and_then(|value| value.to_str().ok());
        if csrf != Some(session.csrf.as_str()) {
            return Err(unauthorized("CSRF token is invalid"));
        }
        Ok(session)
    }

    pub fn set_cookie(&self, session: &WebSession) -> HeaderValue {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        HeaderValue::from_str(&format!("{}={}; Path=/; HttpOnly; SameSite=Strict{}", SESSION_COOKIE, session.id, secure)).expect("valid cookie")
    }

    pub fn clear_cookie(&self) -> HeaderValue {
        HeaderValue::from_static("conex_web_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict")
    }
}

pub async fn login(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
) -> Response {
    let Some(web) = state.web_auth.as_ref() else { return error_response(unavailable("web auth is not configured")); };
    if let Err(error) = web.check_origin(&headers) { return error_response(error); }
    let authorization = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    let inbound = match state.auth.authenticate(authorization) {
        Ok(inbound) => inbound,
        Err(error) => return error_response(error),
    };
    let session = match web.login(inbound) {
        Ok(session) => session,
        Err(error) => return error_response(error),
    };
    let mut response = (StatusCode::OK, axum::Json(json!({
        "principalId": session.caller.principal_id,
        "tenantId": session.caller.tenant_id,
        "role": session.role,
        "expiresAtMs": session.expires_at_ms.to_string(),
    }))).into_response();
    response.headers_mut().insert(header::SET_COOKIE, web.set_cookie(&session));
    response
}

pub async fn session(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
) -> Response {
    let Some(web) = state.web_auth.as_ref() else { return error_response(unavailable("web auth is not configured")); };
    let session = match web.session_from_headers(&headers) { Ok(session) => session, Err(error) => return error_response(error) };
    (StatusCode::OK, axum::Json(json!({
        "principalId": session.caller.principal_id,
        "tenantId": session.caller.tenant_id,
        "role": session.role,
        "csrf": session.csrf,
        "expiresAtMs": session.expires_at_ms.to_string(),
    }))).into_response()
}

pub async fn logout(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
) -> Response {
    let Some(web) = state.web_auth.as_ref() else { return error_response(unavailable("web auth is not configured")); };
    let session = match web.check_mutation(&headers) { Ok(session) => session, Err(error) => return error_response(error) };
    web.revoke(&session.id);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::SET_COOKIE, web.clear_cookie());
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
        code if code == conex_proto::ErrorCode::QuotaExceeded as i32 => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, axum::Json(json!({ "code": error.code(), "message": error.message() }))).into_response()
}

fn unauthorized(message: impl Into<String>) -> CallError { CallError::new(conex_proto::ErrorCode::Unauthorized, message) }
fn unavailable(message: impl Into<String>) -> CallError { CallError::new(conex_proto::ErrorCode::Unavailable, message) }

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
    SystemRandom::new().fill(&mut bytes).expect("system random source unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn session_deadline() -> Duration { Duration::from_millis(SESSION_TTL_MS) }
