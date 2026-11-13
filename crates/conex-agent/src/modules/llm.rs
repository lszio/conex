//! LLM 提供者模块（MVP-P10.2：docs/plans/2026-11-12-conex-mvp-p10-llm-relay.md
//! §3）。节点普通领域模块，经正常发布导出 `conex.llm`；适配器对接 OpenAI
//! 兼容上游（首实例 omniroute），上游地址与凭据留节点配置，凭据不进任何
//! 投影。providers/list 带有界缓存与健康投影；chat 一次返回完整结果（首版
//! 无增量），取消映射为结果未知——取消已请求不等于上游已停止生成。
use crate::node_types::RuntimeFactory;
use conex_proto as p;
use conex_proto::now_ms;
use conex_sdk::{Cancellation, Contract, Error, Execution, Method, StartedModule};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub const CONTRACT_ID: &str = "conex.llm";
/// providers/list 的缓存刷新窗口：窗口内复用上次成功探测（health=live），
/// 超窗先尝试刷新；刷新失败但旧数据在 → stale，从未成功 → unreachable。
const CACHE_TTL_MS: u64 = 30_000;

struct ProviderState {
    config: crate::node_config::LlmProviderConfig,
    client: crate::modules::llm_openai::UpstreamClient,
    cache: Mutex<CacheState>,
}

/// 探测缓存：models/latency 只来自最近一次成功探测；checked_at_ms 是最近
/// 一次尝试（含失败），detail 是最近一次失败摘要（无凭据/正文）。
#[derive(Default)]
struct CacheState {
    models: Option<Vec<p::LlmModel>>,
    latency_ms: Option<u64>,
    checked_at_ms: Option<u64>,
    last_ok_at_ms: Option<u64>,
    detail: Option<String>,
}

pub fn factory(config: crate::node_config::LlmModuleConfig) -> RuntimeFactory {
    Arc::new(move |_runtime, _context| {
        // RuntimeFactory 是 Fn：捕获的配置在每次启动时克隆进异步块。
        let config = config.clone();
        Box::pin(async move {
            let mut providers = Vec::new();
            for provider in &config.providers {
                // 配置校验已保证 >= 1s；这里再兜底，防手写绕过校验。
                let timeout =
                    Duration::from_millis(provider.timeout_ms.unwrap_or(120_000).max(1_000));
                // 凭据在节点启动时从批准的 secret backend 解析进内存；
                // 之后只经 bearer 头出海，不进投影/错误/日志。
                let api_key = match &provider.api_key_backend {
                    Some(backend) => Some(
                        crate::node_config::read_secret(backend)
                            .map_err(|error| crate::node::failure(&error))?,
                    ),
                    None => None,
                };
                let client = crate::modules::llm_openai::UpstreamClient::new(
                    &provider.base_url,
                    api_key,
                    timeout,
                )
                .map_err(|error| crate::node::failure(&error))?;
                providers.push(Arc::new(ProviderState {
                    config: provider.clone(),
                    client,
                    cache: Mutex::new(CacheState::default()),
                }));
            }
            let providers = Arc::new(providers);

            let list = {
                let providers = providers.clone();
                Method::new(
                    "conex.LlmProvidersListInput",
                    "conex.LlmProvidersListResult",
                    Execution::ReadOnly,
                )
                .with_handler(move |_context, _input: p::LlmProvidersListInput| {
                    let providers = providers.clone();
                    async move {
                        let mut items = Vec::new();
                        for provider in providers.iter() {
                            items.push(provider_info_json(&refresh_and_project(provider).await));
                        }
                        Ok(json!({ "items": items }))
                    }
                })
            };

            let chat = {
                let providers = providers.clone();
                Method::new(
                    "conex.LlmChatInput",
                    "conex.LlmChatResult",
                    Execution::NonReplayable,
                )
                .with_handler(move |context, input: p::LlmChatInput| {
                    let providers = providers.clone();
                    async move { run_chat(&providers, context.cancellation, input).await }
                })
            };

            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([
                    ("llm/providers/list".to_string(), list),
                    ("llm/chat".to_string(), chat),
                ]),
                BTreeMap::new(),
            )?);
            Ok(StartedModule::new(contract))
        })
    })
}

/// 读路径刷新：超过 TTL 先探测；失败保留旧数据降级 stale，从未成功报
/// unreachable。health 只反映探测事实，不把「配置存在」当成「上游在线」。
async fn refresh_and_project(provider: &ProviderState) -> p::LlmProviderInfo {
    let now = now_ms();
    let should_refresh = {
        let cache = provider.cache.lock().await;
        match cache.checked_at_ms {
            Some(checked) => now.saturating_sub(checked) > CACHE_TTL_MS,
            None => true,
        }
    };
    if should_refresh {
        match provider.client.list_models().await {
            Ok((models, latency)) => {
                let allowlist = &provider.config.models_allowlist;
                let filtered: Vec<p::LlmModel> = models
                    .into_iter()
                    .filter(|model| allowlist.is_empty() || allowlist.contains(&model.id))
                    .map(|model| p::LlmModel {
                        id: model.id,
                        owned_by: model.owned_by.unwrap_or_default(),
                        context_window: None,
                        tools: "unknown".into(),
                    })
                    .collect();
                let mut cache = provider.cache.lock().await;
                cache.models = Some(filtered);
                cache.latency_ms = Some(latency);
                cache.checked_at_ms = Some(now);
                cache.last_ok_at_ms = Some(now);
                cache.detail = None;
            }
            Err(error) => {
                let mut cache = provider.cache.lock().await;
                // 保留旧模型列表（stale 投影），只更新尝试时间与失败摘要。
                cache.checked_at_ms = Some(now);
                cache.latency_ms = None;
                cache.detail = Some(error.0.message.clone());
            }
        }
    }
    let cache = provider.cache.lock().await;
    let health = match (cache.models.as_ref(), cache.last_ok_at_ms) {
        (Some(_), Some(ok)) if now.saturating_sub(ok) <= CACHE_TTL_MS => "live",
        (Some(_), Some(_)) => "stale",
        _ => "unreachable",
    };
    p::LlmProviderInfo {
        id: provider.config.id.clone(),
        kind: "openai-compat".into(),
        base_url: provider.config.base_url.clone(),
        health: health.into(),
        checked_at_ms: cache.checked_at_ms,
        latency_ms: cache.latency_ms,
        models: cache.models.clone().unwrap_or_default(),
        detail: cache.detail.clone(),
    }
}

/// 投影只含存在字段：optional 的 null 会撞严格输出解码，直接省略。
fn provider_info_json(info: &p::LlmProviderInfo) -> Value {
    let mut value = json!({
        "id": info.id,
        "kind": info.kind,
        "baseUrl": info.base_url,
        "health": info.health,
        "models": info
            .models
            .iter()
            .map(|model| {
                let mut entry = json!({
                    "id": model.id,
                    "ownedBy": model.owned_by,
                    "tools": model.tools,
                });
                if let Some(window) = model.context_window {
                    entry["contextWindow"] = json!(window.to_string());
                }
                entry
            })
            .collect::<Vec<_>>(),
    });
    if let Some(checked) = info.checked_at_ms {
        value["checkedAtMs"] = json!(checked.to_string());
    }
    if let Some(latency) = info.latency_ms {
        value["latencyMs"] = json!(latency.to_string());
    }
    if let Some(detail) = &info.detail {
        value["detail"] = json!(detail);
    }
    value
}

/// 上游之前即可判定的输入规则：全部拒绝发生在任何网络行为前。
fn validate_chat_input(
    providers: &[Arc<ProviderState>],
    input: &p::LlmChatInput,
) -> Result<Vec<Value>, Error> {
    if input.stream == Some(true) {
        return Err(bad_request(
            "llm chat streaming is not supported by this contract version; omit stream",
        ));
    }
    let provider = resolve_provider(providers, &input.provider_id)?;
    let allowlist = &provider.config.models_allowlist;
    if !allowlist.is_empty() && !allowlist.contains(&input.model) {
        return Err(bad_request(format!(
            "model {:?} is not published by provider {:?}",
            input.model, provider.config.id
        )));
    }
    let messages: Vec<Value> = input
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": message.role,
                "content": message.content,
            })
        })
        .collect();
    if messages.is_empty() {
        return Err(bad_request("llm chat requires at least one message"));
    }
    Ok(messages)
}

async fn run_chat(
    providers: &[Arc<ProviderState>],
    mut cancellation: Cancellation,
    input: p::LlmChatInput,
) -> Result<Value, Error> {
    let messages = validate_chat_input(providers, &input)?;
    let provider = resolve_provider(providers, &input.provider_id)?;
    let client = &provider.client;
    let model = input.model.clone();
    let temperature = input.temperature;
    let max_tokens = input.max_tokens;
    // 取消映射：取消信号先到就停等上游；上游往返完成后结果照常返回——
    // 一次补全的副作用在上游已发生，取消不能伪造成未发生。
    let completion = tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            return Err(Error::new(
                p::ErrorCode::OutcomeUnknown,
                "llm chat cancelled; upstream completion outcome unknown",
                "unknown",
                "never",
            ));
        }
        result = client.chat(&model, &messages, temperature, max_tokens) => result?,
    };
    let mut result = json!({
        "content": completion.content,
        "finishReason": completion.finish_reason,
        "model": completion.model,
        "providerId": provider.config.id,
    });
    if let Some(usage) = completion.usage {
        let mut entry = json!({});
        if let Some(tokens) = usage.prompt_tokens {
            entry["promptTokens"] = json!(tokens);
        }
        if let Some(tokens) = usage.completion_tokens {
            entry["completionTokens"] = json!(tokens);
        }
        if let Some(tokens) = usage.total_tokens {
            entry["totalTokens"] = json!(tokens);
        }
        if entry.as_object().is_some_and(|fields| !fields.is_empty()) {
            result["usage"] = entry;
        }
    }
    Ok(result)
}

fn bad_request(message: impl Into<String>) -> Error {
    Error::new(p::ErrorCode::BadRequest, message, "not_started", "never")
}

fn resolve_provider<'a>(
    providers: &'a [Arc<ProviderState>],
    provider_id: &str,
) -> Result<&'a Arc<ProviderState>, Error> {
    if provider_id.is_empty() {
        if providers.len() == 1 {
            return Ok(&providers[0]);
        }
        return Err(bad_request(
            "providerId is required unless the node has exactly one llm provider",
        ));
    }
    providers
        .iter()
        .find(|provider| provider.config.id == provider_id)
        .ok_or_else(|| bad_request(format!("unknown llm provider {provider_id:?}")))
}

/// 形状守卫：输入校验全部发生在上游之前（P10 计划 §3）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_rejects_stream_before_any_network() {
        let error = validate_chat_input(
            &[],
            &p::LlmChatInput {
                provider_id: String::new(),
                model: "any".into(),
                messages: vec![],
                temperature: None,
                max_tokens: None,
                stream: Some(true),
            },
        )
        .expect_err("stream=true must be refused");
        assert_eq!(
            error.0.message,
            "llm chat streaming is not supported by this contract version; omit stream"
        );
    }

    #[test]
    fn chat_rejects_ambiguous_provider_and_empty_messages() {
        let input = p::LlmChatInput {
            provider_id: String::new(),
            model: "m".into(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: None,
        };
        let error = validate_chat_input(&[], &input).expect_err("no providers configured");
        assert!(
            error.0.message.contains("unknown llm provider")
                || error.0.message.contains("providerId"),
            "{}",
            error.0.message
        );
        let message = p::LlmChatMessage {
            role: "user".into(),
            content: "hi".into(),
            tool_call_id: None,
        };
        let input = p::LlmChatInput {
            messages: vec![message],
            ..input
        };
        let error = validate_chat_input(&[], &input).expect_err("empty provider set still refuses");
        assert!(!error.0.message.is_empty());
    }
}
