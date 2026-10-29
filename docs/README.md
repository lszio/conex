# conex 文档导航

conex 的文档按「规范 / 契约 / 计划 / 证据 / 操作」分层，每类有明确权威来源。本页是入口；根 [README](../README.md) 是项目门面，区分已确认目标与现有可运行实现。

## 当前目标与计划（2026-10-29）

已确认 [私有分布式能力组合平台 MVP 架构基线](design/2026-10-29-conex-private-platform.md)：单用户私有、自治节点与协调入口、私网主动 WSS 接入、获准直连/转发、OMP/Codex 适配器、对话及图谱、服务器指标；MVP 同时支持 Web 与最小 Emacs 原生客户端。Desktop/Mobile 后续扩展；不把 Astro、多用户治理、自动打洞或插件市场作为前置条件。

按 [实施总计划](plans/2026-10-29-conex-private-mvp.md) 推进，先执行 [P0 契约与真实协议探针计划](plans/2026-10-29-conex-mvp-p0.md)。**新 MVP 尚未实施**；A01–A16 是待验收条件，不是已有结果。本轮只同步文档与计划，不自动修改 Paca、提交、部署或启动业务实现。

既有 [S1 契约](contracts/s1-capabilities.md) 与 [S2 契约](contracts/s2-plugins.md) 在迁移前仍描述现有 wire/运行时。S1/S2 已有实现与历史验收，见 [S1 验证记录](verification/s1-capabilities.md)、[S2 验证记录](verification/s2-plugins.md)；其中中心盖章授权、完整状态订阅和连接绑定生命周期不能直接当成节点离线授权、持久对话恢复或自治运行。

[2026-10-17 架构](design/2026-10-17-conex-architecture.md)、[迁移盘点](design/2026-10-17-conex-migration-assessment.md) 及 S1–S5 路线是历史基线，不再决定当前 MVP 范围。旧 Paca 编号仅属于原阶段；新工作包未在本轮登记。

现有操作入口：[S1 运行手册](runbooks/s1-capabilities.md)、[S2 运行手册](runbooks/s2-plugins.md)、[双实例样例](../examples/s2/README.md)。

## 历史实现状态（2026-10-04 复核及后续补记）

- **P0 已交付**（提交 `e649ded`）：broker 只读数据连接，JSON-RPC/HTTP 与 inproc 共用同一授权执行路径；fs 与 HTTP catalog 两个 provider；TypeScript consumer；P0 门禁与 CI。
- **P1 已交付**：两条 WSS Profile 真实握手（JSON text + protobuf 二进制，bearer 在 upgrade 认证）；`/wss` `/tickets` `/oidc/*` 挂入 host；`blob/session/operation/agent` 经 broker 派发并强制 bearer 主体绑定；P1-04 Stream 状态机（信用/ACK/重放/reset/slow_consumer）与 `stream.json` 行为测试；1 GiB 经 Stream 弱网续传 e2e（坏块 + 断线续传）；`[oidc]` 配置下 RS256 id_token 验签。见 [P1 验证记录](verification/p1.md)、[P1 运行手册](runbooks/p1.md)。
- **Connected landing 已交付**：中心 Host + 反连 Agent + 浏览器会话/ticket + 端点目录 + 连接面板；三条 e2e 套件（landing / landing-web / landing-connections，含真停 Agent 进程的生命周期场景）。见 [验证记录](verification/connected-landing.md)、[运行手册](runbooks/connected-landing.md)。
- **多主机内容 M0–M5 已交付**：`refactor/arch` 已合入 `dev`。内容契约冻结（endpointId+revision 定位、blob/get 互斥目标、二进制 DataChunk 通道）、访客会话与 blob 所有权授权、逐端点注册生命周期、Agent 复用 provider 机制、同源 `/content`（Range/ETag/防注入/流式取消）、公共浏览页面（文本/图片/视频/DOCX/ZIP 受限预览，1440/390 双视口 38 项浏览器检查全过）。逐项证据见 [多主机内容验证记录](verification/multi-host-content.md)。
- **原阶段未完成项**：M4（Notez 接入）与 M6（验收收口），见 [多主机内容计划](plans/2026-10-01-conex-multi-host-content.md)。这些不是当前私有 MVP 的默认下一步或前置条件。
- **落地页工作台（2026-10-05）**：**一个标签页就是一个客户端**——链接改在取 WSS 票据时铸造，同一浏览器的两个标签页共享 cookie 却是两条链接、两个独立客户端（`x-conex-link-id` 让同源 HTTP 路由认出是哪个标签页在请求）。页面从标签改为工作台：顶部状态栏常驻本标签页名字、短链接、分组 key 与在线数，问候从消息泡泡改为右上角 toast。`client/hello` 新增 `payload` / `answer`：带参数即**提问**，目标 SDK 不再自动确认，回答由页面通知里的「确认并回复」给出。契约 `ClientHelloRequest.payload` / `ClientHelloResult.answer` 与链接铸造时机见 [connected-landing §9](contracts/connected-landing.md)，28 项真浏览器验收见 [验证记录](verification/landing-page.md)。
- **hello 页面已部署（2026-10-04）**：<https://conex.lszio.space>。契约 `ClientProfile` / `ClientSummary` / `client/*`（`connected-landing` §9），访客免凭据。见 [hello 页面验证记录](verification/landing-page.md)、[Dokploy 部署运行手册](runbooks/dokploy.md)。
- 完成状态与证据以各 [验证记录](verification/) 为准。

## 文档地图

| 类别 | 文档 | 作用 | 何时读 |
|---|---|---|---|
| 规范 | [MVP 架构基线](design/2026-10-29-conex-private-platform.md) | 当前定位、SICP 模型、对象/事件、所有权、数据边界和 A01–A16 | 新设计与实施前必读 |
| 计划 | [MVP 实施总计划](plans/2026-10-29-conex-private-mvp.md) | 本轮迁移盘点、完整工作包、依赖与验收覆盖 | 了解全局顺序与文件责任 |
| 计划 | [首批 P0 执行计划](plans/2026-10-29-conex-mvp-p0.md) | 领域/网络契约冻结、真实协议探针、进入编码门槛 | 开始本轮实施准备 |
| 历史设计 | [上一版架构](design/2026-10-17-conex-architecture.md) | 2026-10-17 方向和 S1–S5 切片 | 追溯既有决策 |
| 历史盘点 | [旧迁移盘点](design/2026-10-17-conex-migration-assessment.md) | 当时的静态源码证据，缺口可能已由 S1/S2 填补 | 追溯迁移，不能据此重做已交付功能 |
| 契约 | [S1 双向能力与状态订阅](contracts/s1-capabilities.md) | 通用注册、调用、取消、状态快照与更新、限额和验收场景 | S1 实施、对接与验收 |
| 历史计划 | [S1 实施计划](plans/2026-10-17-conex-s1.md) | 九个任务、A01–A12 覆盖及原 Paca 编号；已交付证据另见验证记录 | 追溯 S1 实施 |
| 证据 | [S1 验证记录](verification/s1-capabilities.md) | 真实命令、A01–A12 跨进程与 SUPPORT 行为证据、总门禁状态 | 验收与排障 |
| 操作 | [S1 运行手册](runbooks/s1-capabilities.md) | TS/Rust 三进程样例、静态批准、HTTP 限制、取消和未知结果 | 本机联调与 SDK 接入 |
| 契约 | [S2 插件运行时与能力组合](contracts/s2-plugins.md) | 插件/实例、本地 DI、显式远程绑定、派生模块、停止释放与 S2-A01…A08 | S2 实施、对接与验收 |
| 证据 | [S2 验证记录](verification/s2-plugins.md) | s2-plugins e2e 实际输出、总门禁状态、遗留边界 | 验收与排障 |
| 操作 | [S2 运行手册](runbooks/s2-plugins.md) | 双实例插件样例三进程、组合拒绝与新代次排障 | 本机联调与插件接入 |
| 历史设计 | [旧设计](design/2026-09-14-conex-design.md) | 现有实现的设计依据与历史阶段验收 | 追溯既有 wire 与实现决定时 |
| 契约 | [P0 wire 契约](contracts/p0-wire.md) | `conex-jsonrpc2-http` 信封、参数、错误码、HTTP 行为 | 实现或对接 wire 时 |
| 历史计划 | [路线图](plans/2026-09-15-conex-roadmap.md) | 旧 P0–P4 阶段、依赖与工作包 | 追溯历史顺序 |
| 历史计划 | [P1 执行计划](plans/2026-09-15-conex-p1.md) | 旧 P1 范围、P1-01 细化与里程碑 | 追溯既有流与会话机制 |
| 历史计划 | [中心服务与交互落地页](plans/2026-09-20-conex-connected-landing.md) | 已实现的中心、反连 Agent、浏览器接入与路由范围 | 维护对应既有实现 |
| 历史计划 | [多主机数据与公共内容页面](plans/2026-10-01-conex-multi-host-content.md) | 原 M0–M6 内容能力工作包与当时状态 | 维护既有内容能力，不作为私有 MVP 前置 |
| 历史计划 | [文件传输、预览与剪贴板](plans/2026-10-13-conex-share-transfer.md) | 原文件场景工作包，范围不因新定位自动删除 | 维护对应能力时单独安排 |
| 证据 | [多主机内容验证记录](verification/multi-host-content.md) | M0–M5 实际交付、测试文件索引、已知缺口（M4/M6 未开始、fmt 全仓债） | 验收、排障、追溯契约变更 |
| 契约 | [Connected landing](contracts/connected-landing.md) | 中心 Host、浏览器会话/ticket、Agent 反连与只读端点目录的 L01 冻结契约 | 实施 connected landing 或校验信任边界时 |
| 计划 | [P0 执行计划（归档）](plans/archive/2026-09-15-conex-p0.md) | 已完成的 P0 计划：冻结决定、任务→提交→证据、遗留缺口 | 追溯 P0 决策 |
| 证据 | [P0 验证记录](verification/p0.md) | 环境、命令、结果、逐任务已知缺口 | 验收、排障、审计 |
| 操作 | [P0 运行手册](runbooks/p0.md) | 生成、启动、入站 token / 出站凭据、SDK 调用 | 部署与联调 |
| 操作 | [Connected landing 运行手册](runbooks/connected-landing.md) | `cargo xtask landing-demo` 三进程、token 三元组、docker-compose、日志、门禁 | 部署 / 演示 / 联调多 Agent 场景 |
| 操作 | [Dokploy 部署运行手册](runbooks/dokploy.md) | 公开演示单容器形态与多主机生产形态、两个明文开关的区别、web_origin、故障排查 | 把落地页推到 Dokploy 时 |
| 证据 | [落地页验证记录](verification/landing-page.md) | 客户端契约、推送字节预算下溢与死链残留、文本预览乱码／后台标签页不轮询文件／撤回先删后校验三个缺陷、一 cookie 挤成两标签页的根因修复与 28 项浏览器验收（含提问-回答往返）、上一版部署故障根因 | 验收、追溯本轮修复时 |
| 证据 | [Connected landing 验证记录](verification/connected-landing.md) | 两条 e2e + Rust 单测 + bun 测试实际输出、未验证项 | 验收、排障 |

## 阅读顺序

1. **了解当前方向**：根 [README](../README.md) → [MVP 架构基线](design/2026-10-29-conex-private-platform.md) → [实施总计划](plans/2026-10-29-conex-private-mvp.md) → [首批 P0 计划](plans/2026-10-29-conex-mvp-p0.md)。
2. **维护现有实现**：对应 wire 契约 → 历史设计/验证记录 → 运行手册；迁移前不把新目标当作已生效 wire。
3. **验收与审计**：新 MVP 按架构 A01–A16 和工作包实际命令核对；旧阶段证据单独保留，静态盘点和元数据握手不替代运行证据。

## 文档规则

- 结构类型只有一个源：`schema/conex/*.proto`；Rust/TS 类型与 JSON Schema 都是生成产物，禁止手改。权威严格解码点是 `MethodContract.prepare`/`validate_output`。
- **设计是规范，计划是执行范围与顺序，验证记录只写实际执行并通过的结果**；未实现能力必须明确标注，不把计划的 "Expected" 复制成结果。
- 已完成的阶段计划移入 `plans/archive/`，只保留冻结决定与证据索引；原全文见对应完成提交（P0 为 `e649ded`）。
- 文档命名：设计/专项规范用 `<日期>-<主题>.md`；阶段计划用 `<日期>-conex-<阶段>.md`。
