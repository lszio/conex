//! Identity keys are namespaced by source; sub alone never merges identities.
use conex_core::{IdentityBinding, IdentityKey, IdentityMap};
use conex_proto::v1;

fn oidc(issuer: &str, subject: &str, principal: &str, tenant: &str) -> IdentityBinding {
    IdentityBinding {
        key: IdentityKey::Oidc {
            issuer: issuer.into(),
            subject: subject.into(),
        },
        principal_id: principal.into(),
        tenant_id: tenant.into(),
    }
}

#[test]
fn same_subject_different_issuers_are_distinct() {
    let map = IdentityMap::new(vec![
        oidc("https://a.example", "sub-1", "principal-a", "tenant-a"),
        oidc("https://b.example", "sub-1", "principal-b", "tenant-b"),
    ]);
    let a = map
        .resolve(
            &IdentityKey::Oidc {
                issuer: "https://a.example".into(),
                subject: "sub-1".into(),
            },
            "peer-1",
        )
        .unwrap();
    let b = map
        .resolve(
            &IdentityKey::Oidc {
                issuer: "https://b.example".into(),
                subject: "sub-1".into(),
            },
            "peer-2",
        )
        .unwrap();
    assert_eq!(a.principal_id, "principal-a");
    assert_eq!(b.principal_id, "principal-b");
    assert_ne!(a.tenant_id, b.tenant_id);
}

#[test]
fn unknown_identity_has_no_default_principal() {
    let map = IdentityMap::new(vec![]);
    let err = map
        .resolve(
            &IdentityKey::Oidc {
                issuer: "x".into(),
                subject: "y".into(),
            },
            "peer",
        )
        .unwrap_err();
    assert_eq!(err.code_enum(), Some(v1::ErrorCode::Unauthorized));
}

#[test]
fn source_tags_do_not_collide() {
    let map = IdentityMap::new(vec![IdentityBinding {
        key: IdentityKey::Service {
            issuer: "iss".into(),
            service_id: "sid".into(),
        },
        principal_id: "svc".into(),
        tenant_id: "t".into(),
    }]);
    assert!(
        map.resolve(
            &IdentityKey::Oidc {
                issuer: "iss".into(),
                subject: "sid".into()
            },
            "p"
        )
        .is_err()
    );
    assert!(
        map.resolve(
            &IdentityKey::Service {
                issuer: "iss".into(),
                service_id: "sid".into()
            },
            "p"
        )
        .is_ok()
    );
}
