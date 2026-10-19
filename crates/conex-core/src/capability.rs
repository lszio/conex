//! S1 预安装契约描述、校验入口与静态批准（docs/contracts/s1-capabilities.md
//! §4、§6；docs/plans/2026-10-17-conex-s1.md T02）。
//!
//! 本模块只有描述与校验逻辑，不包含任何领域实现：域实现（如 T07 的真实
//! 计数器节点）通过 [`ContractRegistry`] 接入预安装契约描述，核心不内置
//! 具体领域契约。执行类别只接受 `read_only` 与 `non_replayable`，其他类别
//! 明确 unsupported，不悄悄降级为 read_only。
//!
//! 静态批准的键是精确声明元组。旧 group/tenant/显示名/凭据引用都不是
//! Space 授权凭证；没有匹配的发布/消费批准项即拒绝（默认 deny）。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use conex_proto;
use serde_json::Value;

use crate::types::{CallError, CallResult};

/// S1 接受的执行类别（契约 §1：只接受 read_only 与 non_replayable）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionClass {
    ReadOnly,
    NonReplayable,
}

impl ExecutionClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionClass::ReadOnly => "read_only",
            ExecutionClass::NonReplayable => "non_replayable",
        }
    }

    /// 只接受这两个类别；其他执行类别显式拒绝，不允许降级为 read_only。
    pub fn parse(value: &str) -> Option<ExecutionClass> {
        match value {
            "read_only" => Some(ExecutionClass::ReadOnly),
            "non_replayable" => Some(ExecutionClass::NonReplayable),
            _ => None,
        }
    }
}

/// 契约可调用方法：typed 输入/输出、授权 action 与执行类别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMethod {
    pub name: String,
    /// 生成 JSON Schema 消息名（如 `conex.CounterAddInput`）。
    pub input_schema: String,
    /// 生成 JSON Schema 消息名（如 `conex.CounterResult`）。
    pub output_schema: String,
    /// 授权提取规则：消费该方法的 ResourceClaim.action。
    pub action: String,
    pub execution: ExecutionClass,
}

/// 契约状态源：sample.value 的 typed schema 与授权 action。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityState {
    pub name: String,
    /// 生成 JSON Schema 消息名（如 `conex.CounterStateValue`）。
    pub value_schema: String,
    /// 授权提取规则：订阅该状态的 ResourceClaim.action。
    pub action: String,
}

/// 预安装契约描述：methods/states、输入输出 schema、claim 与执行类别。
/// 描述本身不加载代码、不授予权限；发布批准与 handler 声明另行校验。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractDescription {
    pub contract_id: String,
    pub methods: Vec<CapabilityMethod>,
    pub states: Vec<CapabilityState>,
}

impl ContractDescription {
    /// 注册时校验描述完整性：标识合法、非空、成员名唯一。
    pub fn validate(&self) -> Result<(), String> {
        if !is_valid_identifier(&self.contract_id) {
            return Err(format!("invalid contract id {}", self.contract_id));
        }
        if self.methods.is_empty() && self.states.is_empty() {
            return Err(format!(
                "contract {} must declare at least one method or state",
                self.contract_id
            ));
        }
        let mut method_names = HashMap::new();
        for method in &self.methods {
            if !is_valid_method_name(&method.name) {
                return Err(format!(
                    "contract {} has invalid method name '{}'",
                    self.contract_id, method.name
                ));
            }
            if method.input_schema.is_empty() || method.output_schema.is_empty() {
                return Err(format!(
                    "contract {} method {} requires input/output schemas",
                    self.contract_id, method.name
                ));
            }
            if !is_valid_identifier(&method.action) {
                return Err(format!(
                    "contract {} method {} has invalid action {}",
                    self.contract_id, method.name, method.action
                ));
            }
            if method_names.insert(method.name.clone(), ()).is_some() {
                return Err(format!(
                    "contract {} declares duplicate method {}",
                    self.contract_id, method.name
                ));
            }
        }
        let mut state_names = HashMap::new();
        for state in &self.states {
            if !is_valid_method_name(&state.name) {
                return Err(format!(
                    "contract {} has invalid state name '{}'",
                    self.contract_id, state.name
                ));
            }
            if state.value_schema.is_empty() {
                return Err(format!(
                    "contract {} state {} requires a value schema",
                    self.contract_id, state.name
                ));
            }
            if !is_valid_identifier(&state.action) {
                return Err(format!(
                    "contract {} state {} has invalid action {}",
                    self.contract_id, state.name, state.action
                ));
            }
            if state_names.insert(state.name.clone(), ()).is_some() {
                return Err(format!(
                    "contract {} declares duplicate state {}",
                    self.contract_id, state.name
                ));
            }
        }
        Ok(())
    }

    pub fn method(&self, name: &str) -> Option<&CapabilityMethod> {
        self.methods.iter().find(|method| method.name == name)
    }

    pub fn state(&self, name: &str) -> Option<&CapabilityState> {
        self.states.iter().find(|state| state.name == name)
    }

    pub fn method_names(&self) -> Vec<String> {
        self.methods
            .iter()
            .map(|method| method.name.clone())
            .collect()
    }

    pub fn state_names(&self) -> Vec<String> {
        self.states.iter().map(|state| state.name.clone()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("duplicate contract: {0}")]
    Duplicate(String),
    #[error("invalid contract description: {0}")]
    Invalid(String),
    #[error("unknown contract: {0}")]
    Unknown(String),
}

/// 预安装契约注册表：域实现通过它接入契约描述；核心不内置具体领域。
#[derive(Debug, Default)]
pub struct ContractRegistry {
    inner: RwLock<HashMap<String, Arc<ContractDescription>>>,
}

impl ContractRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个预安装契约描述；重名拒绝（不覆盖），描述非法拒绝。
    pub fn register(&self, description: ContractDescription) -> Result<(), ContractError> {
        description.validate().map_err(ContractError::Invalid)?;
        let mut inner = self.inner.write().expect("contract registry poisoned");
        if inner.contains_key(&description.contract_id) {
            return Err(ContractError::Duplicate(description.contract_id));
        }
        inner.insert(description.contract_id.clone(), Arc::new(description));
        Ok(())
    }

    pub fn get(&self, contract_id: &str) -> Option<Arc<ContractDescription>> {
        self.inner
            .read()
            .expect("contract registry poisoned")
            .get(contract_id)
            .cloned()
    }

    pub fn installed(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .inner
            .read()
            .expect("contract registry poisoned")
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }
}

/// 标识合法性：非空，字母数字与 `-` `_` `.`（契约 §2 区分大小写，
/// S1 不依赖显示名/组名/凭据引用作为授权维度）。
pub fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// 方法与状态名：契约 wire 名（如 `counter/read`），额外允许 `/`。
pub fn is_valid_method_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
}

// ---- 校验入口 ------------------------------------------------------------

fn schema_error(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, message)
}

fn unknown_member(kind: &str, contract: &str, name: &str) -> CallError {
    CallError::new(
        conex_proto::ErrorCode::UnknownMethod,
        format!("contract {contract} has no {kind} {name}"),
    )
}

/// 校验契约方法的 typed 输入（T03 invoke 与测试共用同一入口）。
pub fn validate_method_input(
    contract: &ContractDescription,
    method: &str,
    input: &Value,
) -> CallResult<()> {
    let member = contract
        .method(method)
        .ok_or_else(|| unknown_member("method", &contract.contract_id, method))?;
    conex_proto::capability::validate_domain_schema(&member.input_schema, input)
        .map_err(|error| schema_error(error.message))?;
    Ok(())
}

/// 校验契约方法的 typed 输出；失败不得被当作"未执行"（non_replayable 由
/// T03 按 outcome_unknown 结算）。
pub fn validate_method_output(
    contract: &ContractDescription,
    method: &str,
    output: &Value,
) -> CallResult<()> {
    let member = contract
        .method(method)
        .ok_or_else(|| unknown_member("method", &contract.contract_id, method))?;
    conex_proto::capability::validate_domain_schema(&member.output_schema, output)
        .map_err(|error| schema_error(error.message))?;
    Ok(())
}

/// 校验状态 sample.value 的 typed schema。
pub fn validate_state_value(
    contract: &ContractDescription,
    state: &str,
    value: &Value,
) -> CallResult<()> {
    let member = contract
        .state(state)
        .ok_or_else(|| unknown_member("state", &contract.contract_id, state))?;
    conex_proto::capability::validate_domain_schema(&member.value_schema, value)
        .map_err(|error| schema_error(error.message))?;
    Ok(())
}

// ---- 静态批准（默认 deny）------------------------------------------------

/// Node 可发布的精确 `(spaceId, integrationId, providerEndpointId,
/// contractId)` 元组（契约 §4.1-2）。注册参数不能创建 Space/接入/节点或
/// 额外权限；未声明即拒绝。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishApproval {
    pub node_id: String,
    pub space_id: String,
    pub integration_id: String,
    pub provider_endpoint_id: String,
    pub contract_id: String,
}

/// Principal 可发现/调用/订阅的目标与契约成员（契约 §4.1-3）。
/// `methods`/`states` 缺省表示整个契约；显式列表必须是契约成员。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeApproval {
    pub principal_id: String,
    pub space_id: String,
    pub provider_endpoint_id: String,
    pub contract_id: String,
    pub methods: Option<Vec<String>>,
    pub states: Option<Vec<String>>,
}

/// 静态批准表：发布批准 + 消费批准。查找全部是精确匹配，其他维度（旧
/// group、tenant、显示名、endpoint 名、凭据引用）不授予任何 Space 权限。
#[derive(Debug, Clone, Default)]
pub struct ApprovalSet {
    publish: Vec<PublishApproval>,
    consume: Vec<ConsumeApproval>,
}

impl ApprovalSet {
    pub fn new(publish: Vec<PublishApproval>, consume: Vec<ConsumeApproval>) -> Self {
        Self { publish, consume }
    }

    /// 精确发布批准。任何维度不匹配即 `None`（默认 deny）。
    pub fn publish(
        &self,
        node_id: &str,
        space_id: &str,
        integration_id: &str,
        provider_endpoint_id: &str,
        contract_id: &str,
    ) -> Option<&PublishApproval> {
        self.publish.iter().find(|approval| {
            approval.node_id == node_id
                && approval.space_id == space_id
                && approval.integration_id == integration_id
                && approval.provider_endpoint_id == provider_endpoint_id
                && approval.contract_id == contract_id
        })
    }

    pub fn consume(
        &self,
        principal_id: &str,
        space_id: &str,
        provider_endpoint_id: &str,
        contract_id: &str,
    ) -> Option<&ConsumeApproval> {
        self.consume.iter().find(|approval| {
            approval.principal_id == principal_id
                && approval.space_id == space_id
                && approval.provider_endpoint_id == provider_endpoint_id
                && approval.contract_id == contract_id
        })
    }

    /// module/list 可见性投影：只有消费批准内的 methods/states 可见
    /// （发现不是授权）。无批准返回 `None`（模块对调用者不可见）。
    pub fn visible(
        &self,
        principal_id: &str,
        space_id: &str,
        provider_endpoint_id: &str,
        contract_id: &str,
        description: &ContractDescription,
    ) -> Option<(Vec<String>, Vec<String>)> {
        let approval = self.consume(principal_id, space_id, provider_endpoint_id, contract_id)?;
        let methods = match &approval.methods {
            Some(declared) => {
                let mut list: Vec<String> = declared
                    .iter()
                    .filter(|name| description.method(name).is_some())
                    .cloned()
                    .collect();
                list.sort();
                list.dedup();
                list
            }
            None => description.method_names(),
        };
        let states = match &approval.states {
            Some(declared) => {
                let mut list: Vec<String> = declared
                    .iter()
                    .filter(|name| description.state(name).is_some())
                    .cloned()
                    .collect();
                list.sort();
                list.dedup();
                list
            }
            None => description.state_names(),
        };
        Some((methods, states))
    }

    /// 装配期校验：所有批准引用的契约必须已安装；消费批准的显式成员必须
    /// 是该契约的成员。静态批准一旦装载即拒绝缺契约/未知成员的配置。
    pub fn validate_contracts(&self, contracts: &ContractRegistry) -> Result<(), String> {
        for approval in &self.publish {
            if contracts.get(&approval.contract_id).is_none() {
                return Err(format!(
                    "publish approval for node {} references contract {} that is not installed",
                    approval.node_id, approval.contract_id
                ));
            }
        }
        for approval in &self.consume {
            if let Some(description) = contracts.get(&approval.contract_id) {
                if let Some(methods) = &approval.methods {
                    for name in methods {
                        if description.method(name).is_none() {
                            return Err(format!(
                                "consume approval for {} references unknown contract method {}",
                                approval.principal_id, name
                            ));
                        }
                    }
                }
                if let Some(states) = &approval.states {
                    for name in states {
                        if description.state(name).is_none() {
                            return Err(format!(
                                "consume approval for {} references unknown contract state {}",
                                approval.principal_id, name
                            ));
                        }
                    }
                }
            } else {
                return Err(format!(
                    "consume approval for {} references contract {} that is not installed",
                    approval.principal_id, approval.contract_id
                ));
            }
        }
        Ok(())
    }

    /// 装配期校验：`(spaceId, providerEndpointId)` 模块主键全局唯一；
    /// 同一 (node, space, endpoint) 发布批准不重复。
    pub fn validate_keys(&self) -> Result<(), String> {
        let mut module_keys = HashMap::new();
        let mut publish_keys = HashMap::new();
        for approval in &self.publish {
            let key = (
                approval.space_id.clone(),
                approval.provider_endpoint_id.clone(),
            );
            if let Some(previous) = module_keys.insert(key.clone(), approval.node_id.clone()) {
                return Err(format!(
                    "module key {:?} is approved for both {} and {}",
                    key, previous, approval.node_id
                ));
            }
            let publish_key = (
                approval.node_id.clone(),
                approval.space_id.clone(),
                approval.provider_endpoint_id.clone(),
            );
            if publish_keys.insert(publish_key, ()).is_some() {
                return Err(format!(
                    "duplicate publish approval for node {} endpoint {}",
                    approval.node_id, approval.provider_endpoint_id
                ));
            }
        }
        let mut consume_keys = HashMap::new();
        for approval in &self.consume {
            let key = (
                approval.principal_id.clone(),
                approval.space_id.clone(),
                approval.provider_endpoint_id.clone(),
            );
            if consume_keys
                .insert(key, approval.contract_id.clone())
                .is_some()
            {
                return Err(format!(
                    "duplicate consume approval for {} on endpoint {}",
                    approval.principal_id, approval.provider_endpoint_id
                ));
            }
        }
        Ok(())
    }
}
