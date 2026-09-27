use std::sync::Arc;

use conex_core::policy::is_valid_resource;
use conex_core::{CallError, CallResult, Caller, Endpoint, Host, Policy, ResourceClaim, StaticPolicy};
use conex_proto;
use serde_json::Value;

use crate::agent::HostSide;
use crate::config::HostConfig;
use crate::remote::RemoteConnections;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 100;

#[derive(Clone)]
pub struct EndpointCatalog {
    entries: Arc<Vec<CatalogEntry>>,
    policy: Arc<StaticPolicy>,
    connections: Option<Arc<RemoteConnections>>,
}

#[derive(Clone)]
struct CatalogEntry {
    endpoint: Endpoint,
    kind: i32,
    agent_id: Option<String>,
    display_name: String,
}

impl EndpointCatalog {
    pub fn from_config(
        config: &HostConfig,
        host: &Host,
        policy: Arc<StaticPolicy>,
        host_side: Option<&HostSide>,
    ) -> CallResult<Self> {
        let mut entries = Vec::with_capacity(config.endpoints.len());
        for configured in &config.endpoints {
            let endpoint = host
                .registry()
                .endpoint(&configured.id)
                .cloned()
                .ok_or_else(|| internal(format!("catalog endpoint {} is not installed", configured.id)))?;
            entries.push(CatalogEntry {
                endpoint,
                kind: attachment_kind(configured.kind.as_str()),
                agent_id: configured.agent_id.clone(),
                display_name: configured
                    .agent_id
                    .as_deref()
                    .and_then(|agent_id| {
                        config
                            .agents
                            .iter()
                            .find(|agent| agent.id == agent_id)
                            .and_then(|agent| agent.display_name.clone())
                    })
                    .unwrap_or_else(|| configured.id.clone()),
            });
        }
        Ok(Self {
            entries: Arc::new(entries),
            policy,
            connections: host_side.map(|side| side.connections.clone()),
        })
    }

    pub fn from_parts(
        entries: Vec<(Endpoint, i32, Option<String>)>,
        policy: Arc<StaticPolicy>,
        connections: Option<Arc<RemoteConnections>>,
    ) -> Self {
        Self {
            entries: Arc::new(
                entries
                    .into_iter()
                    .map(|(endpoint, kind, agent_id)| CatalogEntry {
                        display_name: endpoint.id.clone(),
                        endpoint,
                        kind,
                        agent_id,
                    })
                    .collect(),
            ),
            policy,
            connections,
        }
    }

    pub async fn list(&self, caller: &Caller, input: Value) -> CallResult<Value> {
        let request: conex_proto::EndpointListRequest = serde_json::from_value(input)
            .map_err(|error| bad(format!("invalid endpoint/list request: {error}")))?;
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(bad("limit must be in range 1..=100"));
        }
        if request
            .after_endpoint_id
            .as_deref()
            .is_some_and(str::is_empty)
        {
            return Err(bad("afterEndpointId must not be empty"));
        }

        let mut summaries = Vec::new();
        for entry in self.entries.iter() {
            if entry.endpoint.tenant_id != caller.tenant_id {
                continue;
            }
            if request
                .after_endpoint_id
                .as_deref()
                .is_some_and(|cursor| entry.endpoint.id.as_str() <= cursor)
            {
                continue;
            }
            let Some(summary) = self.summary(caller, entry).await? else {
                continue;
            };
            summaries.push(summary);
        }
        summaries.sort_by(|a, b| a.endpoint_id.cmp(&b.endpoint_id));

        let has_more = summaries.len() > limit as usize;
        if has_more {
            summaries.truncate(limit as usize);
        }
        let next_after_endpoint_id = has_more
            .then(|| summaries.last().map(|summary| summary.endpoint_id.clone()))
            .flatten();
        serde_json::to_value(conex_proto::EndpointListResult {
            endpoints: summaries,
            next_after_endpoint_id,
        })
        .map_err(|error| internal(format!("cannot encode endpoint/list response: {error}")))
    }

    async fn summary(
        &self,
        caller: &Caller,
        entry: &CatalogEntry,
    ) -> CallResult<Option<conex_proto::EndpointSummary>> {
        let actions = [
            ("source/list", "list"),
            ("source/read", "read"),
            ("source/search", "search"),
        ];
        let mut available_methods = Vec::new();
        let mut authorized_scopes = Vec::new();
        let rules = self.policy.rules_for(caller, &entry.endpoint);
        for (method, action) in actions {
            if !entry.endpoint.provides.iter().any(|provided| provided == method) {
                continue;
            }
            let mut scopes = Vec::new();
            for rule in &rules {
                if !rule.actions.iter().any(|candidate| candidate == action)
                    || !is_valid_resource(&rule.root)
                {
                    continue;
                }
                let claim = ResourceClaim {
                    resource_id: rule.root.clone(),
                    action: action.to_string(),
                    subtree: method != "source/read",
                };
                if self.policy.authorize(caller, &entry.endpoint, &claim).is_err() {
                    continue;
                }
                let scope = conex_proto::AuthorizedScope {
                    method: method.to_string(),
                    root: rule.root.clone(),
                    subtree: rule.subtree,
                };
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
            if !scopes.is_empty() {
                available_methods.push(method.to_string());
                authorized_scopes.extend(scopes);
            }
        }
        if available_methods.is_empty() {
            return Ok(None);
        }
        let connection_state = if entry.kind == conex_proto::AttachmentKind::ReverseAgent as i32 {
            match (&self.connections, &entry.agent_id) {
                (Some(connections), Some(agent_id)) if connections.is_ready(agent_id).await => {
                    conex_proto::ConnectionState::Ready as i32
                }
                _ => conex_proto::ConnectionState::Offline as i32,
            }
        } else {
            conex_proto::ConnectionState::NotApplicable as i32
        };
        Ok(Some(conex_proto::EndpointSummary {
            endpoint_id: entry.endpoint.id.clone(),
            display_name: entry.display_name.clone(),
            provider_id: entry.endpoint.provider_id.clone(),
            attachment_kind: entry.kind,
            agent_id: entry.agent_id.clone(),
            region: None,
            connection_state,
            available_methods,
            authorized_scopes,
        }))
    }
}

fn attachment_kind(kind: &str) -> i32 {
    match kind {
        "source-fs" => conex_proto::AttachmentKind::Embedded as i32,
        "source-http-catalog" => conex_proto::AttachmentKind::OutboundHttp as i32,
        "source-remote" => conex_proto::AttachmentKind::ReverseAgent as i32,
        _ => conex_proto::AttachmentKind::Unspecified as i32,
    }
}

fn bad(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, message)
}

fn internal(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::Internal, message)
}
