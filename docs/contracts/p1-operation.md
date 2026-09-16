# P1 operation 与幂等、副作用边界契约（设计 §7.3）

状态：**P1-01 冻结**（2026-09-15）。
类型源：`schema/conex/v1/operation.proto`。
向量：`conformance/vectors/p1/operation.json`，Rust（`crates/conex-proto/tests/operation.rs`）与 TS（`sdk/typescript/tests/operation.test.ts`）两侧分类一致。

## 1. operationId 与去重键

- `operationId` 是稳定 ULID；与 `requestId` 完全分离。
- 去重键 = `(tenantId, principalId, providerEndpointId, spaceId?, resourceId, method, operationId)`。
- 同一 key 不同参数摘要（`param_digest`）返回 `conflict`；新传输请求**不能**生成新 `operationId` 来绕开未知结果。
- 默认保留 24 小时，必须覆盖声明的重试期限；过期状态不可查时不能静默重新执行。

## 2. execution 类别与 retry 推导

```text
read_only        → retry=safe
idempotent       → retry=safe (仍重查策略)
deduplicated     → retry=with_operation_id
non_replayable 已发送 → retry=never & execution=unknown
```

四类全部以 `ExecutionClass` 枚举表达；P1 任意 MethodContract 必须在注册时选择其中之一，**不**允许落到「HTTP 5xx 自动推断安全」的运行时推断。

## 3. 状态机

```text
accepted → running → succeeded
                     → failed
                     → cancelled
                     → unknown
```

- `unknown` 表示「上游可能成功但响应丢失」；通过 `operation/get` 查询或人工核对；**不能**自动重试。
- 状态查询也必须授权（§8.3）；非授权主体拿到 `operationId` 也不能查到状态。
- `execution` 字段记录当前副作用状态；`unknown` 与 `non_replayable` 已发送的语义都进入该字段。

## 4. 持久化与崩溃恢复

- `OperationRecord` 写入本地持久存储；`schema_version = 1`，后续升级走 reserved + 新版本。
- 写入顺序：先持久记录（accepted），再向上游发起副作用；返回响应或失败后回写 settled 状态。
- 同一事务提交时才提供「去重记录 + 本地副作用」的原子保证；外部上游必须支持同一幂等键或可查询操作状态。
- 单独写一张 host 去重表**不能**消除外部调用的崩溃窗口（设计 §7.3）。

## 5. 取消语义

- `call/cancel` 指向 `requestId` / `operationId`；通知已送达**不等于**取消已完成。
- 结果与取消并发时，已确定的结果优先保留。
- 只有执行器确认未开始或已停止，才返回对应终态；无法判断是否产生副作用则返回 `outcome_unknown`。

## 6. 条件写（`expectedRevision`）

- `expectedRevision` 在上游执行点原子检查才有条件写语义。
- 只做「先读再写」的桥接不得广告条件写能力（设计 §7.3）。
- P1 范围：`source/write`（若 provider 实现）允许携带 `expectedRevision`；未实现该能力的 provider 在 `hello` 阶段不得广告 `blob/write` 或 `source/write`。

## 7. 已知答案向量边界

| 用例 | 期望 |
|---|---|
| 响应丢失（deduplicated） | `operation/get` 仍能取回结果；同一 `operationId` 重试返回同一结果 |
| 响应丢失（non_replayable） | `operation/get` 返回 `unknown`；客户端**不**自动重试 |
| 同 key 不同 `param_digest` | `conflict` |
| `operationId` 过期 | `operation/get` 返回 `unknown` 或错误；不静默重新执行 |
| 已 settled 的 operation 取消 | 终态保持；`outcome_unknown` 仅在 truly concurrent 时返回 |
| 未知 principal | `unauthorized` |
| 跨主体同 `operationId` | `unauthorized`（防止 ID 被猜到取得结果） |
| execution=unknown 的 non_replayable | `retry=never`；客户端不得自动重发 |

## 8. 待定（占位待定）

- 持久后端与崩溃恢复策略的具体存储路径属于 P1-08；本契约只规定 schema 与可观察行为。
- 跨进程恢复（host 重启后能否继续 operation）在 P1/P2 不承诺；返回 `session_lost`。