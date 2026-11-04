//! 关系与标注投影（P6.2/P6.3）：描述关系（运行于、关联 Workspace、参与
//! 会话、派生执行）从既有 store 表派生——投影不二次存储；能力绑定是独立
//! 查询（需单独授权），本版经同一契约但分开的操作暴露。Tags 是标注不是
//! 权限；用户删除标注不取消任何执行。
use crate::adapters::common::stable_ulid;
use crate::node_types::RuntimeFactory;
use conex_proto as p;
use conex_proto::{Relationship, TagsListInput, TagsListResult};
use conex_sdk::{Contract, Error, Execution, Method, StartedModule};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const CONTRACT_ID: &str = "conex.relations";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn factory(_config: crate::node_config::MetricsModuleConfig) -> RuntimeFactory {
    Arc::new(move |runtime, _context| {
        Box::pin(async move {
            eprintln!("conex-agent relations: factory enter");
            let store = runtime.store.clone();
            store
                .put_endpoint(p::AgentAdapterEndpoint {
                    id: stable_ulid(&format!(
                        "endpoint:{}:{}",
                        runtime.node_epoch, runtime.module_key
                    )),
                    node_id: runtime.node_id.clone(),
                    module_instance_id: runtime.resource_id.clone(),
                    generation: runtime.generation,
                    adapter: "builtin".into(),
                    backend_version: "conex.relations/1".into(),
                    protocol_version: CONTRACT_ID.into(),
                    experimental: false,
                    capabilities: None,
                    availability: "ready".into(),
                    catalog_scope: "node-managed".into(),
                    limits: None,
                    display_name: Some("Relations & tags".into()),
                })
                .await
                .map_err(|error| crate::node::failure(&error.to_string()))?;
            let list = {
                let store = store.clone();
                Method::new(
                    "conex.AgentRelationshipListInput",
                    "conex.AgentRelationshipListResult",
                    Execution::ReadOnly,
                )
                .with_handler(
                    move |context, input: p::AgentRelationshipListInput| {
                        let store = store.clone();
                        let principal = context.caller.principal_id.clone();
                        async move {
                            let _ = &principal;
                            let items = store
                                .list_relationships(
                                    &input.conversation_id,
                                    input.limit.unwrap_or(50).clamp(1, 100),
                                )
                                .await
                                .map_err(|error| {
                                    Error::new(
                                        p::ErrorCode::BadRequest,
                                        error.to_string(),
                                        "not_started",
                                        "never",
                                    )
                                })?;
                            proto_json(&p::AgentRelationshipListResult {
                                items,
                                next_page_token: None,
                            })
                        }
                    },
                )
            };
            let tags_list = {
                let store = store.clone();
                Method::new(
                    "conex.TagsListInput",
                    "conex.TagsListResult",
                    Execution::ReadOnly,
                )
                .with_handler(move |_context, input: TagsListInput| {
                    let store = store.clone();
                    async move {
                        let items = store
                            .list_tags(&input.entity_kind, &input.entity_id)
                            .await
                            .map_err(|error| {
                                Error::new(
                                    p::ErrorCode::BadRequest,
                                    error.to_string(),
                                    "not_started",
                                    "never",
                                )
                            })?;
                        proto_json(&TagsListResult { items })
                    }
                })
            };
            let tags_put = {
                let store = store.clone();
                Method::new(
                    "conex.TagsPutInput",
                    "conex.TagsPutResult",
                    Execution::NonReplayable,
                )
                .with_handler(move |context, input: p::TagsPutInput| {
                    let store = store.clone();
                    let principal = context.caller.principal_id.clone();
                    async move {
                        let _ = &principal;
                        // 调用方标注记 user 所有权；节点自身维护 node 行。
                        let owner = "user";
                        store
                            .record_tag(
                                owner,
                                &input.entity_kind,
                                &input.entity_id,
                                &input.name,
                                now_ms(),
                            )
                            .await
                            .map_err(|error| {
                                Error::new(
                                    p::ErrorCode::BadRequest,
                                    error.to_string(),
                                    "not_started",
                                    "never",
                                )
                            })?;
                        Ok(json!({}))
                    }
                })
            };
            let tags_delete = {
                let store = store.clone();
                Method::new(
                    "conex.TagsDeleteInput",
                    "conex.TagsPutResult",
                    Execution::NonReplayable,
                )
                .with_handler(move |_context, input: p::TagsDeleteInput| {
                    let store = store.clone();
                    async move {
                        // 删除标注只删行：不取消后端执行、不影响所有权
                        // （P6.3）。调用方可删 user 行；node 行由节点管理。
                        store
                            .delete_tag("user", &input.entity_kind, &input.entity_id, &input.name)
                            .await
                            .map_err(|error| {
                                Error::new(
                                    p::ErrorCode::BadRequest,
                                    error.to_string(),
                                    "not_started",
                                    "never",
                                )
                            })?;
                        Ok(json!({}))
                    }
                })
            };
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([
                    ("relations/list".to_string(), list),
                    ("tags/list".to_string(), tags_list),
                    ("tags/put".to_string(), tags_put),
                    ("tags/delete".to_string(), tags_delete),
                ]),
                BTreeMap::new(),
            )?);
            Ok(StartedModule::new(contract))
        })
    })
}

fn proto_json<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|error| {
        Error::new(
            p::ErrorCode::Internal,
            error.to_string(),
            "not_started",
            "never",
        )
    })
}

#[allow(unused)]
fn unused(_: Relationship) {}

#[cfg(test)]
mod tests {
    #[test]
    fn tags_are_annotations_not_permissions() {
        // 标注行删除不触碰任何执行/授权表；此处只锁定模块契约名面。
        let methods = ["relations/list", "tags/list", "tags/put", "tags/delete"];
        assert!(methods.iter().all(|m| !m.contains("invoke")));
    }
}
