//! Address admission: public-only by default, explicit loopback, pinned dial address.
use std::net::IpAddr;

use conex_core::target_policy::TargetPolicy;
use conex_core::{Caller, Target, TlsTrust};

fn caller() -> Caller {
    Caller {
        principal_id: "p".into(),
        tenant_id: "t".into(),
        actor_peer_id: "peer".into(),
    }
}

fn target(origin: &str, fixed_path: &str, allow_loopback: bool) -> Target {
    Target {
        id: "catalog".into(),
        origin: origin.into(),
        fixed_path: fixed_path.into(),
        allowed_addresses: vec![],
        tls_trust: TlsTrust::default(),
        allow_loopback_http: allow_loopback,
    }
}

fn ip(value: &str) -> IpAddr {
    value.parse().unwrap()
}

#[test]
fn public_https_is_pinned_with_expected_server_name() {
    let allowed = TargetPolicy::new()
        .allow(
            &caller(),
            &target("https://catalog.example", "/api", false),
            &[ip("203.0.113.10")],
        )
        .unwrap();
    assert_eq!(allowed.pinned_address.ip(), ip("203.0.113.10"));
    assert_eq!(allowed.port, 443);
    assert_eq!(allowed.server_name, "catalog.example");
}

#[test]
fn private_loopback_and_link_local_are_rejected_by_default() {
    for address in [
        "10.0.0.5",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.1.1",
        "127.0.0.1",
        "0.0.0.0",
    ] {
        let result = TargetPolicy::new().allow(
            &caller(),
            &target("https://h.example", "/", false),
            &[ip(address)],
        );
        assert!(result.is_err(), "address {address} should be rejected");
    }
}

#[test]
fn mixed_dns_result_uses_only_a_permitted_address() {
    let allowed = TargetPolicy::new()
        .allow(
            &caller(),
            &target("https://h.example", "/", false),
            &[ip("10.0.0.1"), ip("198.51.100.7")],
        )
        .unwrap();
    assert_eq!(allowed.pinned_address.ip(), ip("198.51.100.7"));
}

#[test]
fn loopback_plaintext_requires_flag_and_loopback() {
    let allowed = TargetPolicy::new()
        .allow(
            &caller(),
            &target("http://127.0.0.1:8080", "/", true),
            &[ip("127.0.0.1")],
        )
        .unwrap();
    assert_eq!(allowed.port, 8080);
    // Flag off: loopback rejected.
    assert!(
        TargetPolicy::new()
            .allow(
                &caller(),
                &target("http://127.0.0.1", "/", false),
                &[ip("127.0.0.1")]
            )
            .is_err()
    );
    // Flag on but address not loopback: plaintext http rejected.
    assert!(
        TargetPolicy::new()
            .allow(
                &caller(),
                &target("http://h.example", "/", true),
                &[ip("198.51.100.1")]
            )
            .is_err()
    );
}

#[test]
fn userinfo_path_and_dynamic_path_are_rejected() {
    assert!(
        TargetPolicy::new()
            .allow(
                &caller(),
                &target("https://user@h.example", "/", false),
                &[ip("198.51.100.1")]
            )
            .is_err()
    );
    assert!(
        TargetPolicy::new()
            .allow(
                &caller(),
                &target("https://h.example/some/path", "/", false),
                &[ip("198.51.100.1")]
            )
            .is_err()
    );
    assert!(
        TargetPolicy::new()
            .allow(
                &caller(),
                &target("https://h.example", "/api/{id}", false),
                &[ip("198.51.100.1")]
            )
            .is_err()
    );
}

#[test]
fn explicit_allowlist_is_intersected() {
    let mut t = target("https://h.example", "/", false);
    t.allowed_addresses = vec![ip("198.51.100.7")];
    assert!(
        TargetPolicy::new()
            .allow(&caller(), &t, &[ip("198.51.100.7")])
            .is_ok()
    );
    assert!(
        TargetPolicy::new()
            .allow(&caller(), &t, &[ip("198.51.100.8")])
            .is_err()
    );
}
