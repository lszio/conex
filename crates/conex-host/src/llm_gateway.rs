//! OpenAI 兼容网关（MVP-P10.3：docs/design/2026-11-12-conex-llm-provider-relay.md
//! §5）。hub 内的受约束翻译边缘：把已获准且 relay-allowed 的 `conex.llm`
//! 能力经既有转发路径（[`CallRouter`]）暴露为 `/v1/models` 与
//! `/v1/chat/completions`。不是通用隧道：目标注册、消费主体与 key 全部来自
//! 批准配置；prompt/completion 只在转发期间内存处理，不落盘、不进日志。
//!
//! 流式语义（首版）：`stream:true` 经 SSE 返回整段单帧 + 终帧 + [DONE]——
//! 上游结果已在转发路径上完整到达，增量分帧属后续契约扩展，不伪装打字机。
//! SSE 建立前的上游等待挂在响应体内：客户端断开触发尽力取消
//! （module/cancel 传播到节点），原调用最多一个终态；取消只是尽力而为，
//! 「已请求取消」不等于上游已停止生成。
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::Digest as _;

use conex_core::{CallError, Caller};
use conex_proto::ErrorCode;

use crate::capability::CallRouter;
use crate::capability::ModuleRegistry;
use crate::capability::calls::SourceKey;
use crate::capability::registry::new_registration_id;
use crate::config::LlmGatewayConfig;
use crate::http::HttpState;

const GATEWAY_BINDING: &str = "llm-gateway";
/// Err 变体装箱：响应体较大，避免每次认证调用的栈搬运。
type AuthResult = Result<(GatewayKey, InFlight), Box<Response>>;
/// 路由层硬上限；部署限额取配置 max_body_bytes（<= 此值），处理器内检查。
pub const MAX_BODY_LIMIT: usize = 16 * 1024 * 1024;
/// 取消帧自身的预算；取消只是尽力而为，不影响原调用的终态。
const CANCEL_BUDGET_MS: u64 = 5_000;

pub struct LlmGateway {
    registry: Arc<ModuleRegistry>,
    calls: Arc<CallRouter>,
    consumer: Caller,
    space_id: String,
    provider_endpoint_id: String,
    timeout: Duration,
    max_concurrent_per_key: usize,
    max_body_bytes: usize,
    keys: BTreeMap<String, GatewayKey>,
    inflight: Arc<std::sync::Mutex<HashMap<String, usize>>>,
}

#[derive(Clone)]
struct GatewayKey {
    name: String,
    models: Vec<String>,
}

/// 挂在 SSE 流状态上的取消哨兵：流被丢弃（客户端断开）时向固定注册代次
/// 传播 module/cancel；调用已终结时 cancel 返回 requested:false，无副作用。
struct CancelOnDrop {
    calls: Arc<CallRouter>,
    source: SourceKey,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let calls = self.calls.clone();
        let source = self.source.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(CANCEL_BUDGET_MS);
            let _ = calls.cancel(source, deadline).await;
        });
    }
}

/// 每 key 并发闸：Drop 释放计数；计数表由网关持有（Arc 共享同一张表）。
struct InFlight {
    counter: Arc<std::sync::Mutex<HashMap<String, usize>>>,
    key_hash: String,
}
impl InFlight {
    fn admit(
        counter: &Arc<std::sync::Mutex<HashMap<String, usize>>>,
        key_hash: &str,
        limit: usize,
    ) -> Option<InFlight> {
        let mut counts = counter.lock().ok()?;
        let current = counts.entry(key_hash.to_string()).or_insert(0);
        if *current >= limit {
            return None;
        }
        *current += 1;
        Some(InFlight {
            counter: counter.clone(),
            key_hash: key_hash.to_string(),
        })
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.counter.lock()
            && let Some(current) = counts.get_mut(&self.key_hash)
        {
            *current = current.saturating_sub(1);
        }
    }
}

impl LlmGateway {
    pub fn new(
        config: &LlmGatewayConfig,
        registry: Arc<ModuleRegistry>,
        calls: Arc<CallRouter>,
    ) -> Self {
        let consumer = Caller {
            principal_id: config.consumer_principal.clone().unwrap_or_default(),
            tenant_id: config.consumer_tenant.clone().unwrap_or_default(),
            actor_peer_id: String::new(),
        };
        let keys = config
            .keys
            .iter()
            .map(|key| {
                (
                    key.token_hash.trim().to_ascii_lowercase(),
                    GatewayKey {
                        name: key.name.clone(),
                        models: key.models.clone(),
                    },
                )
            })
            .collect();
        Self {
            registry,
            calls,
            consumer,
            space_id: config.space_id.clone(),
            provider_endpoint_id: config.provider_endpoint_id.clone(),
            timeout: Duration::from_millis(config.timeout_ms.unwrap_or(150_000)),
            max_concurrent_per_key: config.max_concurrent_per_key,
            max_body_bytes: config.max_body_bytes,
            keys,
            inflight: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Bearer 凭据 → (key, 并发闸)。摘要比对，不存明文。
    fn authenticate(&self, headers: &HeaderMap) -> AuthResult {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim);
        let Some(presented) = presented else {
            return Err(Box::new(unauthorized("missing bearer credential")));
        };
        let mut hasher = sha2::Sha256::new();
        hasher.update(presented.as_bytes());
        let digest = hex::encode(hasher.finalize());
        let Some(key) = self.keys.get(&digest).cloned() else {
            return Err(Box::new(unauthorized("unknown credential")));
        };
        let Some(inflight) = InFlight::admit(&self.inflight, &digest, self.max_concurrent_per_key)
        else {
            return Err(Box::new(rate_limited(
                "gateway key concurrency limit reached",
            )));
        };
        // key 名可进日志（非秘密）；凭据本身绝不出现。
        tracing::info!(key = %key.name, "llm gateway credential accepted");
        Ok((key, inflight))
    }

    /// 当前注册记录；节点重连/换代后取到的总是现行代次，stale 由路由层拒绝。
    fn registration_id(&self) -> Result<String, CallError> {
        self.registry
            .active_record(&self.space_id, &self.provider_endpoint_id)
            .map(|record| record.registration_id)
            .ok_or_else(|| {
                CallError::new(
                    ErrorCode::Unavailable,
                    "llm provider registration is not active",
                )
            })
    }

    /// 中继回复经 pbjson Value 往返，整数退化为 f64；在投影前归一。
    fn normalize(result: Value) -> Value {
        crate::http::normalize_numbers(result)
    }

    pub async fn models(&self, headers: &HeaderMap) -> Response {
        let (key, _inflight) = match self.authenticate(headers) {
            Ok(pair) => pair,
            Err(response) => return *response,
        };
        let source = SourceKey::http(GATEWAY_BINDING, format!("models-{}", new_registration_id()));
        let result = match self.registration_id() {
            Ok(registration_id) => {
                let deadline = tokio::time::Instant::now() + self.timeout;
                self.calls
                    .invoke(
                        &self.consumer,
                        source,
                        &self.provider_endpoint_id,
                        &self.space_id,
                        &registration_id,
                        // 空 consumer_module = 操作员主体路径；批准检查按
                        // 网关消费者执行。
                        "",
                        json!({ "method": "llm/providers/list", "argument": {} }),
                        deadline,
                    )
                    .await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(result) => {
                let result = Self::normalize(result);
                let mut data = Vec::new();
                for provider in result
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    for model in provider
                        .get("models")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                    {
                        let Some(id) = model.get("id").and_then(Value::as_str) else {
                            continue;
                        };
                        if !key.models.is_empty() && !key.models.iter().any(|allowed| allowed == id)
                        {
                            continue;
                        }
                        data.push(json!({
                            "id": id,
                            "object": "model",
                            "owned_by": model.get("ownedBy").and_then(Value::as_str).unwrap_or(""),
                        }));
                    }
                }
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    json!({ "object": "list", "data": data }).to_string(),
                )
                    .into_response()
            }
            Err(error) => relay_error_response(&error),
        }
    }

    pub async fn chat_completions(&self, headers: &HeaderMap, body: Bytes) -> Response {
        let (key, inflight) = match self.authenticate(headers) {
            Ok(pair) => pair,
            Err(response) => return *response,
        };
        if body.len() > self.max_body_bytes {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({ "error": { "message": "request body exceeds the configured limit", "type": "invalid_request_error" } }).to_string(),
            )
                .into_response();
        }
        let parsed: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(error) => return bad_request(format!("request body is not JSON: {error}")),
        };
        let Some(model) = parsed
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return bad_request("model is required".to_string());
        };
        if !key.models.is_empty() && !key.models.contains(&model) {
            return forbidden(format!("credential is not approved for model {model:?}"));
        }
        let Some(messages) = parsed.get("messages").and_then(Value::as_array) else {
            return bad_request("messages array is required".to_string());
        };
        if messages.is_empty() {
            return bad_request("messages must not be empty".to_string());
        }
        // 形状按领域契约收敛：role/content 必须是字符串；网关不做语义解释。
        let mut mapped = Vec::with_capacity(messages.len());
        for message in messages {
            let (Some(role), Some(content)) = (
                message.get("role").and_then(Value::as_str),
                message.get("content").and_then(Value::as_str),
            ) else {
                return bad_request("each message requires string role and content".to_string());
            };
            mapped.push(json!({ "role": role, "content": content }));
        }
        let temperature = parsed.get("temperature").and_then(Value::as_f64);
        let max_tokens = parsed
            .get("max_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let stream = parsed
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let argument = {
            let mut argument = json!({
                "providerId": "",
                "model": model,
                "messages": mapped,
            });
            if let Some(temperature) = temperature {
                argument["temperature"] = json!(temperature);
            }
            if let Some(max_tokens) = max_tokens {
                argument["maxTokens"] = json!(max_tokens);
            }
            argument
        };
        let completion_id = format!("chatcmpl-{}", new_registration_id());
        let source = SourceKey::http(GATEWAY_BINDING, new_registration_id());
        if !stream {
            // 非 SSE：并发闸覆盖整个上游等待；断开不中断已完成接收的处理。
            let result = match self.registration_id() {
                Ok(registration_id) => {
                    let deadline = tokio::time::Instant::now() + self.timeout;
                    self.calls
                        .invoke(
                            &self.consumer,
                            source,
                            &self.provider_endpoint_id,
                            &self.space_id,
                            &registration_id,
                            "",
                            json!({ "method": "llm/chat", "argument": argument }),
                            deadline,
                        )
                        .await
                }
                Err(error) => Err(error),
            };
            let _inflight = inflight;
            return match result {
                Ok(result) => completion_response(&completion_id, &model, &Self::normalize(result)),
                Err(error) => relay_error_response(&error),
            };
        }
        // SSE：上游等待挂在响应体内，客户端断开 → 哨兵 Drop 触发尽力取消；
        // 并发闸与 invoke 任务同生共死。
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let calls = self.calls.clone();
            let consumer = self.consumer.clone();
            let space = self.space_id.clone();
            let endpoint = self.provider_endpoint_id.clone();
            let timeout = self.timeout;
            let _inflight = inflight;
            let source = source.clone();
            tokio::spawn(async move {
                let result = async {
                    let registration_id = calls.registration_for(&space, &endpoint)?;
                    calls
                        .invoke(
                            &consumer,
                            source,
                            &endpoint,
                            &space,
                            &registration_id,
                            "",
                            json!({ "method": "llm/chat", "argument": argument }),
                            tokio::time::Instant::now() + timeout,
                        )
                        .await
                }
                .await;
                drop(_inflight);
                let _ = tx.send(result);
            });
        }
        let guard = Some(CancelOnDrop {
            calls: self.calls.clone(),
            source,
        });
        // 流状态：先等上游结果，再逐事件吐出；guard 保留到事件吐完，
        // 中途被 drop（客户端断开）即触发取消。
        let state = (
            Some(rx),
            guard,
            VecDeque::new(),
            completion_id.clone(),
            model.clone(),
        );
        let stream =
            futures::stream::unfold(state, |(rx, guard, mut queue, id, model)| async move {
                if let Some(event) = queue.pop_front() {
                    return Some((
                        Ok::<_, std::convert::Infallible>(event),
                        (rx, guard, queue, id, model),
                    ));
                }
                let rx = rx?;
                let result = match rx.await {
                    Ok(result) => result,
                    Err(_) => {
                        // invoke 任务消失（关机）：明确失败，不静默空流。
                        queue.push_back(error_event("gateway invoke task ended unexpectedly"));
                        queue.push_back(done_event());
                        drop(guard);
                        return Some((
                            Ok::<_, std::convert::Infallible>(queue.pop_front().expect("queued")),
                            (None, None, queue, id, model),
                        ));
                    }
                };
                match result {
                    Ok(value) => {
                        let value = Self::normalize(value);
                        queue.push_back(chunk_event(&id, &model, &delta_content(&value)));
                        queue.push_back(finish_event(&id, &model, &value));
                        queue.push_back(done_event());
                    }
                    Err(error) => {
                        queue.push_back(error_event(error.message()));
                        queue.push_back(done_event());
                    }
                }
                // 哨兵在这里被消费：正常结束同样触发一次 cancel，
                // 对已终结调用是 requested:false 的空操作。
                drop(guard);
                Some((
                    Ok::<_, std::convert::Infallible>(queue.pop_front().expect("queued")),
                    (None, None, queue, id, model),
                ))
            });
        Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response()
    }
}

fn delta_content(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn chunk_event(id: &str, model: &str, content: &str) -> Event {
    Event::default().data(
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": unix_now(),
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant", "content": content},
                "finish_reason": Value::Null,
            }],
        })
        .to_string(),
    )
}

fn finish_event(id: &str, model: &str, result: &Value) -> Event {
    // finish_reason=unknown 是节点对上游缺失字段的诚实表达；OpenAI 形状
    // 里以 null 呈现，其余原样透传（stop/length/content_filter/tool_calls）。
    let finish_reason = result
        .get("finishReason")
        .and_then(Value::as_str)
        .filter(|reason| *reason != "unknown")
        .map(|reason| json!(reason))
        .unwrap_or(Value::Null);
    Event::default().data(
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": unix_now(),
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": finish_reason,
            }],
        })
        .to_string(),
    )
}

fn error_event(message: &str) -> Event {
    Event::default()
        .event("error")
        .data(json!({ "error": { "message": message } }).to_string())
}

fn done_event() -> Event {
    Event::default().data("[DONE]")
}

fn completion_response(id: &str, requested_model: &str, result: &Value) -> Response {
    let finish_reason = result
        .get("finishReason")
        .and_then(Value::as_str)
        .filter(|reason| *reason != "unknown")
        .map(|reason| json!(reason))
        .unwrap_or(Value::Null);
    let mut body = json!({
        "id": id,
        "object": "chat.completion",
        "created": unix_now(),
        "model": result.get("model").and_then(Value::as_str).unwrap_or(requested_model),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": result.get("content").and_then(Value::as_str).unwrap_or_default(),
            },
            "finish_reason": finish_reason,
        }],
    });
    if let Some(usage) = result.get("usage") {
        body["usage"] = json!({
            "prompt_tokens": usage.get("promptTokens"),
            "completion_tokens": usage.get("completionTokens"),
            "total_tokens": usage.get("totalTokens"),
        });
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn relay_error_response(error: &CallError) -> Response {
    let (status, kind) = match error.code_enum() {
        Some(ErrorCode::BadRequest) | Some(ErrorCode::UnknownMethod) => {
            (StatusCode::BAD_REQUEST, "invalid_request_error")
        }
        Some(ErrorCode::Forbidden) => (StatusCode::FORBIDDEN, "permission_error"),
        Some(ErrorCode::Unauthorized) => (StatusCode::UNAUTHORIZED, "authentication_error"),
        Some(ErrorCode::QuotaExceeded) => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error"),
        Some(ErrorCode::Unavailable) | Some(ErrorCode::StaleRevision) => {
            (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable")
        }
        Some(ErrorCode::Timeout) => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
        Some(ErrorCode::OutcomeUnknown) => (StatusCode::BAD_GATEWAY, "outcome_unknown"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": { "message": error.message(), "type": kind } }).to_string(),
    )
        .into_response()
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        json!({ "error": { "message": message, "type": "authentication_error" } }).to_string(),
    )
        .into_response()
}

fn forbidden(message: String) -> Response {
    (
        StatusCode::FORBIDDEN,
        json!({ "error": { "message": message, "type": "permission_error" } }).to_string(),
    )
        .into_response()
}

fn bad_request(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        json!({ "error": { "message": message, "type": "invalid_request_error" } }).to_string(),
    )
        .into_response()
}

fn rate_limited(message: &str) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        json!({ "error": { "message": message, "type": "rate_limit_error" } }).to_string(),
    )
        .into_response()
}

/// axum 处理器：网关未配置时显式 404，不静默回退。
pub async fn v1_models(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
) -> Response {
    let Some(gateway) = state.llm_gateway.as_ref() else {
        return not_configured();
    };
    gateway.models(&headers).await
}

pub async fn v1_chat_completions(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(gateway) = state.llm_gateway.as_ref() else {
        return not_configured();
    };
    gateway.chat_completions(&headers, body).await
}

fn not_configured() -> Response {
    (
        StatusCode::NOT_FOUND,
        json!({ "error": { "message": "llm gateway is not configured on this host", "type": "service_unavailable" } }).to_string(),
    )
        .into_response()
}
