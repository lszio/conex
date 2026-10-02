//! Both real providers register and install through one assembly root.
use conex_assembly::{factories, register_all};
use conex_core::Registry;

#[test]
fn both_factories_are_registered_without_core_branches() {
    let keys: Vec<String> = factories().into_iter().map(|(key, _)| key.kind).collect();
    assert!(keys.contains(&"source-fs".to_string()));
    assert!(keys.contains(&"source-http-catalog".to_string()));

    let mut registry = Registry::new();
    register_all(&mut registry).unwrap();
    assert_eq!(
        registry.method_count(),
        0,
        "registration must not install routes by itself"
    );
}
