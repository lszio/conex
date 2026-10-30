# P1 实施记录：领域状态与恢复

日期：2026-10-29。计划：[MVP-P1](../plans/2026-10-29-conex-mvp-p1.md)。契约：[Agent 领域](../contracts/agent-domain.md)、[事件恢复](../contracts/event-recovery.md)、[私有运行](../contracts/private-runtime.md)。本页只记录实际运行结果与偏差；不表示 OMP/Codex 适配器、Web/Emacs 客户端或公网能力已交付。

## 1. 交付物

| 类别 | 文件 | 状态 |
|---|---|---|
| 协议源 | `schema/conex/{entities,events,agent_domain}.proto` | 新增，package conex |
| 生成物 | `schema/generated/**`、`sdk/typescript/src/generated/**` | 由 `cargo xtask generate` 派生，`--check` 一致 |
| 边界校验 | `crates/conex-proto/src/agent_domain.rs` | 新增：18 方法目录、read_only/non_replayable、权限词表、ULID/规范 uint64/字节上限/decision 互斥/游标上限 |
| 领域实现 | `crates/conex-agent-domain/{src/{lib,model,projection,store,ulid}.rs,src/schema.sql}` | 新增：真实 SQLite 存储 |
| 回归 | `crates/conex-agent-domain/tests/{admission,interactions,recovery}.rs` | 新增 16 条消费者可见回归 |
| 跨语言 smoke | `crates/conex-agent-domain/examples/recovery_smoke.rs` | 新增：真实库 → 重启 → 恢复 → NDJSON |
| TS 客户端 | `sdk/typescript/src/{agent-domain,events}.ts`、`src/index.ts` | 新增：AgentClient、EventFollower/reducer |
| TS 回归与 smoke | `sdk/typescript/tests/agent-domain.test.ts`、`examples/agent-recovery-smoke.ts` | 新增 |
| 共享向量 | `conformance/vectors/agent-domain.json` | 新增：Rust/TS 同源正负向量 |

## 2. 实际运行结果

| 命令 | 结果 |
|---|---|
| `cargo xtask generate` / `generate --check` | 通过，生成物一致 |
| `cargo test -p conex-proto` | 全过（含 3 条 agent_domain 向量测试） |
| `cargo test -p conex-agent-domain` | 全过：2 单元 + 7 admission + 5 interactions + 8 recovery（22 条） |
| `bun test`（sdk/typescript） | 159 pass / 0 fail（含新增 11 条） |
| `bun run typecheck` | 通过 |
| `cargo run -p conex-agent-domain --example recovery_smoke -- --self-check` | 通过，TS 消费者输出 `{"ok":true,...,"cursorAfter":"13","head":"13","events":1,"messages":2,"replayChecked":true}` |
| `cargo xtask check` | 通过（`xtask check: all steps passed`：fmt、clippy `-D warnings`、workspace 测试、bun install/typecheck/test、`generate --check`、conformance 与 e2e 套件） |

跨语言 smoke 的真实路径：临时节点库 → 会话 D 正常完成（终态清槽）→ 会话 B 只到 accepted → 会话 U 分派后流式产出正文 → 关闭进程 → 重新打开同一 nodeId → `recover_unfinished` 把 U 置 unknown 并保留槽、把 B 置 failed/not_dispatched 并释放槽 → 快照（含活动消息正文窗口）→ 后端再投一个片段（delta 接在 totalBytes 之后）→ 事件页 NDJSON → TS 侧同一 reducer 消费。

## 3. 关键不变量如何被证明

| 不变量 | 证明方式 |
|---|---|
| 同键同摘要只产生一个执行，另一个无发送许可 | admission：两独立 Store + 屏障并发，断言 1 个 Created、1 个 Existing 且执行 ID 相同 |
| 同键不同摘要冲突 | admission：`IdempotencyMismatch` |
| busy 不入队、不消耗幂等键 | admission：busy 之后旧执行终态，同一 key 仍可 Created |
| accepted 可证明未发送 → failed/not_dispatched 清槽 | recovery：崩溃重开后断言终态、原因与槽释放，随后可再次准入 |
| dispatching/running → unknown 保留槽、不重发 | recovery：断言 target 为 unknown、槽仍 busy；后端确认退出后由 Quiescent 清槽 |
| 审批只有一次后端发送 | interactions：两端屏障竞答，恰好 1 个 permit，1 个 Conflict/StaleRevision |
| 相同答复可查回、不同答复冲突、unknown 不可再答 | interactions：retry 返回 resolved 且 revision 不变；不同决定 Conflict；unknown 后二次回答 Conflict |
| 过期审批拒绝新答 | interactions：`interaction_expired` |
| 事件位置连续、编码有界 | admission：正文按 ≤4096 字节切块，单事件编码上限 8192 字节由写入路径强制 |
| 旧 epoch / 已清理前缀必须重同步 | recovery：restore 后旧游标 `ResumeUnavailable`；prune 后 `firstAvailable` 前进且旧位置 `ResumeUnavailable`；游标矩阵覆盖 `firstAvailable-1` 可读、`firstAvailable-2` 重同步、`head+1` 非法、旧 epoch 重同步 |
| 快照在预算内且可恢复 | recovery：25 个终态执行 + 20_000 字节长回复 → 快照编码在结果预算内，终态按预算裁剪，长正文经分页补读 |
| 节点库不可跨 nodeId 共用 | recovery：另一个 nodeId 打开/恢复均被拒绝 |
| 正文窗口与 totalBytes 正确 | admission：`agent/messages/read` 分页取回全文，`totalBytes` 与编码后长度一致 |
| 配额超限不无限增长 | admission：活跃消息超配额 → 有界失败，执行转 unknown、写 recovery.gap、端点隔离，槽仍被占用 |

## 4. 与计划的偏差（含理由）

1. **记录消息的 proto 文件归属**：计划写“`agent_domain.proto` 领域记录”，但事件 `data` 需要引用记录、`events.proto` 又被 `agent_domain.proto` 引用会形成 protoc 禁止的循环导入。为保持无环，记录移到 `entities.proto`（`agent_domain → events → entities`），字段与契约逐项一致。类型名同时加前缀以避开既有 `conex.Message`/`rpc.proto` 冲突：`AgentMessage`、`AgentMessagePart`、`AgentTextPart`、`AgentToolPart`；契约的 `AgentEndpoint` 因 `agent.proto` 已占用控制面同名消息，领域记录命名为 `AgentAdapterEndpoint`（wire JSON 不含消息名）。
2. **新增 `Store::backup_to`**：`restore` 需要真实备份来源，计划 API 表未列；补齐为公开方法，并由 restore 回归覆盖。
3. **新增 `put_endpoint`/`put_workspace`**：端点与工作区实体由节点配置产生，P1 需要写入入口以支撑回归与 smoke；不含任何客户端可写路径。
4. **快照正文窗口策略**：活动（未终态执行或 streaming）消息在预算内带正文窗口，其余消息只给元数据与正确 `totalBytes`；客户端经 `agent/messages/read` 补读。这样 `message.delta` 的 `offsetBytes` 始终等于客户端已知 `totalBytes`。
5. **`Execution.moduleInstance_id` 在 P1 为空**：模块实例代际由 P2 节点模块管理器提供；P1 只按端点 generation 建执行，未伪造实例 ID。
6. **关系与 backend_mappings 表已在 schema 内但 P1 无写入路径**：关系来自后端子执行证据（P3/P4 适配器），`backend_mappings` 由适配器在拿到真实后端会话 ID 后登记；P1 不伪造映射。
7. **TS 侧 proto3 默认值省略**：`partIndex`/`offsetBytes` 为 0 时 JSON 省略，reducer 与 smoke 消费者按 proto3 语义取默认 0，而不是判定为缺失并重同步。
8. **恢复契约 §3 第 4 步的 5 秒主动读取健康核对未实现**：计划 §7 的权威状态机未包含该轮询；`recover()` 覆盖断线后的重驱动。

## 5. 未交付与后续

- 未实现 OMP/Codex 适配器、未接入 Host `module/invoke` 分派、未实现 Web/Emacs 客户端、未部署公网服务；这些属于 P2/P3/P4/P5/P9。
- `agent/*` 方法目前只有边界校验与领域存储，HTTP/WS 暴露与 Authorizer 在 P2。
- 审批到后端的一次性发送由 P2 适配器执行；P1 只提供 permit 与 CAS 语义。

## 6. 门禁与图索引

`cargo xtask check` 为仓库总门禁（含格式、clippy `-D warnings`、workspace 测试、bun 安装/类型检查/测试、生成物一致性、conformance 与 e2e 套件）。本轮结果：`xtask check: all steps passed`。

图索引：动手前 `index_status` 为 ready；本轮结束后已重新 `index_repository`（115343 节点/144068 边），`search_graph` 可查到 `Store.mark_dispatching`、`Store.recover_unfinished`、`Store.begin_response`。`detect_changes` 只报告已存在于索引的文件，未跟踪的新文件（`crates/conex-agent-domain/**`、三个新 proto、两个新 TS 模块、新向量文件）不进入 changed_files —— 提交前需再次以含新文件的索引复核影响面。

本轮未提交、未推送、未部署，也未改动用户凭据或现有会话。
