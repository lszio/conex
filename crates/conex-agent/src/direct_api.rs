//! Direct-connection entry (docs/contracts/private-runtime.md §4, P5.3):
//! a verified *direct* grant with an exactly approved Origin may fetch one
//! 30-second opaque ticket, then upgrade WSS with the
//! `conex-ticket.<opaque>` subprotocol next to the real profile. The ticket
//! value never echoes back and never enters a log line. Native clients skip
//! tickets and present `Authorization: Bearer <grant>` on the upgrade. Every
//! business frame rechecks expiry, epoch, generation, purpose and local
//! revocation — the upgrade itself grants nothing permanent.
use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use futures::SinkExt;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};

use conex_access::{GrantVerifier, VerifiedGrant};
use conex_agent_domain::Store;
use conex_proto::message;

use crate::node::NodeHandle;

pub const TICKET_TTL_MS: u64 = 30_000;
/// Bounded consumption table: when full, new tickets are refused — valid
/// tickets are never evicted to fake replay protection (contract §3.3).
const TICKET_TABLE_CAPACITY: usize = 4096;
const PROFILE_JSON: &str = "conex-jsonrpc2-wss";

#[derive(Clone)]
pub struct DirectSettings {
    pub node_id: String,
    pub node_epoch: String,
    pub tenant: String,
    pub verifier: GrantVerifier,
    pub approved_web_origins: Vec<String>,
    pub listen: String,
}

struct TicketEntry {
    grant: VerifiedGrant,
    origin: String,
    expires_at_ms: u64,
}

pub struct DirectApi {
    settings: DirectSettings,
    store: Store,
    node: NodeHandle,
    tickets: Arc<std::sync::Mutex<HashMap<String, TicketEntry>>>,
}

impl DirectApi {
    pub fn spawn(
        settings: DirectSettings,
        store: Store,
        node: NodeHandle,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        let api = Arc::new(Self {
            settings,
            store,
            node,
            tickets: Arc::new(std::sync::Mutex::new(HashMap::new())),
        });
        let handle = api.clone();
        tokio::spawn(async move {
            let app = Router::new()
                .route("/private/tickets", axum::routing::post(ticket_route))
                .route("/private/tickets", axum::routing::options(ticket_preflight))
                .route("/wss", axum::routing::get(upgrade_route))
                .layer(axum::Extension(handle));
            let listener = tokio::net::TcpListener::bind(&api.settings.listen)
                .await
                .map_err(|error| format!("direct entry bind: {error}"))?;
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let mut shutdown = shutdown;
                    while !*shutdown.borrow_and_update() {
                        if shutdown.changed().await.is_err() {
                            break;
                        }
                    }
                })
                .await
                .map_err(|error| format!("direct entry serve: {error}"))
        })
    }

    fn verifier(&self) -> &GrantVerifier {
        &self.settings.verifier
    }

    // axum Responses are intentionally returned whole for HTTP-mapped denials;
    // boxing them would obscure the one-error-one-response boundary.
    #[allow(clippy::result_large_err)]
    async fn check_revocation(&self, grant: &VerifiedGrant) -> Result<(), Response> {
        match self
            .store
            .access_revocation_blocked(&grant.jti, &grant.principal, &grant.kid)
            .await
        {
            Ok(false) => Ok(()),
            Ok(true) => Err(forbidden("grant is revoked")),
            Err(error) => Err(internal(&error.to_string())),
        }
    }

    #[allow(clippy::result_large_err)]
    fn verify_bearer(&self, headers: &HeaderMap, now_ms: u64) -> Result<VerifiedGrant, Response> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| unauthorized("Authorization must carry a direct grant"))?;
        let grant = self
            .verifier()
            .verify(token, now_ms)
            .map_err(|error| unauthorized(&error.to_string()))?;
        if grant.scope.purpose != "direct" {
            return Err(forbidden("the direct entry accepts direct grants only"));
        }
        Ok(grant)
    }
}

use conex_proto::now_ms;

fn new_ticket() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[allow(clippy::result_large_err)]
fn exact_origin<'a>(headers: &'a HeaderMap, allowed: &'a [String]) -> Result<&'a str, Response> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| forbidden("Origin is required"))?;
    if origin == "null" || !allowed.iter().any(|allowed| allowed == origin) {
        return Err(forbidden("Origin is not approved for direct access"));
    }
    Ok(origin)
}

async fn ticket_preflight(api: axum::Extension<Arc<DirectApi>>, headers: HeaderMap) -> Response {
    let allowed = &api.settings.approved_web_origins;
    let origin = match exact_origin(&headers, allowed) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    (StatusCode::NO_CONTENT, allow_origin_headers(origin)).into_response()
}

/// CORS 回显：预检与实际响应都必须带 Allow-Origin，否则浏览器在真实 POST
/// 上照样拦截。逐字回显批准的 Origin（不带凭据的通配不是这里需要的形状）。
fn allow_origin_headers(origin: &str) -> [(HeaderName, String); 5] {
    [
        (header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.to_string()),
        (
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "POST, OPTIONS".to_string(),
        ),
        (
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Authorization, Content-Type".to_string(),
        ),
        (
            header::HeaderName::from_static("access-control-max-age"),
            "600".to_string(),
        ),
        (
            header::HeaderName::from_static("vary"),
            "Origin".to_string(),
        ),
    ]
}

/// `POST /private/tickets` with `Authorization: Bearer <direct grant>` and an
/// exactly approved Origin. The response carries the opaque ticket and the
/// real profile — never an echo of the secret.
async fn ticket_route(api: axum::Extension<Arc<DirectApi>>, headers: HeaderMap) -> Response {
    let now = now_ms();
    let origin = match exact_origin(&headers, &api.settings.approved_web_origins) {
        Ok(origin) => origin.to_string(),
        Err(response) => return response,
    };
    let grant = match api.verify_bearer(&headers, now) {
        Ok(grant) => grant,
        Err(response) => return response,
    };
    if let Err(response) = api.check_revocation(&grant).await {
        return response;
    }
    let ticket = new_ticket();
    {
        let mut tickets = api.tickets.lock().expect("ticket table poisoned");
        tickets.retain(|_, entry| entry.expires_at_ms > now);
        if tickets.len() >= TICKET_TABLE_CAPACITY {
            return quota_response();
        }
        tickets.insert(
            ticket.clone(),
            TicketEntry {
                grant,
                origin: origin.clone(),
                expires_at_ms: now + TICKET_TTL_MS,
            },
        );
    }
    (
        StatusCode::OK,
        allow_origin_headers(&origin),
        axum::Json(json!({
            "ticket": ticket,
            "expiresAtMs": (now + TICKET_TTL_MS).to_string(),
            "profile": PROFILE_JSON,
        })),
    )
        .into_response()
}

/// WSS upgrade: the browser path consumes the ticket subprotocol atomically;
/// the native path verifies the bearer grant directly. Either way the server
/// selects the actual profile and never echoes the ticket value.
async fn upgrade_route(
    ws: WebSocketUpgrade,
    api: axum::Extension<Arc<DirectApi>>,
    headers: HeaderMap,
) -> Response {
    let now = now_ms();
    let offered = headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let ticket_values: Vec<&str> = offered
        .iter()
        .filter_map(|part| part.strip_prefix("conex-ticket."))
        .collect();
    let grant = if !ticket_values.is_empty() {
        if ticket_values.len() > 1 {
            return unauthorized("only one ticket subprotocol is accepted");
        }
        let ticket = ticket_values[0];
        if headers.get(header::AUTHORIZATION).is_some() {
            return unauthorized("ticket and bearer authentication are mutually exclusive");
        }
        let origin = match exact_origin(&headers, &api.settings.approved_web_origins) {
            Ok(origin) => origin.to_string(),
            Err(response) => return response,
        };
        // Atomic single-use consumption: remove first, then validate.
        let entry = api
            .tickets
            .lock()
            .expect("ticket table poisoned")
            .remove(ticket);
        let Some(entry) = entry else {
            return unauthorized("ticket is invalid or already used");
        };
        if entry.expires_at_ms <= now || entry.origin != origin {
            return unauthorized("ticket is expired or origin mismatch");
        }
        entry.grant
    } else {
        match api.verify_bearer(&headers, now) {
            Ok(grant) => grant,
            Err(response) => return response,
        }
    };
    if let Err(response) = api.check_revocation(&grant).await {
        return response;
    }
    if offered.iter().all(|part| part != PROFILE_JSON) {
        return bad_request("the actual Conex profile subprotocol is required");
    }
    let max_frame = 1usize << 20;
    ws.protocols([PROFILE_JSON])
        .max_frame_size(max_frame)
        .max_message_size(max_frame)
        .on_upgrade(move |socket| handle_connection(socket, api.0, grant))
        .into_response()
}

async fn handle_connection(mut socket: WebSocket, api: Arc<DirectApi>, grant: VerifiedGrant) {
    // One business frame per connection step; every frame rechecks the grant.
    while let Some(Ok(frame)) = socket.recv().await {
        let Message::Text(text) = frame else {
            // Binary business frames are a protobuf-profile concern; the
            // direct entry serves the JSON profile only.
            let _ = socket.send(Message::Text(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":12,"message":"the direct entry serves the JSON profile only"}}).to_string().into(),
            )).await;
            continue;
        };
        let reply = dispatch_frame(&api, &grant, text.as_bytes()).await;
        let Ok(reply) = serde_json::to_string(&reply) else {
            break;
        };
        if socket.send(Message::Text(reply.into())).await.is_err() {
            break;
        }
    }
    let _ = socket.close().await;
}

async fn dispatch_frame(api: &DirectApi, grant: &VerifiedGrant, bytes: &[u8]) -> Value {
    let decoded = conex_proto::wire::decode_wire_for_boundary(
        bytes,
        conex_proto::capability::Boundary::Client,
    );
    let (id, result) = match decoded {
        Ok(conex_proto::Message {
            body: Some(message::Body::Request(request)),
        }) => {
            let id = request.request_id.clone();
            let result = dispatch_request(api, grant, request).await;
            (Some(id), result)
        }
        Ok(_) => (None, Err(crate::node::failure("bad_request"))),
        Err(_) => (None, Err(crate::node::failure("bad_request"))),
    };
    match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => json!({"jsonrpc":"2.0","id":id,"error":{
            "code":error.0.code,"message":error.0.message,
            "execution":error.0.execution,"retry":error.0.retry,
        }}),
    }
}

async fn dispatch_request(
    api: &DirectApi,
    grant: &VerifiedGrant,
    request: conex_proto::Request,
) -> crate::node::NodeResult {
    let params = request
        .params
        .ok_or_else(|| crate::node::failure("bad_request"))?;
    let context = params
        .context
        .ok_or_else(|| crate::node::failure("bad_request"))?;
    // A direct client supplies none of the hub's routing identity; the node
    // is the authority here and the grant is the caller.
    if context.plane != conex_proto::Plane::Broker as i32
        || !context.principal_id.is_empty()
        || !context.tenant_id.is_empty()
    {
        return Err(crate::node::failure("forbidden"));
    }
    if request.method != "node/invoke" {
        return Err(crate::node::failure("unsupported_capability"));
    }
    let input = params
        .input
        .map(|value| conex_proto::wire::value_to_json(&value))
        .transpose()
        .map_err(|_| crate::node::failure("bad_request"))?
        .unwrap_or_else(|| json!({}));
    let invoke: InvokeInput =
        serde_json::from_value(input).map_err(|_| crate::node::failure("bad_request"))?;
    let generation = parse_generation(&invoke.generation)?;
    // Local revocation and hub-synced revocation both gate the call.
    match api
        .store
        .access_revocation_blocked(&grant.jti, &grant.principal, &grant.kid)
        .await
    {
        Ok(false) => {}
        Ok(true) => return Err(crate::node::failure("forbidden")),
        Err(error) => return Err(crate::node::failure(&error.to_string())),
    }
    api.node
        .invoke_granted(
            grant.clone(),
            invoke.instance_id,
            invoke.module,
            generation,
            invoke.method,
            invoke.argument,
        )
        .await
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InvokeInput {
    instance_id: String,
    module: String,
    generation: String,
    method: String,
    argument: Value,
}

fn parse_generation(text: &str) -> Result<u64, conex_sdk::Error> {
    let value = text
        .parse::<u64>()
        .ok()
        .filter(|value| *value != 0 && value.to_string() == text);
    value.ok_or_else(|| crate::node::failure("bad_request"))
}

fn unauthorized(message: &str) -> Response {
    error_response(StatusCode::UNAUTHORIZED, message)
}
fn forbidden(message: &str) -> Response {
    error_response(StatusCode::FORBIDDEN, message)
}
fn bad_request(message: &str) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}
fn internal(message: &str) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, message)
}
fn error_response(status: StatusCode, message: &str) -> Response {
    (status, message.to_owned()).into_response()
}

fn quota_response() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(json!({"code": 9, "message": "ticket table is full"})),
    )
        .into_response()
}
