# conex 文档导航

conex 的文档按「规范 / 契约 / 证据 / 操作」分层，每类有明确权威来源。本页是入口；根 [README](../README.md) 是项目门面，区分已确认目标与现有可运行实现。

> 文档口径（2026-11-13 整理）：阶段实施计划已从仓库移除（含 `plans/archive/`），
> 计划内容的长期价值并入架构设计与功能契约；执行证据只保留在各验证记录。
> 历史计划全文在 git 历史可追溯（`git log -- docs/plans/`）。旧文档（历史设计、
> 验证记录）中的计划链接指向已移除文件，属历史记录的一部分，不改写。

## 架构与功能设计（权威来源）

| 文档 | 内容 |
|---|---|
| [MVP 架构基线](design/2026-10-29-conex-private-platform.md) | 当前定位、SICP 模型、对象/事件、所有权、数据边界和 A01–A16 产品验收 |
| [LLM 提供者架构增补](design/2026-11-12-conex-llm-provider-relay.md) | LLM 提供者领域、OpenAI 兼容网关、工作台模型服务视图与 L01–L07 验收（2026-11-12 用户确认；地基已实现，见 [运行手册 §4](runbooks/private-mvp.md)） |
| [Agent 领域契约](contracts/agent-domain.md) | 精确操作、身份、busy/幂等、取消、审批 CAS 与后端边界 |
| [事件恢复契约](contracts/event-recovery.md) | 节点持久流、快照边界、有界补读、head 提示、游标失效 |
| [私有运行契约](contracts/private-runtime.md) | 应用包、所有权、认证、直连/转发、离线授权与数据边界 |
| [S1 双向能力与状态订阅](contracts/s1-capabilities.md) | 通用注册、调用、取消、状态快照与更新、限额和验收场景 |
| [S1 模块消费主体](contracts/s1-module-principals.md) | 组合模块远程消费的模块粒度授权与归属（A14 消费侧） |
| [S2 插件运行时与能力组合](contracts/s2-plugins.md) | 插件/实例、本地 DI、显式远程绑定、派生模块、停止释放 |
| [Connected landing 契约](contracts/connected-landing.md) | 中心 Host、浏览器会话/ticket、Agent 反连与只读端点目录 |
| [P0 wire 契约](contracts/p0-wire.md) | `conex-jsonrpc2-http` 信封、参数、错误码、HTTP 行为 |
| [P1 流契约族](contracts/p1-stream.md)（chunking/blob/operation 同目录） | Stream 信用/ACK/重放状态机 |

历史设计（[旧设计](design/2026-09-14-conex-design.md)、[2026-10-17 架构](design/2026-10-17-conex-architecture.md) 及其迁移盘点）保留为追溯依据，不决定当前方向。

## 实现进度与证据

已交付且有真实验证记录的能力（详见各记录的「实际执行结果与已知缺口」）：

| 阶段 | 证据 |
|---|---|
| P0 broker 只读数据连接 | [P0 验证记录](verification/p0.md) |
| P1 私有侧反连、二进制通道、OIDC | [P1 验证记录](verification/p1.md) |
| Connected landing（中心 + 反连 agent + 工作台雏形） | [验证记录](verification/connected-landing.md)、[落地页](verification/landing-page.md) |
| 多主机内容 M0–M5 | [验证记录](verification/multi-host-content.md) |
| S1 能力 / S2 插件组合 | [S1](verification/s1-capabilities.md)、[S2](verification/s2-plugins.md) |
| MVP-P1 领域存储 / P2 自治节点 | [P1](verification/agent-domain-p1.md)、[P2](verification/private-runtime-p2.md) |
| MVP-P3 OMP / P4 Codex 适配器 | [P3](verification/agent-omp-p3.md)（live 全过）、[P4](verification/agent-codex-p4.md)（离线交付，live 被后端形态漂移阻断） |
| MVP-P5 私有访问与网络 / P6 指标关系 | [P5](verification/private-network-p5.md)、[P6](verification/private-entities-p6.md) |
| MVP-P7 Web 工作台 | [P7](verification/web-p7.md)（真实 Chrome 11/11） |
| S1 模块消费主体 / 统一审计汇 | [证据页](verification/s1-module-principals.md) |
| MVP-P10 LLM 中继地基（契约 + 节点模块 + hub 网关 + 模型服务视图） | `cargo xtask e2e --suite private-llm`；证据页待两机 live 后落盘 |
| 协议探针（OMP/Codex 安装版本实调） | [探针记录](verification/agent-protocol-probes.md) |

线上部署形态与操作见 [私有 MVP 运行手册](runbooks/private-mvp.md)；其余操作入口：
[自治节点](runbooks/private-runtime.md)、[私有网络](runbooks/private-network.md)、
[P0](runbooks/p0.md)、[P1](runbooks/p1.md)、[S1](runbooks/s1-capabilities.md)、
[S2](runbooks/s2-plugins.md)、[Connected landing](runbooks/connected-landing.md)、
[Dokploy](runbooks/dokploy.md)。

## 阅读顺序

1. **了解当前方向**：根 [README](../README.md) → [MVP 架构基线](design/2026-10-29-conex-private-platform.md) → [LLM 提供者架构增补](design/2026-11-12-conex-llm-provider-relay.md)。
2. **实现与对接**：对应功能契约 → 验证记录（证据口径）→ 运行手册。
3. **验收与审计**：架构 A01–A16 与设计增补 L01–L07 对照各验证记录的实际命令与结果。

## 文档规则

- 结构类型只有一个源：`schema/conex/*.proto`；Rust/TS 类型与 JSON Schema 都是生成产物，禁止手改。权威严格解码点是 `MethodContract.prepare`/`validate_output`。
- **设计是规范，验证记录只写实际执行并通过的结果**；未实现能力必须明确标注，不把计划的 "Expected" 复制成结果。
- 文档命名：设计/专项规范用 `<日期>-<主题>.md`；新领域设计入 `docs/design/`，功能契约入 `docs/contracts/`，不再新建计划文档。
