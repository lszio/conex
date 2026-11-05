# Conex S1 模块消费主体 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `executing-plans` or `subagent-driven-development`，按任务执行；先读 [模块消费主体契约](../contracts/s1-module-principals.md)（已评审，2026-11-05 定案）与 [S1 契约](../contracts/s1-capabilities.md) §4/§5.2/§7。**状态（2026-11-06）：M0–M3、M5 已实施**，实施中的设计修订（node 段取链路认证 peer、节点 remote_deps 启动语义）以 [证据页](../verification/s1-module-principals.md) §3 为准；M4 审计维度待统一 P1 audit sink。

**Goal:** 组合模块经显式远程绑定消费能力时获得模块粒度的授权与归属：`ConsumeApproval` 接受模块主体，中心按「形状 → 节点归属 → 批准」三步判定，提供端转发上下文绑定真实消费者，为 A14 消费侧提供数据与验收。

**Architecture:** 纯增量。唯一新 wire 字段是 `RequestContext.consumer_module`（tag 8）；批准表仍是精确元组、默认拒绝；操作员路径（字段缺省）行为逐字节不变。中心不独立验证进程内归属——节点声明（attestation）是既定 MVP 信任边界（基线 §10.2）的显式化。本地依赖直调零帧，不入模型。

**Tech Stack:** 无新依赖。现有 proto 生成链（prost/pbjson/ts-proto）、conex-core 批准表、host capability 判定、双 SDK、xtask e2e。

## 1. 前置与文件

P2 节点/发布、P5 授权底座、S1/S2 契约与实现已交付。codebase-memory 图工具近几个会话不可用；每个被改符号先做调用点全量枚举（grep + 逐点核对）并在提交正文注明。

| 文件 | 责任 |
|---|---|
| `schema/conex/rpc.proto` | 本包唯一结构扩展点：`RequestContext.consumer_module = 8`（string）；不新增消息类型 |
| `crates/conex-core/src/capability.rs` | 模块主体形状解析/校验（`module/<nodeId>/<applicationId>/<moduleId>`，段规则沿用 `is_valid_identifier`）；`ConsumeApproval` 文档更新 |
| `crates/conex-host/src/capability/{mod,calls,registry}.rs` | 配置装载接受模块主体；`module/invoke`、`state/subscribe` 的三步判定与转发上下文 `principal_id` 改填 |
| `crates/conex-host/src/{broker,ws_transport}.rs` | 消费→中心帧的 `consumer_module` 提取与校验入口；转发帧盖章；审计记录扩展 |
| `crates/conex-sdk/src/plugin.rs`（Rust） | `RemoteDep` 出站请求自动填充组合身份（node/application/module）；`InstanceConfig` 增加身份来源 |
| `sdk/typescript/src/plugin.ts`（TS） | 与 Rust 对称 |
| `crates/conex-agent/src/{node,assembly}.rs` | 把节点身份与应用包标识传入 SDK 实例组合上下文 |
| `xtask/src/{e2e,private_module_principal_e2e}.rs` | 新套件 `module-principals`：M-A01…M-A07 |
| `docs/verification/s1-module-principals.md` | 证据页（完成后写实际结果） |

实施顺序 M0→M1→M2→M3→M4→M5，每包 `cargo xtask check` 全绿后独立提交；契约回写（M6）随 M1 的实施提交完成。

## 2. M0 schema 与生成链

- [ ] `RequestContext` 增加 `string consumer_module = 8;`，注释写明：客户端（节点 SDK）声明消费主体，中心校验后盖章转发；空 = 连接主体自身。
- [ ] `cargo xtask generate` 重新生成 Rust/TS/JSON Schema，`generate --check` 过；无其他 proto 变更，`reserved` 不动。

## 3. M1 中心判定（host）

- [ ] conex-core：模块主体解析（三段形状 + `nodeId` 提取）；非法形状返回类型化错误供 host 映射 `bad_request`。
- [ ] 配置装载（`capability/mod.rs::from_config`）：`consume` 批准的 `principal_id` 允许模块主体形状（操作员主体规则不变）；`validate_contracts`/`validate_keys` 逻辑不变。
- [ ] `capability/calls.rs::invoke` 与 `state/subscribe` 判定改为：请求带 `consumer_module` 时——①形状非法 → `bad_request`；②`nodeId` 段 ≠ 连接认证节点 → `forbidden`；③`(模块主体, spaceId, endpointId, contractId)` 无精确批准或方法不在投影 → `forbidden`。三者均在发送边界前，不触达提供端。字段缺省走原路径，零行为差异。
- [ ] 通过后转发上下文 `principal_id` 填模块主体、`tenant_id` 沿连接；审计记录增加 consumer 维度（连接主体、模块主体、generation、目标、方法）。HTTP `module/invoke` 无特判：操作员 binding 的节点归属检查天然 `forbidden`（契约 §3 步骤 2）。
- [ ] `s1-capabilities.md` §4.1/§5.2/§7 增补交叉引用（随本包提交）。

## 4. M2/M3 双 SDK 身份填充与节点接线

- [ ] Rust SDK：`InstanceConfig` 增加消费身份来源（node/application 标识）；`RemoteDep::invoke/subscribe` 出站 `params.context.consumer_module = module/<nodeId>/<applicationId>/<moduleId>`（moduleId 取依赖消费所在模块名，由组合上下文提供，作者不手写、不可声明他节点）。未配置身份的独立 SDK 实例保持空字段——操作员路径不变。
- [ ] TS `plugin.ts` 对称实现；跨语言验收沿用 S1/S2 既有对齐方式。
- [ ] `conex-agent`：`node.rs` 装配应用实例时传入 `NodeConfig.node_id` 与应用包稳定标识（assembly 已有 label）；本节点模块消费本节点发布同样走该路径。

## 5. M4 审计与绑定图数据

- [ ] 中心审计出口扩展 consumer 字段（M1 已列数据）；P6 relations 的能力绑定投影无需改表：绑定记录以审计/调用记录为源，视图消费归 P7，本包只保证数据可得。

## 6. M5 验收与向量

- [ ] 新 e2e 套件 `cargo xtask e2e --suite module-principals`，覆盖 M-A01…M-A07（复用 S2 双实例 harness；竞态用屏障不用 sleep）。关键断言：M-A01 无批准 forbidden 且 handler 未执行；M-A02 冒用他节点 forbidden；M-A03 形状非法 bad_request；M-A04 两层消费 + 再发布被第三消费者使用；M-A05 `deps.local` 零帧回归；M-A06 撤销批准后在途按 S1 §7 收敛；M-A07 现有 S1/S2 套件不改一行仍全绿。
- [ ] 核心形状校验加单元测试（conex-core）与 JSON 向量（conformance/vectors/s1 旁路目录或既有结构）。
- [ ] 证据页 `docs/verification/s1-module-principals.md`：只写实际执行并通过的结果；未跑项如实标注。

## 7. 运行与退出

- [ ] `cargo xtask check` 全绿；浏览器验收不涉及（无视图变更）。
- [ ] 契约状态在 M5 证据齐后改为「已实施，见证据页」；`docs/README.md` 状态列同步。
- [ ] 退出条件：A14 的消费侧可被真实证据支撑（组合模块经模块主体消费两个能力、再发布、被第三消费者使用），且操作员路径回归零差异。
