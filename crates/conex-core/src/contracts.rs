//! P1 MethodContracts: blob/session/operation (stream ack/flow/reset are wire
//! frames, not method calls; their recovery lives in the WSS transport).
//!
//! Each entry pair (`prepare`, `validate_output`) is the authoritative strict
//! decoder + output check; downstream handlers receive the canonicalized input
//! from `PreparedInput` (design §5.2). Handlers themselves live next to the
//! `conex-content`, `conex-core::session` and `conex-core::operation` stores
//! to keep domain logic out of `conex-host`.
#![forbid(unsafe_code)]

use serde_json::{Map, Value, json};

use crate::types::{CallError, CallResult, MethodContract, PreparedInput};

use conex_proto::cid;

pub const BLOB_PUT: &str = "blob/put";
pub const BLOB_CHUNK: &str = "blob/chunk";
pub const BLOB_COMMIT: &str = "blob/commit";
pub const BLOB_PIN: &str = "blob/pin";
pub const BLOB_UNPIN: &str = "blob/unpin";
pub const BLOB_HAVE: &str = "blob/have";
pub const BLOB_GET: &str = "blob/get";
pub const BLOB_CANCEL: &str = "blob/cancel";

pub const SESSION_OPEN: &str = "session/open";
pub const SESSION_RESUME: &str = "session/resume";
pub const SESSION_RENEW: &str = "session/renew";
pub const SESSION_CLOSE: &str = "session/close";

pub const OPERATION_GET: &str = "operation/get";
pub const OPERATION_CANCEL: &str = "operation/cancel";

const ADAPTER_ID: &str = "conex-broker"; // every P1 method is owned by the broker adapter.

fn bad(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::v1::ErrorCode::BadRequest, message)
}

fn unsupported(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::v1::ErrorCode::UnsupportedCapability, message)
}

fn object(input: &Value) -> CallResult<&Map<String, Value>> {
    input
        .as_object()
        .ok_or_else(|| bad("input must be an object"))
}

fn required_string<'a>(map: &'a Map<String, Value>, key: &str) -> CallResult<&'a str> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("{key} must be a string")))
}

fn required_u64(map: &Map<String, Value>, key: &str) -> CallResult<u64> {
    let raw = map
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("{key} must be a decimal string")))?;
    raw.parse::<u64>()
        .map_err(|_| bad(format!("{key} is not a valid decimal")))
}

fn required_u32(map: &Map<String, Value>, key: &str) -> CallResult<u32> {
    let raw = map
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("{key} must be a decimal string")))?;
    raw.parse::<u32>()
        .map_err(|_| bad(format!("{key} is not a valid u32")))
}

fn optional_string<'a>(map: &'a Map<String, Value>, key: &str) -> CallResult<Option<&'a str>> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(bad(format!("{key} must be a string"))),
    }
}

fn known_keys(map: &Map<String, Value>, allowed: &[&str]) -> CallResult<()> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(bad(format!("unknown input field {key}")));
        }
    }
    Ok(())
}

fn content_address(value: &Value) -> CallResult<String> {
    let map = object(value)?;
    known_keys(map, &["rawCid", "manifestCid"])?;
    if let Some(raw) = map.get("rawCid") {
        let s = raw.as_str().ok_or_else(|| bad("rawCid must be a string"))?;
        cid::parse_cid(s).map_err(|error| bad(error.message))?;
        return Ok(s.to_string());
    }
    if let Some(manifest) = map.get("manifestCid") {
        let s = manifest
            .as_str()
            .ok_or_else(|| bad("manifestCid must be a string"))?;
        cid::parse_cid(s).map_err(|error| bad(error.message))?;
        return Ok(s.to_string());
    }
    Err(bad("contentAddress must carry rawCid or manifestCid"))
}

fn blob_access(value: &Value) -> CallResult<conex_proto::v1::BlobAccess> {
    let map = object(value)?;
    known_keys(map, &["providerId", "plane", "spaceId", "resourceId"])?;
    let provider_id = required_string(map, "providerId")?.to_string();
    let plane = required_string(map, "plane")?.to_string();
    let space_id = optional_string(map, "spaceId")?.map(str::to_string);
    let resource_id = required_string(map, "resourceId")?.to_string();
    Ok(conex_proto::v1::BlobAccess {
        provider_id,
        plane,
        space_id,
        resource_id,
    })
}

fn parse_recovery(value: &str) -> CallResult<&'static str> {
    match value {
        "none" => Ok("none"),
        "in_process" => Ok("in_process"),
        "persistent" => Ok("persistent"),
        other => Err(unsupported(format!(
            "recovery level {other} is not recognised"
        ))),
    }
}

fn not_negative_u32(map: &Map<String, Value>, key: &str) -> CallResult<u32> {
    let raw = map
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("{key} must be a decimal string")))?;
    raw.parse::<u32>()
        .map_err(|_| bad(format!("{key} is not a valid u32")))
}

fn dedup_key(value: &Value) -> CallResult<conex_proto::v1::DedupKey> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "tenantId",
            "principalId",
            "providerEndpointId",
            "spaceId",
            "resourceId",
            "method",
            "operationId",
        ],
    )?;
    let tenant_id = required_string(map, "tenantId")?.to_string();
    let principal_id = required_string(map, "principalId")?.to_string();
    let provider_endpoint_id = required_string(map, "providerEndpointId")?.to_string();
    let space_id = optional_string(map, "spaceId")?.map(str::to_string);
    let resource_id = required_string(map, "resourceId")?.to_string();
    let method = required_string(map, "method")?.to_string();
    let operation_id = required_string(map, "operationId")?.to_string();
    if operation_id.is_empty() {
        return Err(bad("operationId must not be empty"));
    }
    if method.is_empty() {
        return Err(bad("method must not be empty"));
    }
    Ok(conex_proto::v1::DedupKey {
        tenant_id,
        principal_id,
        provider_endpoint_id,
        space_id,
        resource_id,
        method,
        operation_id,
    })
}

fn stream_cursors(value: &Value) -> CallResult<Vec<conex_proto::v1::StreamCursor>> {
    let array = value
        .as_array()
        .ok_or_else(|| bad("streams must be an array"))?;
    let mut cursors = Vec::with_capacity(array.len());
    for entry in array {
        let map = object(entry)?;
        known_keys(map, &["streamId", "lastReceivedSeq", "consumedBytes"])?;
        let stream_id = required_string(map, "streamId")?.to_string();
        let last_received_seq = required_u64(map, "lastReceivedSeq")?;
        let consumed_bytes = required_u64(map, "consumedBytes")?;
        cursors.push(conex_proto::v1::StreamCursor {
            stream_id,
            last_received_seq,
            consumed_bytes,
        });
    }
    Ok(cursors)
}

fn session_binding(value: &Value) -> CallResult<conex_proto::v1::SessionBinding> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "principalId",
            "tenantId",
            "providerEndpointId",
            "plane",
            "workspacePeerId",
            "humanPeerId",
        ],
    )?;
    let principal_id = required_string(map, "principalId")?.to_string();
    let tenant_id = required_string(map, "tenantId")?.to_string();
    let provider_endpoint_id = required_string(map, "providerEndpointId")?.to_string();
    let plane = map
        .get("plane")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("plane must be an integer"))? as i32;
    let workspace_peer_id = optional_string(map, "workspacePeerId")?.map(str::to_string);
    let human_peer_id = optional_string(map, "humanPeerId")?.map(str::to_string);
    if principal_id.is_empty() || tenant_id.is_empty() || provider_endpoint_id.is_empty() {
        return Err(bad("binding fields must not be empty"));
    }
    Ok(conex_proto::v1::SessionBinding {
        principal_id,
        tenant_id,
        provider_endpoint_id,
        plane,
        workspace_peer_id,
        human_peer_id,
    })
}

fn stream_cursors_to_value(cursors: &[conex_proto::v1::StreamCursor]) -> Value {
    Value::Array(
        cursors
            .iter()
            .map(|cursor| {
                json!({
                    "streamId": cursor.stream_id,
                    "lastReceivedSeq": cursor.last_received_seq.to_string(),
                    "consumedBytes": cursor.consumed_bytes.to_string(),
                })
            })
            .collect(),
    )
}

pub fn prepare_blob_put(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(
        map,
        &[
            "formatVersion",
            "declaredSizeBytes",
            "declaredChunkSize",
            "expectedRoot",
            "access",
            "requestedLeaseMs",
        ],
    )?;
    let format_version = not_negative_u32(map, "formatVersion")?;
    if format_version != 1 {
        return Err(unsupported(format!(
            "formatVersion {format_version} is not supported"
        )));
    }
    let declared_size_bytes = required_u64(map, "declaredSizeBytes")?;
    let declared_chunk_size = required_u32(map, "declaredChunkSize")?;
    if declared_chunk_size == 0 {
        return Err(bad("declaredChunkSize must be > 0"));
    }
    let expected_root = map
        .get("expectedRoot")
        .ok_or_else(|| bad("expectedRoot is required"))?;
    let expected_root_cid = content_address(expected_root)?;
    let access = blob_access(map.get("access").ok_or_else(|| bad("access is required"))?)?;
    let requested_lease_ms = map
        .get("requestedLeaseMs")
        .and_then(Value::as_str)
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| bad("requestedLeaseMs is not a valid decimal"))
        })
        .transpose()?;
    let canonical = json!({
        "formatVersion": format_version.to_string(),
        "declaredSizeBytes": declared_size_bytes.to_string(),
        "declaredChunkSize": declared_chunk_size.to_string(),
        "expectedRootCid": expected_root_cid,
        "access": {
            "providerId": access.provider_id,
            "plane": access.plane,
            "spaceId": access.space_id,
            "resourceId": access.resource_id,
        },
        "requestedLeaseMs": requested_lease_ms.map(|n| n.to_string()),
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: access.resource_id.clone(),
            action: "blob.upload".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant {
            principal_id: String::new(),
            tenant_id: String::new(),
        },
        // filled by dispatcher from caller:
    })
}

pub fn prepare_blob_chunk(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["uploadId", "chunkIndex", "chunkCid", "chunkBytes"])?;
    let upload_id = required_string(map, "uploadId")?.to_string();
    if upload_id.is_empty() {
        return Err(bad("uploadId must not be empty"));
    }
    let chunk_index = required_u32(map, "chunkIndex")?;
    let chunk_cid = required_string(map, "chunkCid")?.to_string();
    cid::parse_cid(&chunk_cid).map_err(|error| bad(error.message))?;
    // chunkBytes is bytes on the wire; we accept it as base64 in JSON.
    let chunk_bytes = match map.get("chunkBytes") {
        Some(Value::String(b64)) => base64_decode(b64)?,
        Some(_) => return Err(bad("chunkBytes must be a base64 string")),
        None => return Err(bad("chunkBytes is required")),
    };
    if chunk_bytes.len() > 1024 * 1024 {
        return Err(bad("chunkBytes exceeds 1 MiB"));
    }
    let canonical = json!({
        "uploadId": upload_id,
        "chunkIndex": chunk_index.to_string(),
        "chunkCid": chunk_cid,
        "chunkBytes": chunk_bytes,
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: upload_id,
            action: "blob.upload".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_blob_commit(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(
        map,
        &["uploadId", "declaredRoot", "pinUntilMs", "persistence"],
    )?;
    let upload_id = required_string(map, "uploadId")?.to_string();
    let declared_root = content_address(
        map.get("declaredRoot")
            .ok_or_else(|| bad("declaredRoot is required"))?,
    )?;
    let persistence = required_string(map, "persistence")?.to_string();
    if persistence != "local" {
        return Err(unsupported(format!(
            "persistence {persistence} is not supported by the broker"
        )));
    }
    let pin_until_ms = map
        .get("pinUntilMs")
        .and_then(Value::as_str)
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| bad("pinUntilMs is not a valid decimal"))
        })
        .transpose()?;
    let canonical = json!({
        "uploadId": upload_id,
        "declaredRootCid": declared_root,
        "persistence": persistence,
        "pinUntilMs": pin_until_ms.map(|n| n.to_string()),
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: upload_id,
            action: "blob.upload".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_blob_pin(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["root", "pinUntilMs", "spaceId"])?;
    let root = content_address(map.get("root").ok_or_else(|| bad("root is required"))?)?;
    let pin_until_ms = map
        .get("pinUntilMs")
        .and_then(Value::as_str)
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| bad("pinUntilMs is not a valid decimal"))
        })
        .transpose()?;
    let space_id = optional_string(map, "spaceId")?.map(str::to_string);
    let canonical = json!({
        "rootCid": root,
        "pinUntilMs": pin_until_ms.map(|n| n.to_string()),
        "spaceId": space_id,
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: root,
            action: "blob.pin".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_blob_unpin(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["pinId"])?;
    let pin_id = required_string(map, "pinId")?.to_string();
    if pin_id.is_empty() {
        return Err(bad("pinId must not be empty"));
    }
    let canonical = json!({"pinId": pin_id});
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: pin_id,
            action: "blob.unpin".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_blob_have(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["chunkCids"])?;
    let array = map
        .get("chunkCids")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("chunkCids must be an array"))?;
    if array.is_empty() {
        return Err(bad("chunkCids must not be empty"));
    }
    let mut cids = Vec::with_capacity(array.len());
    for entry in array {
        let s = entry
            .as_str()
            .ok_or_else(|| bad("chunkCids entries must be strings"))?;
        cid::parse_cid(s).map_err(|error| bad(error.message))?;
        cids.push(s.to_string());
    }
    let canonical = json!({"chunkCids": cids});
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: cids.first().cloned().unwrap_or_default(),
            action: "blob.have".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_blob_get(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["chunkCid", "rangeOffset", "rangeLength"])?;
    let chunk_cid = required_string(map, "chunkCid")?.to_string();
    cid::parse_cid(&chunk_cid).map_err(|error| bad(error.message))?;
    let range_offset = map
        .get("rangeOffset")
        .and_then(Value::as_str)
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| bad("rangeOffset is not a valid decimal"))
        })
        .transpose()?;
    let range_length = map
        .get("rangeLength")
        .and_then(Value::as_str)
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| bad("rangeLength is not a valid decimal"))
        })
        .transpose()?;
    let canonical = json!({
        "chunkCid": chunk_cid,
        "rangeOffset": range_offset.map(|n| n.to_string()),
        "rangeLength": range_length.map(|n| n.to_string()),
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: chunk_cid.clone(),
            action: "blob.get".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_blob_cancel(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["uploadId", "reason"])?;
    let upload_id = required_string(map, "uploadId")?.to_string();
    let reason = optional_string(map, "reason")?.map(str::to_string);
    let canonical = json!({"uploadId": upload_id, "reason": reason});
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: upload_id,
            action: "blob.cancel".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_session_open(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(
        map,
        &[
            "requested",
            "recovery",
            "provides",
            "requires",
            "attachmentId",
            "peerRole",
        ],
    )?;
    let requested = session_binding(
        map.get("requested")
            .ok_or_else(|| bad("requested binding is required"))?,
    )?;
    let recovery_raw = required_string(map, "recovery")?.to_string();
    let recovery = parse_recovery(&recovery_raw)?.to_string();
    let provides = map
        .get("provides")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|entry| entry.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad("provides must be an array of strings"))
        })
        .transpose()?
        .unwrap_or_default();
    let requires = map
        .get("requires")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|entry| entry.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad("requires must be an array of strings"))
        })
        .transpose()?
        .unwrap_or_default();
    let attachment_id = required_string(map, "attachmentId")?.to_string();
    let peer_role = required_string(map, "peerRole")?.to_string();
    if attachment_id.is_empty() || peer_role.is_empty() {
        return Err(bad("attachmentId and peerRole must not be empty"));
    }
    let canonical = json!({
        "binding": {
            "principalId": requested.principal_id,
            "tenantId": requested.tenant_id,
            "providerEndpointId": requested.provider_endpoint_id,
            "plane": requested.plane,
            "workspacePeerId": requested.workspace_peer_id,
            "humanPeerId": requested.human_peer_id,
        },
        "recovery": recovery,
        "provides": provides,
        "requires": requires,
        "attachmentId": attachment_id,
        "peerRole": peer_role,
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: attachment_id,
            action: "session.attach".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_session_resume(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(
        map,
        &[
            "sessionId",
            "attachmentId",
            "expectedEpoch",
            "streams",
            "binding",
        ],
    )?;
    let session_id = required_string(map, "sessionId")?.to_string();
    let attachment_id = required_string(map, "attachmentId")?.to_string();
    let expected_epoch = required_u64(map, "expectedEpoch")?;
    let cursors = map
        .get("streams")
        .map(stream_cursors_to_canonical)
        .transpose()?
        .unwrap_or_default();
    let binding = map.get("binding").cloned();
    let canonical = json!({
        "sessionId": session_id,
        "attachmentId": attachment_id,
        "expectedEpoch": expected_epoch.to_string(),
        "streams": cursors,
        "binding": binding,
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: session_id,
            action: "session.resume".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_session_renew(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(
        map,
        &["sessionId", "attachmentId", "expectedEpoch", "extendMs"],
    )?;
    let session_id = required_string(map, "sessionId")?.to_string();
    let attachment_id = required_string(map, "attachmentId")?.to_string();
    let expected_epoch = required_u64(map, "expectedEpoch")?;
    let extend_ms = required_u64(map, "extendMs")?;
    let canonical = json!({
        "sessionId": session_id,
        "attachmentId": attachment_id,
        "expectedEpoch": expected_epoch.to_string(),
        "extendMs": extend_ms.to_string(),
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: session_id,
            action: "session.renew".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_session_close(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(
        map,
        &["sessionId", "attachmentId", "expectedEpoch", "reason"],
    )?;
    let session_id = required_string(map, "sessionId")?.to_string();
    let attachment_id = required_string(map, "attachmentId")?.to_string();
    let expected_epoch = required_u64(map, "expectedEpoch")?;
    let reason = optional_string(map, "reason")?.map(str::to_string);
    let canonical = json!({
        "sessionId": session_id,
        "attachmentId": attachment_id,
        "expectedEpoch": expected_epoch.to_string(),
        "reason": reason,
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: session_id,
            action: "session.close".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_operation_get(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["key"])?;
    let key = dedup_key(map.get("key").ok_or_else(|| bad("key is required"))?)?;
    let canonical = json!({
        "key": {
            "tenantId": key.tenant_id,
            "principalId": key.principal_id,
            "providerEndpointId": key.provider_endpoint_id,
            "spaceId": key.space_id,
            "resourceId": key.resource_id,
            "method": key.method,
            "operationId": key.operation_id,
        }
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: key.resource_id.clone(),
            action: "operation.get".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

pub fn prepare_operation_cancel(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["key", "reason"])?;
    let key = dedup_key(map.get("key").ok_or_else(|| bad("key is required"))?)?;
    let reason = optional_string(map, "reason")?.map(str::to_string);
    let canonical = json!({
        "key": {
            "tenantId": key.tenant_id,
            "principalId": key.principal_id,
            "providerEndpointId": key.provider_endpoint_id,
            "spaceId": key.space_id,
            "resourceId": key.resource_id,
            "method": key.method,
            "operationId": key.operation_id,
        },
        "reason": reason,
    });
    Ok(PreparedInput {
        canonical,
        claim: crate::types::ResourceClaim {
            resource_id: key.resource_id.clone(),
            action: "operation.cancel".into(),
            subtree: false,
        },
        binding: crate::types::PrincipalTenant::default(),
    })
}

fn base64_decode(input: &str) -> CallResult<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| bad("chunkBytes is not valid base64"))
}

fn stream_cursors_to_canonical(value: &Value) -> CallResult<Value> {
    Ok(stream_cursors_to_value(&stream_cursors(value)?))
}

fn validate_blob_put_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "uploadId",
            "chunkSize",
            "leaseMs",
            "maxBlobBytes",
            "inlineThresholdBytes",
            "alreadyHaveChunkCids",
            "resourceId",
        ],
    )?;
    let _ = required_string(map, "uploadId")?;
    let _ = not_negative_u32(map, "chunkSize")?;
    let _ = required_u64(map, "leaseMs")?;
    let _ = required_u64(map, "maxBlobBytes")?;
    let _ = not_negative_u32(map, "inlineThresholdBytes")?;
    if let Some(value) = map.get("alreadyHaveChunkCids") {
        let _ = value
            .as_array()
            .ok_or_else(|| bad("alreadyHaveChunkCids must be an array"))?;
    }
    Ok(())
}

fn validate_blob_chunk_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["chunkIndex", "receivedBytes", "remainingBytes"])?;
    let _ = not_negative_u32(map, "chunkIndex")?;
    let _ = required_u64(map, "receivedBytes")?;
    let _ = required_u64(map, "remainingBytes")?;
    Ok(())
}

fn validate_blob_commit_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "resourceRootCid",
            "committedBytes",
            "persistence",
            "receiptId",
        ],
    )?;
    let _ = cid::parse_cid(required_string(map, "resourceRootCid")?)
        .map_err(|error| bad(error.message))?;
    let _ = required_u64(map, "committedBytes")?;
    let _ = required_string(map, "persistence")?;
    let _ = required_string(map, "receiptId")?;
    Ok(())
}

fn validate_blob_pin_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["pinId", "expiresAtMs"])?;
    let _ = required_string(map, "pinId")?;
    let _ = required_u64(map, "expiresAtMs")?;
    Ok(())
}

fn validate_blob_have_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["present"])?;
    let _ = map
        .get("present")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("present must be an array"))?;
    Ok(())
}

fn validate_blob_get_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["chunkBytes", "chunkCid"])?;
    let _ = required_string(map, "chunkCid")?;
    let _ = required_string(map, "chunkBytes")?;
    Ok(())
}

fn validate_session_open_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "sessionId",
            "binding",
            "recovery",
            "attachmentEpoch",
            "leaseMs",
            "provides",
            "rejectedCapabilities",
        ],
    )?;
    let _ = required_string(map, "sessionId")?;
    let _ = required_string(map, "recovery")?;
    let _ = required_u64(map, "attachmentEpoch")?;
    let _ = required_u64(map, "leaseMs")?;
    Ok(())
}

fn validate_session_resume_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "sessionId",
            "attachmentId",
            "newEpoch",
            "streamsReset",
            "restoredWindowBytes",
        ],
    )?;
    let _ = required_string(map, "sessionId")?;
    let _ = required_string(map, "attachmentId")?;
    let _ = required_u64(map, "newEpoch")?;
    let _ = required_u64(map, "restoredWindowBytes")?;
    Ok(())
}

fn validate_session_renew_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["newEpoch", "leaseMs"])?;
    let _ = required_u64(map, "newEpoch")?;
    let _ = required_u64(map, "leaseMs")?;
    Ok(())
}

fn validate_session_close_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["sessionId", "finalEpoch"])?;
    let _ = required_string(map, "sessionId")?;
    let _ = required_u64(map, "finalEpoch")?;
    Ok(())
}

fn validate_operation_get_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(
        map,
        &[
            "key",
            "state",
            "expiresAtMs",
            "success",
            "failure",
            "execution",
            "acceptedAtMs",
            "settledAtMs",
        ],
    )?;
    let _ = map
        .get("state")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("state must be an integer"))?;
    let _ = required_u64(map, "expiresAtMs")?;
    let _ = required_u64(map, "acceptedAtMs")?;
    Ok(())
}

fn validate_operation_cancel_output(value: &Value) -> CallResult<()> {
    let map = object(value)?;
    known_keys(map, &["state", "outcomeUnknown"])?;
    let _ = map
        .get("state")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("state must be an integer"))?;
    let _ = map
        .get("outcomeUnknown")
        .and_then(Value::as_bool)
        .ok_or_else(|| bad("outcomeUnknown must be a bool"))?;
    Ok(())
}

pub fn contracts() -> Vec<(&'static str, MethodContract)> {
    vec![
        (
            BLOB_PUT,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobPutRequest",
                output_schema: "conex.v1.BlobPutResponse",
                prepare: prepare_blob_put,
                validate_output: validate_blob_put_output,
            },
        ),
        (
            BLOB_CHUNK,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobChunkRequest",
                output_schema: "conex.v1.BlobChunkResponse",
                prepare: prepare_blob_chunk,
                validate_output: validate_blob_chunk_output,
            },
        ),
        (
            BLOB_COMMIT,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobCommitRequest",
                output_schema: "conex.v1.BlobCommitResponse",
                prepare: prepare_blob_commit,
                validate_output: validate_blob_commit_output,
            },
        ),
        (
            BLOB_PIN,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobPinRequest",
                output_schema: "conex.v1.BlobPinResponse",
                prepare: prepare_blob_pin,
                validate_output: validate_blob_pin_output,
            },
        ),
        (
            BLOB_UNPIN,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobUnpinRequest",
                output_schema: "conex.v1.BlobUnpinResponse",
                prepare: prepare_blob_unpin,
                validate_output: pass,
            },
        ),
        (
            BLOB_HAVE,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobHaveRequest",
                output_schema: "conex.v1.BlobHaveResponse",
                prepare: prepare_blob_have,
                validate_output: validate_blob_have_output,
            },
        ),
        (
            BLOB_GET,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobGetRequest",
                output_schema: "conex.v1.BlobGetResponse",
                prepare: prepare_blob_get,
                validate_output: validate_blob_get_output,
            },
        ),
        (
            BLOB_CANCEL,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.BlobCancelRequest",
                output_schema: "conex.v1.BlobCancelResponse",
                prepare: prepare_blob_cancel,
                validate_output: pass,
            },
        ),
        (
            SESSION_OPEN,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.SessionOpenRequest",
                output_schema: "conex.v1.SessionOpenResponse",
                prepare: prepare_session_open,
                validate_output: validate_session_open_output,
            },
        ),
        (
            SESSION_RESUME,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.SessionResumeRequest",
                output_schema: "conex.v1.SessionResumeResponse",
                prepare: prepare_session_resume,
                validate_output: validate_session_resume_output,
            },
        ),
        (
            SESSION_RENEW,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.SessionRenewRequest",
                output_schema: "conex.v1.SessionRenewResponse",
                prepare: prepare_session_renew,
                validate_output: validate_session_renew_output,
            },
        ),
        (
            SESSION_CLOSE,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.SessionCloseRequest",
                output_schema: "conex.v1.SessionCloseResponse",
                prepare: prepare_session_close,
                validate_output: validate_session_close_output,
            },
        ),
        (
            OPERATION_GET,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.OperationGetRequest",
                output_schema: "conex.v1.OperationGetResponse",
                prepare: prepare_operation_get,
                validate_output: validate_operation_get_output,
            },
        ),
        (
            OPERATION_CANCEL,
            MethodContract {
                adapter_id: ADAPTER_ID,
                input_schema: "conex.v1.OperationCancelRequest",
                output_schema: "conex.v1.OperationCancelResponse",
                prepare: prepare_operation_cancel,
                validate_output: validate_operation_cancel_output,
            },
        ),
    ]
}

fn pass(_value: &Value) -> CallResult<()> {
    Ok(())
}

// Default::default is derived for PreparedInput/PrincipalTenant/ResourceClaim.
