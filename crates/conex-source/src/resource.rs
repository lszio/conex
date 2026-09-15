//! ResourceId normalization and claim extraction shared by source providers.
use conex_core::policy::{is_valid_resource, resource_within};
use conex_core::{CallError, CallResult, ResourceClaim};
use conex_proto::v1;

/// Validate without URL-decoding or Unicode normalization; reject traversal.
pub fn normalize_resource(raw: &str) -> CallResult<String> {
    if !is_valid_resource(raw) {
        return Err(CallError::new(
            v1::ErrorCode::BadRequest,
            "invalid resourceId",
        ));
    }
    Ok(raw.to_string())
}

/// Exact-resource claim for source/read.
pub fn read_claim(resource: &str) -> CallResult<ResourceClaim> {
    let resource_id = normalize_resource(resource)?;
    if resource_id.is_empty() {
        return Err(CallError::new(
            v1::ErrorCode::BadRequest,
            "resourceId must not be empty",
        ));
    }
    Ok(ResourceClaim {
        resource_id,
        action: "read".into(),
        subtree: false,
    })
}

/// Whole-subtree claim for source/list and source/search.
pub fn subtree_claim(root: &str, action: &str) -> CallResult<ResourceClaim> {
    let resource_id = normalize_resource(root)?;
    Ok(ResourceClaim {
        resource_id,
        action: action.into(),
        subtree: true,
    })
}

/// Segment-aware containment, delegating to the core policy rule.
pub fn within(root: &str, resource: &str) -> bool {
    resource_within(root, resource)
}
