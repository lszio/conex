//! Partial-result aggregation. Each target is invoked independently.
use serde_json::Value;

use crate::types::CallResult;

#[derive(Debug, Clone)]
pub struct TargetCall {
    pub endpoint_id: String,
    pub method: String,
    pub input: Value,
}

#[derive(Debug)]
pub struct ProviderResult {
    pub endpoint_id: String,
    pub result: CallResult<Value>,
}

#[derive(Debug)]
pub struct AggregateResult {
    pub provider_results: Vec<ProviderResult>,
}

impl AggregateResult {
    pub fn successes(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.provider_results
            .iter()
            .filter_map(|entry| match &entry.result {
                Ok(value) => Some((entry.endpoint_id.as_str(), value)),
                Err(_) => None,
            })
    }

    pub fn failures(&self) -> impl Iterator<Item = (&str, &crate::types::CallError)> {
        self.provider_results
            .iter()
            .filter_map(|entry| match &entry.result {
                Err(error) => Some((entry.endpoint_id.as_str(), error)),
                Ok(_) => None,
            })
    }
}
