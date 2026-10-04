# conex 文档导航

conex 的文档按「规范 / 契约 / 计划 / 证据 / 操作」分层，每类只有一个权威来源。本页是入口；根 [README](../README.md) 只保留项目简介、快速开始与三个调用例子。

## 状态（2026-10-04 复核）

- **P0 已交付**（提交 `e649ded`）：broker 只读数据连接，JSON-RPC/HTTP 与 inproc 共用同一授权执行路径；fs 与 HTTP catalog 两个 provider；TypeScript consumer；P0 门禁与 CI。
- **P1 已交付**：两条 WSS Profile 真实握手（JSON text + protobuf 二进制，bearer 在 upgrade 认证）；`/wss` `/tickets` `/oidc/*` 挂入 host；`blob/session/operation/agent` 经 broker 派发并强制 bearer 主体绑定；P1-04 Stream 状态机（信用/ACK/重放/reset/slow_consumer）与 `stream.json` 行为测试；1 GiB 经 Stream 弱网续传 e2e（坏块 + 断线续传）；`[oidc]` 配置下 RS256 id_token 验签。见 [P1 验证记录](verification/p1.md)、[P1 运行手册](runbooks/p1.md)。
- **Connected landing 已交付**：中心 Host + 反连 Agent + 浏览器会话/ticket + 端点目录 + 连接面板；三条 e2e 套件（landing / landing-web / landing-connections，含真停 Agent 进程的生命周期场景）。见 [验证记录](verification/connected-landing.md)、[运行手册](runbooks/connected-landing.md)。
- **多主机内容 M0–M5 已交付**（分支 `refactor/arch`，未提交）：内容契约冻结（endpointId+revision 定位、blob/get 互斥目标、二进制 DataChunk 通道）、访客会话与 blob 所有权授权、逐端点注册生命周期、Agent 复用 provider 机制、同源 `/content`（Range/ETag/防注入/流式取消）、公共浏览页面（文本/图片/视频/DOCX/ZIP 受限预览，1440/390 双视口 38 项浏览器检查全过）。逐项证据见 [多主机内容验证记录](verification/multi-host-content.md)。
- **下一步**：M4（Notez 接入，需真实 Notez 仓库与实例）与 M6（验收收口），见 [多主机内容计划](plans/2026-10-01-conex-multi-host-content.md)。
- **落地页场景扩展（2026-10-05）**：页面从「hello 单场景」改为**项目介绍 + hello 场景 + 文件场景 + 状态**，默认落在 hello 场景。`group` 升级为隔离键（SHA-256 派生）：跨组不可见、不可问候、不可访问文件。`client/status` 报出全 Host 客户端数、分组数与每组实测延迟。文件共享支持选择文件或整个目录，按提供者分卡片，可预览与下载；提供者断开即回收。契约 `ClientSummary.groupKey` / `GroupStatus` / `ClientStatusResponse` 与四条 `/web/files*` 路由（`connected-landing` §10）。见 [验证记录](verification/landing-page.md)。
- **hello 页面已部署（2026-10-04）**：<https://conex.lszio.space>。契约 `ClientProfile` / `ClientSummary` / `client/*`（`connected-landing` §9），访客免凭据。见 [hello 页面验证记录](verification/landing-page.md)、[Dokploy 部署运行手册](runbooks/dokploy.md)。
- 完成状态与证据以各 [验证记录](verification/) 为准。

## 文档地图

| 类别 | 文档 | 作用 | 何时读 |
|---|---|---|---|
| 规范 | [设计 v6](design/2026-09-14-conex-design.md) | 唯一权威设计与分阶段验收 | 改契约、加能力前必读 |
| 契约 | [P0 wire 契约](contracts/p0-wire.md) | `conex-jsonrpc2-http` 信封、参数、错误码、HTTP 行为 | 实现或对接 wire 时 |
| 计划 | [路线图](plans/2026-09-15-conex-roadmap.md) | P0–P4 阶段、依赖与工作包 | 了解全局与推进顺序 |
| 计划 | [P1 执行计划](plans/2026-09-15-conex-p1.md) | 下一步：P1 范围、P1-01 细化、里程碑 | 开始下一步工作 |
| 计划 | [中心服务与交互落地页](plans/2026-09-20-conex-connected-landing.md) | 已确认 A 拓扑：浏览器连接中心 Host，私有 Agent 反连；含安全接入、真实路由、页面和验收 | 实施多地服务连接展示时 |
| 计划 | [多主机数据服务与公共内容页面优化](plans/2026-10-01-conex-multi-host-content.md) | M0–M6 工作包：契约冻结、权限修复、通用内容传输、Notez 接入、媒体展示及跨主机验收；M0–M5 已实施 | 推进多主机全类型数据公开浏览时 |
| 证据 | [多主机内容验证记录](verification/multi-host-content.md) | M0–M5 实际交付、测试文件索引、已知缺口（M4/M6 未开始、fmt 全仓债） | 验收、排障、追溯契约变更 |
| 契约 | [Connected landing](contracts/connected-landing.md) | 中心 Host、浏览器会话/ticket、Agent 反连与只读端点目录的 L01 冻结契约 | 实施 connected landing 或校验信任边界时 |
| 计划 | [P0 执行计划（归档）](plans/archive/2026-09-15-conex-p0.md) | 已完成的 P0 计划：冻结决定、任务→提交→证据、遗留缺口 | 追溯 P0 决策 |
| 证据 | [P0 验证记录](verification/p0.md) | 环境、命令、结果、逐任务已知缺口 | 验收、排障、审计 |
| 操作 | [P0 运行手册](runbooks/p0.md) | 生成、启动、入站 token / 出站凭据、SDK 调用 | 部署与联调 |
| 操作 | [Connected landing 运行手册](runbooks/connected-landing.md) | `cargo xtask landing-demo` 三进程、token 三元组、docker-compose、日志、门禁 | 部署 / 演示 / 联调多 Agent 场景 |
| 操作 | [Dokploy 部署运行手册](runbooks/dokploy.md) | 公开演示单容器形态与多主机生产形态、两个明文开关的区别、web_origin、故障排查 | 把落地页推到 Dokploy 时 |
| 证据 | [落地页验证记录](verification/landing-page.md) | 客户端契约、推送字节预算下溢与死链残留、文本预览乱码／后台标签页不轮询文件／撤回先删后校验三个缺陷、23 项三上下文浏览器验收、上一版部署故障根因 | 验收、追溯本轮修复时 |
| 证据 | [Connected landing 验证记录](verification/connected-landing.md) | 两条 e2e + Rust 单测 + bun 测试实际输出、未验证项 | 验收、排障 |

## 阅读顺序

1. **了解项目**：根 [README](../README.md) → 本页 → 设计 §0–§2 → 运行手册。
2. **上手实现**：设计 §4–§8 → [P0 wire 契约](contracts/p0-wire.md) → 对应阶段计划。
3. **验收与审计**：阶段验证记录 → 设计 §14 对应阶段验收项。

## 文档规则

- 结构类型只有一个源：`schema/conex/*.proto`；Rust/TS 类型与 JSON Schema 都是生成产物，禁止手改。权威严格解码点是 `MethodContract.prepare`/`validate_output`。
- **设计是规范，计划是执行范围与顺序，验证记录只写实际执行并通过的结果**；未实现能力必须明确标注，不把计划的 "Expected" 复制成结果。
- 已完成的阶段计划移入 `plans/archive/`，只保留冻结决定与证据索引；原全文见对应完成提交（P0 为 `e649ded`）。
- 文档命名：设计/专项规范用 `<日期>-<主题>.md`；阶段计划用 `<日期>-conex-<阶段>.md`。
