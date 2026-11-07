# P1 统一审计汇（M4 补齐）实施计划

> 日期：2026-11-08。分支 `feat/p1-audit-sink`（基于 dev）。
> 背景：SICP 完善（2026-11-04 审查）刀2 的 M4 被显式推迟——「S1 能力路径当前没有审计出口（agent.rs:745 已注明统一 P1 audit sink 为待办）」。
> 契约承诺：[s1-module-principals](../contracts/s1-module-principals.md) §3 —— operator principal 不随转发下传，保留在中心的审计与绑定图记录里：`(连接主体, 模块主体, generation, 目标, 方法)`。

## 现状

- core 已有审计端口：`AuditSink::reserve`（先预留后副作用）+ `AuditStart`/`AuditEnd`；P0 能力路径（`Host::invoke`）已接。
- host 已有 `FileAuditSink`（0600、容量有界、预留失败即拒绝）与 `NullAudit`。
- **缺口**：P1 broker 分发路径不经过 `Host::invoke`，无审计；且 `FileReservation::finish` 只写 End 摘要，`AuditStart`（谁、对谁、什么方法）被丢弃——现有审计文件只有容量会计、没有内容。

## 决定

1. **钩子位置**：`Broker::invoke` 分发前 `reserve`、分发后 `finish`。`Capability` 回退族**不重复审计**（它落回 `Host::invoke`，归 P0 的审计域，避免双记）。
2. **记录内容**：`AuditStart` 增量加 `consumer_module: Option<String>` 与 `generation: u64`；`FileAuditSink` 改为预留时持有 Start、收尾时写**一行完整 JSON**（Start 字段 + End 字段，camelCase 与现状键风格一致；既有键名不变）。
3. **判定拒绝也留审计**：三步判定失败发生在分发内（预留之后），`outcome="error"`、`error_code=forbidden`；`execution` 取错误自带的诊断阶段（判定拒绝为 `not_started`——刀5② 的类型化在此处直接受益）。
4. **容量语义不变**：预留失败 → 拒绝先于副作用；P1 记录与 P0 共享同一 `audit_file`/容量（同一个 `Arc<dyn AuditSink>` 同时注入 Host 与 `BrokerDeps`）。
5. **身份映射**：`event_id` 为中心侧 ULID；`trace_id` 用 JSON-RPC request id（真实链路关联）；`resource_scope` S1 调用为 `spaceId/registrationId`，其余族为 endpoint id；`policy_version=0`（P1 分发无 grant policy）。

## 影响清单

| 位置 | 变更 |
|---|---|
| `conex-core/src/audit.rs` | `AuditStart` 增 `consumer_module`/`generation`（增量，`None`/`0` 为缺省） |
| `conex-core/src/execution.rs` | 构造点补缺省字段 |
| `conex-host/src/audit_file.rs` | Reservation 持有 Start；finish 写完整记录；测试断言内容落盘 |
| `conex-host/src/broker.rs` | `BrokerDeps.audit: Arc<dyn AuditSink>`（去 Default）；invoke 预留/收尾；`audit_start` 构造 |
| `conex-host/src/serve.rs` | 同一 audit Arc 注入 BrokerDeps |
| `conex-host/tests/*` | 既有 5 处 BrokerDeps 构造补 `NullAudit`；新增模块主体判定拒绝的记录断言 |

## 验收

- 新单测：判定拒绝产生一行含 `consumerModule`/`principalId`(连接主体)/`method`/`outcome=error`/`errorCode=forbidden` 的记录；批准路径 `outcome=allow`。
- 既有审计测试（容量/权限）不变全绿；既有 P1/S1 套件不改动仍通过。
- 全门禁 `cargo xtask check` 绿。

## 非目标

- 绑定图落表（基线 §6 的图存储）——审计记录是它的原料，落表另立工作项。
- WSS/HTTP 之外的平面、审计轮转/导出、P0 记录形状变更。
