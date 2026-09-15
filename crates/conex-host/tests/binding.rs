//! Bindings: 60s lifetime, principal binding, caps and caps, no renewal.
use std::collections::HashMap;

use conex_core::{Caller, Limits};
use conex_host::{BindingStore, PROFILE_ID};
use conex_proto::v1;

fn store() -> BindingStore {
    let mut capabilities = HashMap::new();
    capabilities.insert(
        "tenant-a".to_string(),
        vec!["source/list".to_string(), "source/read".to_string()],
    );
    BindingStore::new(PROFILE_ID, "host.example", capabilities, Limits::default())
}

fn alice() -> Caller {
    Caller {
        principal_id: "alice".into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "peer-1".into(),
    }
}

fn hello() -> v1::HelloRequest {
    v1::HelloRequest {
        profile_id: PROFILE_ID.into(),
        plane: v1::Plane::Broker as i32,
        provides: vec![],
        requires: vec!["source/read".into()],
    }
}

#[tokio::test(start_paused = true)]
async fn binding_cannot_outlive_sixty_seconds() {
    let store = store();
    let response = store.issue(&alice(), &hello()).unwrap();
    assert_eq!(response.expires_in_ms, 60_000);
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    assert!(store.validate(&alice(), &response.binding_id).is_err());
}

#[tokio::test(start_paused = true)]
async fn validate_never_renews() {
    let store = store();
    let response = store.issue(&alice(), &hello()).unwrap();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    let first = store.validate(&alice(), &response.binding_id).unwrap();
    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    let second = store.validate(&alice(), &response.binding_id).unwrap();
    assert_eq!(first.expires_at, second.expires_at);
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    assert!(store.validate(&alice(), &response.binding_id).is_err());
}

#[tokio::test]
async fn binding_is_bound_to_the_principal() {
    let store = store();
    let response = store.issue(&alice(), &hello()).unwrap();
    let bob = Caller {
        principal_id: "bob".into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "peer-2".into(),
    };
    assert!(store.validate(&bob, &response.binding_id).is_err());
}

#[tokio::test]
async fn unsupported_requires_is_rejected() {
    let store = store();
    let mut request = hello();
    request.requires = vec!["source/write".into()];
    assert_eq!(
        store.issue(&alice(), &request).unwrap_err().code_enum(),
        Some(v1::ErrorCode::UnsupportedCapability)
    );
}

#[tokio::test(start_paused = true)]
async fn per_principal_binding_cap_is_enforced() {
    let store = store();
    for _ in 0..64 {
        store.issue(&alice(), &hello()).unwrap();
    }
    assert_eq!(
        store.issue(&alice(), &hello()).unwrap_err().code_enum(),
        Some(v1::ErrorCode::QuotaExceeded)
    );
}
