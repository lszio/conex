//! S1 通用能力（docs/contracts/s1-capabilities.md §4-§6；docs/plans/
//! 2026-10-17-conex-s1.md T02）：模块注册表、静态批准装载与代次 fence。
//!
//! T02 只实现注册表与发现边界：`module/register`、`module/unregister`、
//! `module/list` 以及断线/迟到的代次清理。invoke/cancel（T03）与状态中继
//! （T04）明确 `unsupported_capability`，不静默降级。
#![forbid(unsafe_code)]

pub mod calls;
pub mod registry;
pub mod state;

pub use calls::*;
pub use registry::*;
pub use state::*;

use std::sync::Arc;

use conex_core::CallError;
use conex_core::capability::{
    ApprovalSet, CapabilityMethod, CapabilityState, ConsumeApproval, ContractDescription,
    ContractRegistry, ExecutionClass, PublishApproval, is_valid_identifier,
};
use conex_proto;

use crate::config::CapabilityConfig;

/// 契约 §4.2：新连接先 staged，首个成功 `module/register` 原子激活新代次并
/// 失效同一 Node 的旧发布。`from_config` 是中心侧的安装期入口：契约描述随
/// 软件安装，批准未声明即拒绝，旧 group/tenant/显示名不授予 Space 权限。
pub fn from_config(config: &CapabilityConfig) -> Result<Arc<ModuleRegistry>, CallError> {
    let contracts = Arc::new(ContractRegistry::new());
    for contract in &config.contracts {
        let description = ContractDescription {
            contract_id: contract.contract_id.clone(),
            methods: contract
                .methods
                .iter()
                .map(|method| {
                    let execution = ExecutionClass::parse(&method.execution).ok_or_else(|| {
                        invalid_config(format!(
                            "contract {} method {} has unsupported execution class '{}' (S1 accepts only read_only and non_replayable)",
                            contract.contract_id, method.name, method.execution
                        ))
                    })?;
                    Ok(CapabilityMethod {
                        name: method.name.clone(),
                        input_schema: method.input_schema.clone(),
                        output_schema: method.output_schema.clone(),
                        action: method.action.clone(),
                        execution,
                    })
                })
                .collect::<Result<Vec<_>, CallError>>()?,
            states: contract
                .states
                .iter()
                .map(|state| CapabilityState {
                    name: state.name.clone(),
                    value_schema: state.value_schema.clone(),
                    action: state.action.clone(),
                })
                .collect(),
        };
        contracts
            .register(description)
            .map_err(|error| invalid_config(error.to_string()))?;
    }
    let publish: Vec<PublishApproval> = config
        .publish
        .iter()
        .map(|approval| PublishApproval {
            node_id: approval.node_id.clone(),
            space_id: approval.space_id.clone(),
            integration_id: approval.integration_id.clone(),
            provider_endpoint_id: approval.provider_endpoint_id.clone(),
            contract_id: approval.contract_id.clone(),
        })
        .collect();
    let consume: Vec<ConsumeApproval> = config
        .consume
        .iter()
        .map(|approval| ConsumeApproval {
            principal_id: approval.principal_id.clone(),
            space_id: approval.space_id.clone(),
            provider_endpoint_id: approval.provider_endpoint_id.clone(),
            contract_id: approval.contract_id.clone(),
            methods: approval.methods.clone(),
            states: approval.states.clone(),
        })
        .collect();
    for approval in &publish {
        require_identifier("publish approval node_id", &approval.node_id)?;
        require_identifier("publish approval space_id", &approval.space_id)?;
        require_identifier("publish approval integration_id", &approval.integration_id)?;
        require_identifier(
            "publish approval provider_endpoint_id",
            &approval.provider_endpoint_id,
        )?;
        require_identifier("publish approval contract_id", &approval.contract_id)?;
    }
    for approval in &consume {
        require_identifier("consume approval principal_id", &approval.principal_id)?;
        require_identifier("consume approval space_id", &approval.space_id)?;
        require_identifier(
            "consume approval provider_endpoint_id",
            &approval.provider_endpoint_id,
        )?;
        require_identifier("consume approval contract_id", &approval.contract_id)?;
    }
    let approvals = ApprovalSet::new(publish, consume);
    approvals
        .validate_contracts(&contracts)
        .map_err(invalid_config)?;
    approvals.validate_keys().map_err(invalid_config)?;
    Ok(ModuleRegistry::new(contracts, Arc::new(approvals)))
}

fn require_identifier(what: &str, value: &str) -> Result<(), CallError> {
    if !is_valid_identifier(value) {
        return Err(invalid_config(format!(
            "{what} '{value}' is not a valid identifier"
        )));
    }
    Ok(())
}

fn invalid_config(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::Internal, message.into())
}
