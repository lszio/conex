//! HTTP layer for P1 ticket issuance + OIDC code/PKCE.
//!
//! All three routes pull state from the shared `Arc<HttpState>`. The
//! handlers do no real cryptographic verification — that lives behind a
//! future feature flag. Today they:
//!
//! - Issue a 30 s ticket bound to `{principal, tenant, origin, host}`.
//! - Issue/consume OIDC codes with `S256` PKCE verification.
//! - Forward ticket consumption as a `Broker` call that returns a fresh
//!   session id when the request body asks for one (it doesn't today —
//!   the broker layer rejects unknown sub-commands cleanly).

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use conex_core::CallError;
use conex_proto::v1;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::agent::{HostSide, OidcRegistry, TicketRegistry};
use crate::broker::Broker;
use crate::http::HttpState;

pub struct TicketsState {
    pub tickets: Arc<TicketRegistry>,
    pub broker: Arc<Broker>,
}

pub struct OidcState {
    pub oidc: Arc<OidcRegistry>,
}

fn call_error_to_response(error: CallError) -> Response {
    let code = error.code();
    let status = if code == v1::ErrorCode::Unauthorized as i32 {
        StatusCode::UNAUTHORIZED
    } else if code == v1::ErrorCode::BadRequest as i32 {
        StatusCode::BAD_REQUEST
    } else if code == v1::ErrorCode::Forbidden as i32 {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (
        status,
        Json(json!({
            "code": code,
            "message": error.message(),
        })),
    )
        .into_response()
}

fn call_error_data_to_response(error: CallError) -> Response {
    call_error_to_response(error)
}

fn principal_from_auth(
    state: &HttpState,
    headers: &HeaderMap,
) -> Result<conex_core::Caller, Box<CallError>> {
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    state
        .auth
        .authenticate(authorization)
        .map(|inbound| inbound.caller)
        .map_err(Box::new)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketRequest {
    pub principal_id: String,
    pub tenant_id: String,
    pub origin: String,
    pub target_host: String,
    pub peer_role: String,
    #[serde(default)]
    pub capability_caps: Vec<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub async fn issue_ticket(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
    Json(request): Json<TicketRequest>,
) -> Response {
    let side = match state.host_side.as_ref() {
        Some(side) => side,
        None => {
            return call_error_data_to_response(CallError::new(
                v1::ErrorCode::Unavailable,
                "ticket backend is not configured",
            ));
        }
    };
    let caller = match principal_from_auth(&state, &headers) {
        Ok(caller) => caller,
        Err(error) => return call_error_to_response(*error),
    };
    if caller.principal_id != request.principal_id || caller.tenant_id != request.tenant_id {
        return call_error_data_to_response(CallError::new(
            v1::ErrorCode::Forbidden,
            "ticket principal/tenant does not match the bearer",
        ));
    }
    match side.tickets.issue(
        &request.principal_id,
        &request.tenant_id,
        &request.origin,
        &request.target_host,
        &request.peer_role,
        request.capability_caps,
        request.session_id,
    ) {
        Ok(ticket) => (
            StatusCode::CREATED,
            Json(json!({
                "ticket": ticket.ticket,
                "principalId": ticket.principal_id,
                "tenantId": ticket.tenant_id,
                "origin": ticket.origin,
                "targetHost": ticket.target_host,
                "peerRole": ticket.peer_role,
                "capabilityCaps": ticket.capability_caps,
                "sessionId": ticket.session_id,
                "issuedAtMs": ticket.issued_at_ms.to_string(),
                "expiresAtMs": ticket.expires_at_ms.to_string(),
            })),
        )
            .into_response(),
        Err(error) => call_error_data_to_response(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcAuthorizeRequest {
    pub principal_id: String,
    pub tenant_id: String,
    pub origin: String,
    pub audience: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

pub async fn oidc_authorize(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
    Json(request): Json<OidcAuthorizeRequest>,
) -> Response {
    let side = match state.host_side.as_ref() {
        Some(side) => side,
        None => {
            return call_error_data_to_response(CallError::new(
                v1::ErrorCode::Unavailable,
                "oidc backend is not configured",
            ));
        }
    };
    let caller = match principal_from_auth(&state, &headers) {
        Ok(caller) => caller,
        Err(error) => return call_error_to_response(*error),
    };
    if caller.principal_id != request.principal_id || caller.tenant_id != request.tenant_id {
        return call_error_data_to_response(CallError::new(
            v1::ErrorCode::Forbidden,
            "oidc principal/tenant does not match the bearer",
        ));
    }
    match side.oidc.issue(
        &request.principal_id,
        &request.tenant_id,
        &request.audience,
        &request.origin,
        &request.code_challenge,
        &request.code_challenge_method,
    ) {
        Ok(code) => (
            StatusCode::CREATED,
            Json(json!({
                "code": code.code,
                "principalId": code.principal_id,
                "tenantId": code.tenant_id,
                "audience": code.audience,
                "origin": code.origin,
                "expiresAtMs": code.expires_at_ms.to_string(),
            })),
        )
            .into_response(),
        Err(error) => call_error_data_to_response(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcTokenRequest {
    pub code: String,
    pub code_verifier: String,
    pub origin: String,
}

pub async fn oidc_token(
    Extension(state): Extension<Arc<HttpState>>,
    Json(request): Json<OidcTokenRequest>,
) -> Response {
    let side = match state.host_side.as_ref() {
        Some(side) => side,
        None => {
            return call_error_data_to_response(CallError::new(
                v1::ErrorCode::Unavailable,
                "oidc backend is not configured",
            ));
        }
    };
    match side
        .oidc
        .exchange(&request.code, &request.code_verifier, &request.origin)
    {
        Ok(code) => {
            let mut ticket_seed = Vec::new();
            ticket_seed.extend_from_slice(code.principal_id.as_bytes());
            ticket_seed.push(b':');
            ticket_seed.extend_from_slice(code.tenant_id.as_bytes());
            ticket_seed.push(b':');
            ticket_seed.extend_from_slice(code.origin.as_bytes());
            ticket_seed.push(b':');
            ticket_seed.extend_from_slice(b"web");
            ticket_seed.push(b':');
            ticket_seed.extend_from_slice(code.code.as_bytes());
            let ticket_value = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(&ticket_seed));
            (
                StatusCode::OK,
                Json(json!({
                    "principalId": code.principal_id,
                    "tenantId": code.tenant_id,
                    "audience": code.audience,
                    "origin": code.origin,
                    "ticket": ticket_value,
                })),
            )
                .into_response()
        }
        Err(error) => call_error_data_to_response(error),
    }
}

/// Marker trait so route handlers can share a typed `State` extractor.
pub trait TicketsContext {
    fn tickets(&self) -> Option<Arc<TicketRegistry>>;
    fn broker(&self) -> Option<Arc<Broker>>;
    fn oidc(&self) -> Option<Arc<OidcRegistry>>;
}

impl TicketsContext for HttpState {
    fn tickets(&self) -> Option<Arc<TicketRegistry>> {
        self.host_side.as_ref().map(|side| side.tickets.clone())
    }
    fn broker(&self) -> Option<Arc<Broker>> {
        self.broker.clone()
    }
    fn oidc(&self) -> Option<Arc<OidcRegistry>> {
        self.host_side.as_ref().map(|side| side.oidc.clone())
    }
}

/// Ensure `HostSide` is reachable from this module.
pub fn _host_side_marker(_: &HostSide) -> &HostSide {
    unreachable!()
}

/// Decode a base64-or-utf8 helper used by ticket/OIDC tests.
pub fn decode_payload(value: &Value) -> Option<Vec<u8>> {
    value
        .as_str()
        .and_then(|raw| base64::engine::general_purpose::STANDARD.decode(raw).ok())
}
