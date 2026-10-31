//! Agent 后端适配器（docs/plans/2026-10-31-conex-mvp-p3.md、-p4.md）。适配器是
//! 普通节点模块：`conex.agent/1` 契约的方法经既有 invoke 路径分派，不新增顶层
//! JSON-RPC 方法；领域状态机以节点 SQLite 为唯一权威，后端帧只提供观察。
pub mod omp;

/// 契约 §1 权限类到 agent/* 操作的映射。方法名与权限类不一致时不允许
/// 字面点号权限绕过语义：权限类与操作一一对应，未映射的操作必须拒绝。
pub fn agent_permission_class(method: &str) -> Option<&'static str> {
    match method {
        "agent/endpoints/list"
        | "agent/workspaces/list"
        | "agent/conversations/list"
        | "agent/conversations/get"
        | "agent/conversations/history"
        | "agent/messages/read"
        | "agent/executions/get"
        | "agent/interactions/list"
        | "agent/relationships/list"
        | "agent/subagents/list"
        | "events/snapshot"
        | "events/read" => Some("agent.read"),
        "agent/conversations/create" => Some("agent.create"),
        "agent/executions/submit" | "agent/executions/steer" => Some("agent.execute"),
        "agent/executions/cancel" | "agent/subagents/cancel" => Some("agent.cancel"),
        "agent/interactions/respond" => Some("agent.interact"),
        _ => None,
    }
}

/// 调用方权限判定：字面点号形式（counter/read -> counter.read）或 agent 权限类。
/// node.rs 的两个 invoke 路径与 assembly 的 exposed_methods 过滤共用此判定。
pub fn method_allowed<'a, I>(permissions: I, method: &str) -> bool
where
    I: IntoIterator<Item = &'a String>,
{
    let dotted = method.replace('/', ".");
    let class = agent_permission_class(method);
    let mut matched = false;
    for permission in permissions {
        if *permission == dotted {
            matched = true;
        }
        if class.is_some_and(|class| *permission == class) {
            matched = true;
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::method_allowed;
    use std::collections::BTreeSet;

    #[test]
    fn agent_methods_require_contract_permission_classes() {
        let mut permissions = BTreeSet::new();
        permissions.insert("counter.read".to_string());
        assert!(!method_allowed(&permissions, "agent/executions/submit"));
        assert!(!method_allowed(&permissions, "counter/add"));
        permissions.insert("agent.execute".to_string());
        assert!(method_allowed(&permissions, "agent/executions/submit"));
        assert!(!method_allowed(&permissions, "agent/executions/cancel"));
        // 未映射的方法没有权限类，不能靠泛化前缀放行。
        assert!(!method_allowed(&permissions, "agent/executions/unknown"));
        permissions.insert("agent.interact".to_string());
        assert!(method_allowed(&permissions, "agent/interactions/respond"));
        let literal = ["agent.endpoints.list".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(method_allowed(&literal, "agent/endpoints/list"));
    }
}
