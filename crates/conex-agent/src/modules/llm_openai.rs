//! OpenAI 兼容上游客户端（P10.2）。首实例 omniroute；Ollama `/v1`、vllm 等
//! 同面服务共用本适配。请求与解析只按公共面实现，实例差异在接入时核对，
//! 不按在线文档断言。凭据只存在于本模块内存，错误摘要绝不回显凭据或
//! 上游响应体全文；响应体有界，防慢上游/超大响应拖垮节点。
//!
//! 错误的执行语义按「请求是否已送达」区分：连接/发送失败是未开始
//! （not_started/safe）；已送达后的读写失败对 chat 是结果未知
//! （unknown/never）——上游可能已开始生成。
use conex_proto::ErrorCode;
use conex_sdk::Error;
use serde_json::Value;
use std::time::Duration;

/// 单个响应体上限：模型列表与补全结果都是小 JSON；超限按上游故障处理。
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// 上游错误摘要上限：只取 error.message 的前缀，足够排障、不够携带正文。
const MAX_ERROR_DETAIL: usize = 200;

pub struct UpstreamClient {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
    timeout: Duration,
}

pub struct UpstreamModel {
    pub id: String,
    pub owned_by: Option<String>,
}

pub struct UpstreamCompletion {
    pub content: String,
    pub finish_reason: String,
    pub usage: Option<UpstreamUsage>,
    pub model: String,
}

pub struct UpstreamUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

impl UpstreamClient {
    pub fn new(base_url: &str, api_key: Option<String>, timeout: Duration) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| format!("upstream http client: {error}"))?;
        Ok(Self {
            // 装配期已校验 http(s) 与无 query/fragment；这里只归一斜杠。
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            http,
            timeout,
        })
    }

    /// GET {base}/models。返回模型列表与一次往返耗时（毫秒）供健康投影。
    pub async fn list_models(&self) -> Result<(Vec<UpstreamModel>, u64), Error> {
        let started = std::time::Instant::now();
        let response = self
            .send(self.http.get(format!("{}/models", self.base_url)))
            .await?;
        let status = response.status();
        let body = self.read_bounded(response, false).await?;
        if !status.is_success() {
            return Err(upstream_err(
                map_status(status.as_u16()),
                sanitized_http_error(status.as_u16(), &body),
                false,
            ));
        }
        let parsed: Value = serde_json::from_slice(&body).map_err(|error| {
            upstream_err(
                ErrorCode::Internal,
                format!("upstream /models body: {error}"),
                false,
            )
        })?;
        let entries = parsed
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                upstream_err(
                    ErrorCode::Internal,
                    "upstream /models response has no data array",
                    false,
                )
            })?;
        let mut models = Vec::with_capacity(entries.len());
        for entry in entries {
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            models.push(UpstreamModel {
                id: id.to_string(),
                owned_by: entry
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
        Ok((models, started.elapsed().as_millis() as u64))
    }

    /// POST {base}/chat/completions（stream:false）。一次返回完整结果；
    /// 增量流式属后续契约扩展。已送达后的失败按结果未知表达。
    #[allow(clippy::too_many_arguments)]
    pub async fn chat(
        &self,
        model: &str,
        messages: &[Value],
        temperature: Option<f64>,
        max_tokens: Option<u32>,
    ) -> Result<UpstreamCompletion, Error> {
        let mut payload = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });
        if let Some(temperature) = temperature {
            payload["temperature"] = serde_json::json!(temperature);
        }
        if let Some(max_tokens) = max_tokens {
            payload["max_tokens"] = serde_json::json!(max_tokens);
        }
        let response = self
            .send(
                self.http
                    .post(format!("{}/chat/completions", self.base_url))
                    .json(&payload),
            )
            .await?;
        let status = response.status();
        let body = self.read_bounded(response, true).await?;
        if !status.is_success() {
            // 收到明确状态码 = 上游明确拒绝（本次生成未发生），终态已知；
            // 不降级成结果未知。retry=never：副作用操作不由节点自动重放。
            return Err(Error::new(
                map_status(status.as_u16()),
                sanitized_http_error(status.as_u16(), &body),
                "completed",
                "never",
            ));
        }
        let parsed: Value = serde_json::from_slice(&body).map_err(|error| {
            upstream_err(
                ErrorCode::Internal,
                format!("upstream completion body: {error}"),
                true,
            )
        })?;
        let choice = parsed
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .ok_or_else(|| {
                upstream_err(
                    ErrorCode::Internal,
                    "upstream completion has no choices",
                    true,
                )
            })?;
        let content = choice
            .pointer("/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // finish_reason 缺失按 unknown 表达，不编造 stop。
        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let usage = parsed.get("usage").map(|usage| UpstreamUsage {
            prompt_tokens: usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
            completion_tokens: usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
            total_tokens: usage
                .get("total_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
        });
        let model = parsed
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(model)
            .to_string();
        Ok(UpstreamCompletion {
            content,
            finish_reason,
            usage,
            model,
        })
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, Error> {
        let request = if let Some(key) = &self.api_key {
            request.bearer_auth(key)
        } else {
            request
        };
        // 发送阶段失败 = 请求未送达：not_started/safe，不冒充结果未知。
        request.send().await.map_err(|error| {
            if error.is_timeout() {
                upstream_err(
                    ErrorCode::Timeout,
                    format!(
                        "upstream exceeded {:?} budget before completing",
                        self.timeout
                    ),
                    false,
                )
            } else {
                upstream_err(
                    ErrorCode::Unavailable,
                    format!("upstream unreachable: {error}"),
                    false,
                )
            }
        })
    }

    async fn read_bounded(
        &self,
        mut response: reqwest::Response,
        delivered: bool,
    ) -> Result<Vec<u8>, Error> {
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            upstream_err(
                ErrorCode::Unavailable,
                format!("upstream read failed: {error}"),
                delivered,
            )
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(upstream_err(
                    ErrorCode::Internal,
                    "upstream response exceeds the bounded body limit",
                    delivered,
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn map_status(status: u16) -> ErrorCode {
    match status {
        401 | 403 => ErrorCode::Unavailable,
        429 => ErrorCode::QuotaExceeded,
        400..=499 => ErrorCode::BadRequest,
        _ => ErrorCode::Internal,
    }
}

/// 上游 4xx/5xx 摘要：状态码 + 上游 error.message 前缀（截断）。绝不回显
/// 请求体、Authorization 或上游响应全文——错误会进入 hub 审计与日志。
fn sanitized_http_error(status: u16, body: &[u8]) -> String {
    let detail = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|parsed| {
            parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|mut message| {
            message.truncate(MAX_ERROR_DETAIL);
            message
        })
        .unwrap_or_default();
    if detail.is_empty() {
        format!("upstream returned HTTP {status}")
    } else {
        format!("upstream returned HTTP {status}: {detail}")
    }
}

fn upstream_err(code: ErrorCode, message: impl Into<String>, delivered: bool) -> Error {
    if delivered {
        // 副作用已可能发生：失败联是结果未知，不由节点自判重试安全。
        Error::new(ErrorCode::OutcomeUnknown, message, "unknown", "never")
    } else {
        Error::new(code, message, "not_started", "safe")
    }
}
