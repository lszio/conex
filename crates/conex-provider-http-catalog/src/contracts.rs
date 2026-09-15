//! Source-wide contracts: reuse shared validation but claim the whole partition.
use conex_core::{CallResult, MethodContract, PreparedInput, ResourceClaim};
use conex_source::contracts::{self as shared, SOURCE_LIST, SOURCE_READ, SOURCE_SEARCH};
use serde_json::Value;

/// Declared in the capability description: every method needs partition scope.
pub const SCOPE: &str = "source-wide";

pub fn contracts() -> Vec<(&'static str, MethodContract)> {
    shared::contracts()
        .into_iter()
        .map(|(method, base)| {
            let prepare: fn(&Value) -> CallResult<PreparedInput> = match method {
                SOURCE_LIST => prepare_list,
                SOURCE_READ => prepare_read,
                SOURCE_SEARCH => prepare_search,
                _ => base.prepare,
            };
            (
                method,
                MethodContract {
                    input_schema: base.input_schema,
                    output_schema: base.output_schema,
                    prepare,
                    validate_output: base.validate_output,
                },
            )
        })
        .collect()
}

pub fn source_wide(claim: ResourceClaim) -> ResourceClaim {
    ResourceClaim {
        resource_id: String::new(),
        action: claim.action,
        subtree: true,
    }
}

fn prepare_list(input: &Value) -> CallResult<PreparedInput> {
    let mut prepared = shared::prepare_list(input)?;
    prepared.claim = source_wide(prepared.claim);
    Ok(prepared)
}

fn prepare_read(input: &Value) -> CallResult<PreparedInput> {
    let mut prepared = shared::prepare_read(input)?;
    prepared.claim = source_wide(prepared.claim);
    Ok(prepared)
}

fn prepare_search(input: &Value) -> CallResult<PreparedInput> {
    let mut prepared = shared::prepare_search(input)?;
    prepared.claim = source_wide(prepared.claim);
    Ok(prepared)
}
