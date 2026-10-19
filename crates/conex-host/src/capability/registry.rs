//! S1 模块注册表：`(spaceId, providerEndpointId)` 主键、registrationId 绑定
//! Node/Link 代次、staged→active 原子激活与代次 fence（docs/contracts/
//! s1-capabilities.md §4.2、§6；docs/plans/2026-10-17-conex-s1.md T02）。
//!
//! 规则：
//! - 注册校验完整发布批准（精确元组，默认 deny）+ 契约为预安装描述 +
//!   提供端对批准契约全部成员的 handler 声明；失败不改动旧代次，健康旧
//!   连接不受影响。
//! - 同一 Node 的新代次首个成功注册原子替换旧发布；旧代次迟到的注册、
//!   撤销、断线清理一律被 fence，绝不删除或覆盖新代次的注册。
//! - 同一活动代次重复注册同一模块键 → conflict；旧 registrationId →
//!   stale_revision；无权主体 → forbidden（不泄漏注册是否存在）。
//! - list 只投影调用者消费批准内的 methods/states；编码超帧显式
//!   payload_too_large，绝不静默截断。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use conex_core::CallError;
use conex_core::capability::{ApprovalSet, ContractRegistry};
use conex_proto;
use serde_json::{Value, json};

/// Default per-node ceiling; each ready link may negotiate a lower limit.
pub const DEFAULT_MAX_MODULES: usize = 64;

/// 模块键：`(spaceId, providerEndpointId)`（契约 §3）。纯字符串即可，
/// 键的唯一性由 `ApprovalSet::validate_keys` 在装载期强制。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleKey {
    pub space_id: String,
    pub provider_endpoint_id: String,
}

/// 一次成功发布的活动注册。`registration_id` 绑定 Node、Link 代次、
/// Space、接入与契约，是 hierarchical ULID（契约 §2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRecord {
    pub registration_id: String,
    pub node_id: String,
    pub space_id: String,
    pub integration_id: String,
    pub provider_endpoint_id: String,
    pub contract_id: String,
    /// 发布该注册的 WSS Link 代次；cleanup/撤销都以它作 fence。
    pub generation: u64,
}

/// `module/register` 的输入（§6.1 的四个标识）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRequest {
    pub space_id: String,
    pub integration_id: String,
    pub provider_endpoint_id: String,
    pub contract_id: String,
}

/// 提供端对其“已实现契约成员”的声明。wire 冻结（`ModuleRegisterInput` 只有
/// 四个标识字段），SDK 在本地强制 handler 完整性后才发送；中心侧接收的声明
/// 即批准契约的完整成员集（§6.1：中心只能校验声明和批准）。注册表保留该
/// 入参作为显式检查点，未来 wire 携带逐成员声明时无需改注册逻辑。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandlerDeclaration {
    pub methods: Vec<String>,
    pub states: Vec<String>,
}

#[derive(Debug)]
struct NodeState {
    /// 当前活动代次；首个成功注册才写入（staged→active 原子点）。
    generation: u64,
    modules: HashMap<ModuleKey, ModuleRecord>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("{0}")]
    BadRequest(String),
    #[error("module ({space}, {endpoint}) is not approved for node {node}")]
    NotApproved {
        space: String,
        endpoint: String,
        node: String,
    },
    #[error("contract {0} is not installed")]
    UnknownContract(String),
    #[error("provider declares no handler for contract member {0}")]
    HandlerMissing(String),
    #[error("module ({space}, {endpoint}) is already registered in the active generation")]
    Conflict { space: String, endpoint: String },
    #[error("{0}")]
    StaleRevision(String),
    #[error("module capacity exceeded for node {0}")]
    QuotaExceeded(String),
    #[error("module list payload exceeds the frame budget")]
    PayloadTooLarge,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    Internal(String),
}

impl ModuleError {
    pub fn into_call(self) -> CallError {
        let code = match &self {
            ModuleError::BadRequest(_)
            | ModuleError::UnknownContract(_)
            | ModuleError::HandlerMissing(_) => conex_proto::ErrorCode::BadRequest,
            ModuleError::NotApproved { .. } | ModuleError::Forbidden => {
                conex_proto::ErrorCode::Forbidden
            }
            ModuleError::Conflict { .. } => conex_proto::ErrorCode::Conflict,
            ModuleError::StaleRevision(_) => conex_proto::ErrorCode::StaleRevision,
            ModuleError::QuotaExceeded(_) => conex_proto::ErrorCode::QuotaExceeded,
            ModuleError::PayloadTooLarge => conex_proto::ErrorCode::PayloadTooLarge,
            ModuleError::Internal(_) => conex_proto::ErrorCode::Internal,
        };
        CallError::new(code, self.to_string())
    }
}

pub struct ModuleRegistry {
    contracts: Arc<ContractRegistry>,
    approvals: Arc<ApprovalSet>,
    max_modules: usize,
    link_limits: RwLock<HashMap<u64, usize>>,
    nodes: RwLock<HashMap<String, NodeState>>,
}

impl ModuleRegistry {
    pub fn new(contracts: Arc<ContractRegistry>, approvals: Arc<ApprovalSet>) -> Arc<Self> {
        Arc::new(Self {
            contracts,
            approvals,
            max_modules: DEFAULT_MAX_MODULES,
            link_limits: RwLock::new(HashMap::new()),
            nodes: RwLock::new(HashMap::new()),
        })
    }

    pub(crate) fn configure_link(&self, generation: u64, max_modules: u32) {
        self.link_limits
            .write()
            .expect("module limits poisoned")
            .insert(generation, (max_modules as usize).min(self.max_modules));
    }

    pub fn contracts(&self) -> &Arc<ContractRegistry> {
        &self.contracts
    }

    /// 活动注册查找（T03）：按模块主键 `(spaceId, providerEndpointId)` 定位
    /// 当前发布。模块键全局唯一（装载期校验），至多一条记录。调用方先做
    /// 权限检查，再披露 registrationId 不匹配等过期情况（契约 §6.3）。
    pub fn active_record(
        &self,
        space_id: &str,
        provider_endpoint_id: &str,
    ) -> Option<ModuleRecord> {
        let nodes = self.nodes.read().expect("module registry poisoned");
        for state in nodes.values() {
            if let Some(record) = state.modules.get(&ModuleKey {
                space_id: space_id.to_string(),
                provider_endpoint_id: provider_endpoint_id.to_string(),
            }) {
                return Some(record.clone());
            }
        }
        None
    }

    pub fn approvals(&self) -> &Arc<ApprovalSet> {
        &self.approvals
    }

    /// 提供端声明的成员集：wire 冻结下即批准契约的完整成员集。
    pub fn handler_declaration(
        &self,
        contract_id: &str,
    ) -> Result<HandlerDeclaration, ModuleError> {
        let contract = self
            .contracts
            .get(contract_id)
            .ok_or_else(|| ModuleError::UnknownContract(contract_id.to_string()))?;
        Ok(HandlerDeclaration {
            methods: contract.method_names(),
            states: contract.state_names(),
        })
    }

    /// 原子发布一个已批准模块。首个成功注册（新代次）激活 Node 的新代次：
    /// 失效旧发布；同代次后续注册逐个追加。任何失败不改动旧代次状态。
    pub fn register(
        &self,
        node_id: &str,
        generation: u64,
        request: ModuleRequest,
        handlers: &HandlerDeclaration,
    ) -> Result<ModuleRecord, ModuleError> {
        if !conex_core::capability::is_valid_identifier(node_id) {
            return Err(ModuleError::BadRequest("invalid node id".into()));
        }
        for (what, value) in [
            ("space_id", &request.space_id),
            ("integration_id", &request.integration_id),
            ("provider_endpoint_id", &request.provider_endpoint_id),
            ("contract_id", &request.contract_id),
        ] {
            if !conex_core::capability::is_valid_identifier(value) {
                return Err(ModuleError::BadRequest(format!("invalid {what}")));
            }
        }
        // 完整批准：精确元组，未声明即拒绝（默认 deny）。
        if self
            .approvals
            .publish(
                node_id,
                &request.space_id,
                &request.integration_id,
                &request.provider_endpoint_id,
                &request.contract_id,
            )
            .is_none()
        {
            return Err(ModuleError::NotApproved {
                space: request.space_id.clone(),
                endpoint: request.provider_endpoint_id.clone(),
                node: node_id.to_string(),
            });
        }
        let contract = self
            .contracts
            .get(&request.contract_id)
            .ok_or_else(|| ModuleError::UnknownContract(request.contract_id.clone()))?;
        // handler 声明必须覆盖批准契约的全部成员（§6.1 提供端确认）。
        for method in &contract.methods {
            if !handlers.methods.iter().any(|name| name == &method.name) {
                return Err(ModuleError::HandlerMissing(format!(
                    "method {}",
                    method.name
                )));
            }
        }
        for state in &contract.states {
            if !handlers.states.iter().any(|name| name == &state.name) {
                return Err(ModuleError::HandlerMissing(format!("state {}", state.name)));
            }
        }
        let key = ModuleKey {
            space_id: request.space_id.clone(),
            provider_endpoint_id: request.provider_endpoint_id.clone(),
        };
        let record = ModuleRecord {
            registration_id: new_registration_id(),
            node_id: node_id.to_string(),
            space_id: request.space_id.clone(),
            integration_id: request.integration_id.clone(),
            provider_endpoint_id: request.provider_endpoint_id.clone(),
            contract_id: request.contract_id.clone(),
            generation,
        };
        let max_modules = self
            .link_limits
            .read()
            .expect("module limits poisoned")
            .get(&generation)
            .copied()
            .unwrap_or(self.max_modules);
        let mut nodes = self.nodes.write().expect("module registry poisoned");
        // 代次 fence：旧代次迟到注册先拒绝；绝不覆盖或修改新代次状态。
        if nodes
            .get(node_id)
            .is_some_and(|state| state.generation > generation)
        {
            return Err(ModuleError::StaleRevision(
                "late registration from an older connection generation".into(),
            ));
        }
        // 模块键不能被另一 Node 持有：替换只发生在同一 Node 的代次激活内。
        if let Some((space, endpoint)) = self.key_owned_by_other(&nodes, node_id, &key) {
            return Err(ModuleError::Conflict { space, endpoint });
        }
        match nodes.get(node_id) {
            // 活动代次后续模块：逐个注册；重复键冲突。
            Some(state) if state.generation == generation => {
                if state.modules.contains_key(&key) {
                    return Err(ModuleError::Conflict {
                        space: request.space_id.clone(),
                        endpoint: request.provider_endpoint_id.clone(),
                    });
                }
                if state.modules.len() >= max_modules {
                    return Err(ModuleError::QuotaExceeded(node_id.to_string()));
                }
                let state = nodes.get_mut(node_id).expect("present");
                state.modules.insert(key, record.clone());
                Ok(record)
            }
            // 新代次（或首注册）：原子激活，失效/替换该 Node 的旧发布。
            Some(_) | None => {
                if max_modules == 0 {
                    return Err(ModuleError::QuotaExceeded(node_id.to_string()));
                }
                nodes.insert(
                    node_id.to_string(),
                    NodeState {
                        generation,
                        modules: HashMap::from([(key, record.clone())]),
                    },
                );
                Ok(record)
            }
        }
    }

    /// 撤销当前连接拥有的精确注册。只有代次等于当前活动代次的拥有者可以
    /// 撤销；旧代次或旧 registrationId → stale_revision，不删除新注册；
    /// 无权主体 → forbidden，不泄漏注册是否存在。
    pub fn unregister(
        &self,
        node_id: &str,
        generation: u64,
        provider_endpoint_id: &str,
        registration_id: &str,
    ) -> Result<bool, ModuleError> {
        if !conex_proto::wire::is_ulid(registration_id) {
            return Err(ModuleError::BadRequest(
                "registrationId must be a ULID".into(),
            ));
        }
        let mut nodes = self.nodes.write().expect("module registry poisoned");
        let Some(state) = nodes.get(node_id) else {
            return Err(ModuleError::Forbidden);
        };
        // 代次 fence：旧连接的撤销绝不能删除新代次的注册；未激活的新连接
        // 也不是当前 owner。
        if state.generation != generation {
            return Err(ModuleError::StaleRevision(
                "registration belongs to a different link generation".into(),
            ));
        }
        let owned = state
            .modules
            .values()
            .find(|record| record.provider_endpoint_id == provider_endpoint_id);
        match owned {
            // 无权/未知：不泄漏。
            None => Err(ModuleError::Forbidden),
            Some(record) if record.registration_id != registration_id => Err(
                ModuleError::StaleRevision("registrationId is not the current registration".into()),
            ),
            Some(_) => {
                let state = nodes.get_mut(node_id).expect("present");
                state.modules.retain(|_, record| {
                    record.registration_id != registration_id
                        || record.provider_endpoint_id != provider_endpoint_id
                });
                if state.modules.is_empty() {
                    nodes.remove(node_id);
                }
                Ok(true)
            }
        }
    }

    /// 断线清理（代次 fence）：只有当前活动代次的清理才删除 Node 状态；
    /// 旧代次的迟到 cleanup 对新注册无效果。
    pub fn disconnect(&self, node_id: &str, generation: u64) {
        self.link_limits
            .write()
            .expect("module limits poisoned")
            .remove(&generation);
        let mut nodes = self.nodes.write().expect("module registry poisoned");
        if nodes
            .get(node_id)
            .is_some_and(|state| state.generation == generation)
        {
            nodes.remove(node_id);
        }
    }

    /// 发现：只返回活动且调用者有消费批准的模块；methods/states 按调用者
    /// 授权投影，绝不越过契约展开。编码结果超过 `max_frame_bytes` 时显式
    /// 返回 payload_too_large，不静默截断（§6.3）。
    pub fn list(
        &self,
        principal_id: &str,
        space_id: &str,
        max_frame_bytes: usize,
    ) -> Result<Value, ModuleError> {
        let nodes = self.nodes.read().expect("module registry poisoned");
        let mut modules: Vec<Value> = Vec::new();
        for state in nodes.values() {
            for record in state.modules.values() {
                if record.space_id != space_id {
                    continue;
                }
                let Some(contract) = self.contracts.get(&record.contract_id) else {
                    continue;
                };
                let Some((methods, states)) = self.approvals.visible(
                    principal_id,
                    space_id,
                    &record.provider_endpoint_id,
                    &record.contract_id,
                    &contract,
                ) else {
                    continue;
                };
                modules.push(json!({
                    "providerEndpointId": record.provider_endpoint_id,
                    "registrationId": record.registration_id,
                    "spaceId": record.space_id,
                    "integrationId": record.integration_id,
                    "contractId": record.contract_id,
                    "methods": methods,
                    "states": states,
                }));
            }
        }
        modules.sort_by(|a, b| {
            let left = (
                a.get("providerEndpointId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                a.get("registrationId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            let right = (
                b.get("providerEndpointId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                b.get("registrationId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            left.cmp(&right)
        });
        let value = json!({ "modules": modules });
        let encoded = serde_json::to_vec(&value).map_err(|error| {
            ModuleError::Internal(format!("cannot encode module list: {error}"))
        })?;
        if encoded.len() > max_frame_bytes {
            return Err(ModuleError::PayloadTooLarge);
        }
        Ok(value)
    }

    fn key_owned_by_other(
        &self,
        nodes: &HashMap<String, NodeState>,
        node_id: &str,
        key: &ModuleKey,
    ) -> Option<(String, String)> {
        for state in nodes.values() {
            if let Some(record) = state.modules.get(key)
                && record.node_id != node_id
            {
                return Some((record.space_id.clone(), record.provider_endpoint_id.clone()));
            }
        }
        None
    }
}

/// Crockford base32 ULID（48 位毫秒时间戳 + 80 位随机，首字符 ≤ '7'）。
/// T03 调用路由用它生成每跳转发 ID 与控制帧 ID（契约 §2 每跳 ULID）。
pub(crate) fn new_registration_id() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    use std::time::{SystemTime, UNIX_EPOCH};

    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
        & ((1 << 48) - 1);
    let mut out = [b'0'; 26];
    let mut value = millis;
    for slot in (0..10).rev() {
        out[slot] = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    let mut random = [0u8; 16];
    SystemRandom::new()
        .fill(&mut random)
        .expect("system random source unavailable");
    let mut value = u128::from_be_bytes(random) & ((1u128 << 80) - 1);
    for slot in (10..26).rev() {
        out[slot] = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("ULID is ASCII")
}
