# conex 文档导航

conex 的文档按「规范 / 契约 / 计划 / 证据 / 操作」分层，每类只有一个权威来源。本页是入口；根 [README](../README.md) 只保留项目简介、快速开始与三个调用例子。

## 状态（2026-09-16 复核）

- **P0 已交付**（提交 `e649ded`）：broker 只读数据连接，JSON-RPC/HTTP 与 inproc 共用同一授权执行路径；fs 与 HTTP catalog 两个 provider；TypeScript consumer；P0 门禁与 CI。
- **P1 库级切片完成**（`a2ee453`）：`conex-proto::cid` 单一寻址 + `ChunkManifest` wire（`P1-01b`，`7daf172`）、`conex-content` 持久化与崩溃恢复、`conex-core::{session,operation}`、`conex-host::broker` 库、`conex-host::agent` OIDC+ticket、`conex-agent` CLI 桩、`p1_e2e` 进程内 4 用例、`xtask check` 全绿。**P1 尚未接入 host 装配根**——`serve::build` 不构造 P1 后端，`/wss` `/tickets` `/oidc/*` 在运行二进制 404，broker/agent 不经过资源策略与审计，P1-04 Stream 仍为设计验收项。逐包证据见 [P1 验证记录](verification/p1.md)；接线、授权/审计修复、P1-04 Stream、真 OIDC 完成后才进入 P2（ACP/MCP，详见 [路线图](plans/2026-09-15-conex-roadmap.md)）。
- 完成状态与证据以 [P1 验证记录](verification/p1.md)、[P0 验证记录](verification/p0.md) 为准；P0 执行计划已归档。

## 文档地图

| 类别 | 文档 | 作用 | 何时读 |
|---|---|---|---|
| 规范 | [设计 v6](design/2026-09-14-conex-design.md) | 唯一权威设计与分阶段验收 | 改契约、加能力前必读 |
| 契约 | [P0 wire 契约](contracts/p0-wire.md) | `conex-jsonrpc2-http-v1` 信封、参数、错误码、HTTP 行为 | 实现或对接 wire 时 |
| 计划 | [路线图](plans/2026-09-15-conex-roadmap.md) | P0–P4 阶段、依赖与工作包 | 了解全局与推进顺序 |
| 计划 | [P1 执行计划](plans/2026-09-15-conex-p1.md) | 下一步：P1 范围、P1-01 细化、里程碑 | 开始下一步工作 |
| 计划 | [P0 执行计划（归档）](plans/archive/2026-09-15-conex-p0.md) | 已完成的 P0 计划：冻结决定、任务→提交→证据、遗留缺口 | 追溯 P0 决策 |
| 证据 | [P0 验证记录](verification/p0.md) | 环境、命令、结果、逐任务已知缺口 | 验收、排障、审计 |
| 操作 | [P0 运行手册](runbooks/p0.md) | 生成、启动、入站 token / 出站凭据、SDK 调用 | 部署与联调 |

## 阅读顺序

1. **了解项目**：根 [README](../README.md) → 本页 → 设计 §0–§2 → 运行手册。
2. **上手实现**：设计 §4–§8 → [P0 wire 契约](contracts/p0-wire.md) → 对应阶段计划。
3. **验收与审计**：阶段验证记录 → 设计 §14 对应阶段验收项。

## 文档规则

- 结构类型只有一个源：`schema/conex/v1/*.proto`；Rust/TS 类型与 JSON Schema 都是生成产物，禁止手改。权威严格解码点是 `MethodContract.prepare`/`validate_output`。
- **设计是规范，计划是执行范围与顺序，验证记录只写实际执行并通过的结果**；未实现能力必须明确标注，不把计划的 "Expected" 复制成结果。
- 已完成的阶段计划移入 `plans/archive/`，只保留冻结决定与证据索引；原全文见对应完成提交（P0 为 `e649ded`）。
- 文档命名：设计/专项规范用 `<日期>-<主题>.md`；阶段计划用 `<日期>-conex-<阶段>.md`。
