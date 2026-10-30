# Conex 私有分布式平台 MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` or `executing-plans` to implement approved work packages. Track execution with checkboxes; read this plan and the current architecture before changing code.

**Goal:** 交付单用户私有平台：私网节点主动接入公网，支持获准直连/转发；OMP 与 Codex 的真实会话、流式、取消与人工交互；Web 图谱/对话/服务器视图与最小 Emacs 原生客户端；模块独立运行且可组合。

**Architecture:** 复用现有协议、Host 路由、TS/Rust SDK 与插件依赖机制。拆分节点本地实例、中心发布与连接的生命周期；领域模型、持久事件和授权独立于前端及后端适配器。中心不默认存完整对话，节点保持自治。

**Tech Stack:** Rust/Tokio、现有 protobuf/JSON WSS/类型生成链、SQLite、React/Bun、Emacs Lisp；OMP RPC 与 Codex app-server。不强制引入 Astro，不新建通用 DSL 或分布式数据库。

---

## 1. 状态、权威和执行粒度

- 日期：2026-10-29。行为权威：[MVP 架构基线](../design/2026-10-29-conex-private-platform.md)。P0协议调查、P1领域存储和P2自治节点已执行；完整产品仍未交付。
- 本文是**完整交付的工作包与依赖总计划**；实际记录见[P0协议证据](../verification/agent-protocol-probes.md)、[P1实施记录](../verification/agent-domain-p1.md)、[P2实施记录](../verification/private-runtime-p2.md)。P3/P4进入编码前仍须文件级计划，[P5私有访问](2026-10-29-conex-mvp-p5.md)已有执行计划。
- `MVP-P0` 至 `MVP-P9` 是本轮编号，不复用历史 P0/P1/S1/S2 或 Paca CNX 编号。旧阶段未完成项不自动成为前置条件。
- P0/P1/P2已有实际证据；P3–P9尚未交付。路径/命令标“目标/待实现”的不表示已存在或通过，产品验收仍为A01–A16。
- 本轮不提交、不推送、不部署、不发布、不修改Paca；P2按已批准范围迁移SDK默认生命周期。不得改用户凭据、现有会话或删除业务数据。

## 2. 当前实现盘点与迁移处置

本轮已用 codebase-memory-mcp 建立/查询 Conex 索引，并交叉阅读以下文件。这里的“已有”是源码机制，不等于新产品验收。

| 当前文件/机制 | 处置 | 不可直接沿用的假设 |
|---|---|---|
| `crates/conex-host/src/capability/registry.rs` | 复用静态批准、代次与目录投影 | 注册依赖中心，不是自治节点本地授权 |
| `crates/conex-host/src/capability/calls.rs` | 保留固定目标、取消、预算、未知结果与不自动重放 | 仅中心转发路径，不能直接当作直连执行入口 |
| `crates/conex-host/src/capability/state.rs` | 保留快照顺序、有界队列、清理规则 | 完整状态更新不等于消息增量与持久事件补读 |
| `sdk/typescript/src/plugin.ts`、`crates/conex-sdk/src/plugin.rs` | P2已保留显式依赖/本地DI/派生能力并迁移生命周期 | 断线只影响发布，显式stop才清理本地业务；不再使用broken销毁默认语义 |
| `crates/conex-agent/src/runtime.rs`、`config.rs`、`main.rs` | 演进为现有节点程序的自治运行路径 | 不把当前反连程序误叫成 Agent 领域；不另造第二套默认节点二进制 |
| `sdk/typescript/src/ws-client.ts`、`s1-wire.ts`、`capability.ts` | 复用通信、严格边界和请求关联 | 连接恢复不等于命令可以重发；中心上下文不能直接用于节点直连 |
| `crates/conex-sdk/src/client.rs` | 同步必要协议与验证变化 | Rust/TS 均不可手工接受新字段而让另一端静默忽略 |
| `schema/conex/*.proto`、`xtask/src/generate.rs`、`crates/conex-proto/src/{wire,validation,capability}.rs` | 保持唯一类型源与生成链 | 不手改生成物；旧 `agent.proto`/`session.proto` 是已有运行协议，不覆盖成新领域模型 |
| `web/src/app.tsx`、`hooks/use-hello-page.ts`、`components/` | 复用 React/Bun、通用控件 | 不在旧大 hook 里加入 OMP/Codex 专用分支；私有入口不能沿用匿名访客默认 |
| `xtask/src/{main,e2e,check,s1_e2e,s2_e2e}.rs` | 延续真实进程验收与总门禁 | 旧结果不是 MVP A01–A16 的结果；不删除旧门禁换绿 |

P0真实后端交互/取消/恢复调查见[专项证据](../verification/agent-protocol-probes.md)；P1已有SQLite/跨语言恢复smoke，P2已有真实Host/Node/自有及外部进程验收。它们不替代P3/P4 Adapter或公网产品验收。历史S2三进程证据见[原记录](../verification/s2-plugins.md)，迁移后结果见[P2](../verification/private-runtime-p2.md)。

### 2.1 清理与兼容边界

- 实际切换前列出受影响调用方、配置、样例、测试和数据迁移；导出符号先查 LSP references，函数/类/方法先做 graph inbound impact。HIGH/CRITICAL 先说明风险和保护证据。
- 目标模型可新增独立领域类型，但不建立同一行为的 v1/v2 平行默认实现、别名或永久双路由。
- 旧 wire 的行为变化必须显式协商/迁移；不通过放宽严格解码偷偷兼容。
- 历史文件共享及存储数据不因新定位删除。旧页面切换以私有新入口真实可用和调用方迁移为条件，必要的独立演示用途需明确标注，不能作为私有产品匿名后门。
- 不预建空 crate、占位 Adapter 或未连通的 UI。新目录只在对应真实实现开始时创建。

## 3. 文件责任布局

以下是后续实施目标；P0 完成后，详细计划可在同一责任边界内调整内部文件，不借机改变已确认范围。

| 边界 | 现有入口 | 拟新增路径与责任 |
|---|---|---|
| 领域结构源 | `schema/conex/`、`crates/conex-proto/src/` | `schema/conex/agent_domain.proto`（Agent/会话/执行/交互/Workspace）；`entities.proto`（实体关系/标注/指标）；`events.proto`（快照游标/事件补读）；`runtime_access.proto`（路径与授权上下文） |
| 领域行为与存储 | 现有 proto 生成类型 | `crates/conex-agent-domain/`：`src/model.rs`、`projection.rs`、`store.rs`、`lib.rs`；业务状态、原子转换与 SQLite，不依赖 Host 或某个 Adapter |
| 节点装配 | `crates/conex-agent/src/{main,config,runtime}.rs` | `node.rs`（生命周期）、`publication.rs`（中心发布）、`local_api.rs`（本地/直连入口）、`access.rs`（节点授权）；在现有二进制内装配 |
| 应用与组合 | TS/Rust `plugin`、节点配置 | `crates/conex-agent/src/application.rs`（校验/装配/所有权）；复用插件依赖机制，不远程执行任意下载代码 |
| Agent 适配器 | 节点运行时 | `crates/conex-agent/src/adapters/{mod,omp,codex}.rs`；隔离进程/连接、协议转换、能力声明 |
| 指标与关系 | 领域 store、节点配置 | `crates/conex-agent/src/modules/{mod,metrics,relations,environment}.rs`；真实指标、关系投影、组合摘要 |
| 协调端私有入口 | `crates/conex-host/src/{config,auth,serve,http,ws_transport,broker}.rs` | `private_access.rs`（接入及路由策略）、`private_store.rs`（配置/目录/标注）；扩展现有入口而非复制完整 Host |
| SDK | TS/Rust 现有 client、wire、capability | `sdk/typescript/src/agent-domain.ts`、`events.ts`；Rust 对应领域消费入口随实际需要接入 `conex-sdk` |
| Web | `web/src/app.tsx`、通用组件 | `web/src/domain/client.ts`、`hooks/use-agent-session.ts`、`views/{agents,conversation,graph,servers}.tsx` |
| Emacs | 无客户端 | `sdk/emacs/conex.el`、`conex-transport.el`、`conex-tests.el`、`README.md`；原生 UI 与独立传输层 |
| 运行验收 | `xtask/src/` | `mvp_e2e.rs`、`agent_probe.rs`；`examples/mvp/`（非秘密配置）、`docs/runbooks/private-mvp.md`；完成时新增实际证据页 |

SQLite 在领域存储和协调端配置存储分别落地。实现前检查现有依赖并选一个受维护的 Rust SQLite 驱动；不实现 SQL 引擎，不把阻塞数据库操作放在异步执行器线程上。驱动与 Emacs WebSocket 依赖须在各自详细计划中锁定版本和许可；这是接口边界后的选型，不是无限延期的占位实现。

## 4. 工作包与依赖

| 包 | 可验收交付 | 前置 | 主要文件责任 | 架构验收 |
|---|---|---|---|---|
| MVP-P0 | 领域/事件/网络契约、适配器能力差异、可复现实机探针和后续文件级计划输入 | 本文与基线 | 新专项契约、探针、证据 | 为 A01–A16 固定操作与判据 |
| MVP-P1 | 领域状态、SQLite、会话串行准入、事件快照/补读、审批一次性转换、TS 领域绑定 | P0 | proto、agent-domain、SDK、生成链 | A07/A11/A12/A15/A16 的行为边界 |
| MVP-P2 | 自治节点、应用包装配、Scope、独立发布/连接生命周期与本地入口 | P1 | conex-agent、双 SDK plugin、配置/样例 | A04/A13，提供所有模块宿主 |
| MVP-P3 | OMP RPC 完整交互适配器 | P2 | adapters/omp、节点会话映射 | A05，补充 A07/A09/A12/A13 |
| MVP-P4 | Codex app-server 完整交互适配器 | P2 | adapters/codex、节点会话映射 | A06，补充 A07/A09/A12/A13 |
| MVP-P5 | 私有认证、节点侧授权、直连/转发策略、主动 WSS 接入与重连 | P2 | host 私有入口、node access/local_api/publication、wire/SDK | A01/A02/A03/A04/A12/A15/A16 |
| MVP-P6 | 服务器指标、实体关系/Tags/Workspace 投影与查询 | P2 | modules/metrics、relations、领域查询 | A09，提供 A10/A14 输入 |
| MVP-P7 | Web 领域工作台：列表、对话、审批、图谱、服务器 | P1 接口；完成需 P3/P4/P5/P6 | web、TS domain client | A08/A09/A10/A11/A15 |
| MVP-P8 | Emacs 原生列表、对话、流式、取消、审批、Workspace | P1 接口；完成需 P3/P4/P5 | sdk/emacs | A07/A08/A11/A13 |
| MVP-P9 | 组合摘要、两机公网接入、双客户端与故障全链路验收、私有启动默认切换 | P3–P8 | environment、examples、xtask、runbooks、入口迁移 | A01–A16 全覆盖 |

P3 与 P4 在 P2 接口稳定后可并行；P5/P6 也可独立推进。P7/P8 可在 P1 协议冻结后开发客户端，但不能用 mock 界面宣称交付。共享 proto、wire、生成物、节点配置和总门禁由集成所有者统一修改/生成，避免多代理覆盖。

## 5. 每包的执行与可观察退出条件

### MVP-P0：先冻结真实边界

- [x] 版本/schema/元数据和必要真实会话、取消、交互、恢复调查；worker记录、主会话补证、失败和未知分列，不以文档充运行通过。
- [x] 固定身份/独立代际、busy/幂等、发送边界/unknown、审批CAS与持久游标；未发送accepted不形成幽灵槽。
- [x] 固定direct-only传播/无中心正文、Ed25519授权/独立nodeEpoch、本地持久撤权/离线窗口与有界保留。
- [x] P1/P2/P5文件级计划已落盘，引用专项契约；公共proto/wire未提前扩充。

退出：三个专项契约可实现、适配器差异表有证据、未知项有可重复探针与明确产品处理；不能把缺失后端信息伪造成统一能力。

### MVP-P1：领域与恢复

- [x] 在唯一结构源新增领域/事件类型，连接严格解码、方法契约和生成工具；同步 Rust/TS，不手改生成物。
- [x] 实现 Conversation 与 Execution 分离，利用数据库原子准入保证同会话只有一个顶层执行；未知执行不释放成空闲。
- [x] 实现交互请求 CAS、提交幂等键与后端映射，重启后先核对未确定状态，不重新提交操作。真实后端ID映射写入随P3/P4，P1不伪造映射。
- [x] SQLite 事务同时写投影与事件位置；快照和续接共享一致边界。有界保留后旧游标明确要求重同步。
- [x] 建立最小行为回归：并发提交 busy、双审批仅一次、重启恢复未知、增量去重/缺口、清理不破坏在用游标语义。

实际运行与偏差见[P1实施记录](../verification/agent-domain-p1.md)；本轮P2总门禁也保留领域/生成链回归。

退出：同一真实领域存储被独立消费者观察；测试之外有一次重启与恢复 smoke，不需要模型。

### MVP-P2：自治与应用装配

- [x] 将本地实例管理从中心连接管理剥离；失去中心只清理发布/远程绑定，不关闭自有模块进程。实际Agent后端随P3/P4安装。
- [x] 读取应用包，验证允许的模块实现、配置、显式依赖/环与权限要求；启动不下载或执行任意插件代码。
- [x] 实现自有实例和外部引用的所有权清单、截止时间停止、逆序清理及失败回滚；模块可独立部署。
- [x] 双 SDK plugin 生命周期变更已迁移 S2 样例/调用方/行为测试，删除“断线即业务销毁”旧默认语义；不保留第二个自治 Instance。
- [x] 启动节点和真实长运行子进程，断开/重启中心并观察子进程仍运行；显式停止节点后只回收自有进程。

已通过：Rust/TS plugin回归、`s2-plugins`、新增`private-runtime`及包含它的`cargo xtask check`。实际断中心、未知资源不冲突重启、清理失败和入库部署样例结果见[P2实施记录](../verification/private-runtime-p2.md)。

退出：应用包可启动、停止、失败回滚；无跨实例误杀或中心断线误停。

### MVP-P3：OMP 适配器

- [ ] 使用当前安装版本的 RPC schema，处理 ready、响应关联、消息/工具事件、prompt_result、session_settled 与 UI 请求。
- [ ] 节点限定会话目录/后端路径映射；不同并发会话使用明确隔离容器，不抢占活动会话，不编辑正在运行的历史文件。
- [ ] 实现创建、继续、历史、发送、流式、执行状态、取消与交互回答；子代理通过后端真实 API 读取并关联。
- [ ] 同时用 Web/Emacs 之外的协议消费者验证一次真实模型调用、审批拒绝与取消；不把 abort 响应当作最终终止。
- [ ] 已有 TUI 只做独立附着调查，结果不阻塞已批准 RPC 主路径；界面明确区分可控、只读、不可附着。

运行：新增 `agent-omp-live` 验收入口，必须使用本机真实模型配置和隔离 Workspace；若需要凭据或额度，明确记录缺少前提，不能用 fixture 顶替 live 通过。

退出：A05 真实通过，证明会话持续、后台任务状态与取消终态准确。

### MVP-P4：Codex 适配器

- [ ] 从实际版本生成 schema，按 initialize 后的 thread/turn/item 协议实现会话及事件，避免照最新文档盲接实验字段。
- [ ] 自有 stdio/app-server 与连接外部 socket/server 显式区分，限定允许地址和凭据来源；不把外部服务当自有进程 kill。
- [ ] 映射消息、工具活动、终态、审批与用户输入；未知 server request 拒绝/报告，不无限等待或自动批准。
- [ ] 协商实验关系能力；不可查询的 subagent 信息标记未知。验证中途订阅是否可补读，而不假设重新 resume 会重发全部 delta。
- [ ] 用真实后端观察创建/继续、历史、流式、取消、审批拒绝及外部服务存活。

运行：目标 `agent-codex-live` 入口；先运行已有 `codex --version` 与 `codex app-server --help`，具体 API 从本机 schema 获取。live 与离线解析测试分别报告。

退出：A06 真实通过，连接断开或关闭适配器不误杀外部 app-server。

### MVP-P5：私有访问与网络

- [ ] 私有产品入口默认拒绝匿名；用户客户端、节点接入和模块操作分别验证。凭据不出现在 URL、目录和日志。
- [ ] 节点执行本地授权校验，直连与转发不能自报 Principal；授权绑定目标/操作/路径/有效期/代际，轮换不靠进程永久重启。
- [ ] 节点主动 WSS 出站，中心只转发批准能力；实现已知地址直连，不做任意 URL 代理或自动打洞。
- [ ] direct-only 拒绝经中心调用、订阅和同步正文；派生导出不能绕过来源策略。中心重放缓存不落对话正文。
- [ ] 实现断线重新认证与发布，不重发未确定副作用调用；中心停止时已有直连继续到授权到期。

运行：新增 `private-network` 套件验证 loopback/局域网 TLS 链路、假身份/过期/换代/错误路由/无效凭据；P9 必须另做真实公网入口与私网出站，不以本机网络替代。

退出：A01/A03/A04/A12/A16 的正负路径明确；中心磁盘和日志敏感 canary 扫描无泄漏。远程公网部署需另行授权，不在本次规划中执行。

### MVP-P6：指标与关系

- [ ] Linux 采集 CPU、内存、磁盘真实数据并携带时间与质量；CPU 使用率按采样差计算，不把累计计数当比例。
- [ ] 关系具有源、实体稳定引用、来源及有效状态；Workspace 绑定节点身份，Tags 不充当权限。
- [ ] 支持描述关系和能力绑定分别查询；用户删除关系不取消后端执行。
- [ ] 先快照后更新，节点断开投影过期而非把后端业务强制改 failed。

运行：真实节点采样，与本机 OS 数据源核对量纲、时间和边界；一次断连/重连 smoke。新增 `private-entities` 套件只覆盖可见业务边界，不测试简单字段转发。

退出：A09 所需实体/关系可从独立消费者查询，供图谱和组合使用。

### MVP-P7：Web

- [ ] 在现有 React/Bun 应用内建立领域客户端和独立会话 hook，不能把 OMP/Codex 判断塞进 use-hello-page。
- [ ] 真实 Agent/会话列表，Workspace 选择，发送/流式/取消，忙碌/过期/未知/不支持状态，审批和输入。
- [ ] 图谱与对话共用实体 ID，展示 Agent/会话/Workspace/Tags/status/subagent 关联；关系和可执行绑定用不同操作表达。
- [ ] 服务器列表与详情展示真实采样和过期；断线/恢复保持可理解，不用演示数据填空。
- [ ] 浏览器验证实际登录、操作和图谱跳转，含窄屏基本可用、键盘访问与错误状态；关闭页面不停止 Agent。

运行：现有 `bun run typecheck` 与 `bun run build:web`（实现时一次集成）；实际浏览器打开私有入口，保留截图及操作证据。不为 UI 引入仅验证 mock 回显的测试。

退出：A08/A10 的 Web 部分真实通过，前端产物和正在运行的 Host 源码一致。

### MVP-P8：Emacs

- [ ] 原生 Emacs Lisp 客户端使用同一认证/领域协议；WebSocket 复用受维护库，不自己实现帧协议。依赖在包头声明并锁定验收版本。
- [ ] 独立 transport 层处理响应关联、订阅、游标与错误；buffer 层处理列表、会话、流式插入、Workspace 及派生信息。
- [ ] 实现取消、审批/输入提示、过期请求拒绝、busy/未知状态；不要求 EAF/ETAF，不关闭 buffer 就 kill 远端执行。
- [ ] 最小 ERT 行为回归覆盖分片消息解析、重连续接去重、已处理审批不重复提交，避免仅测试字符串布局。
- [ ] 与 Web 同时打开同一会话，用真实 Agent 验证流式同步和审批竞答；保留实际 Emacs 交互录屏/截图或终端证据。

运行：目标 `emacs -Q --batch -L sdk/emacs -l sdk/emacs/conex-tests.el -f ert-run-tests-batch-and-exit`；命令执行时需提供已声明依赖的 load-path。batch 测试不替代真实 Emacs buffer 操作。

退出：A08 的 Emacs 范围全部实现，两个客户端不各自管理后端进程。

### MVP-P9：组合与全链路切换

- [ ] 工作环境摘要模块显式绑定真实 Agent 状态与指标，提供自己的普通能力；另一个消费者读取/订阅，证明组合闭包。
- [ ] `examples/mvp/` 给出中心、两节点、应用包的非秘密配置与启动手册；隔离凭据与数据目录，明确直接/转发策略。
- [ ] 在两台真实机器和公网入口运行 A01–A16；包含停中心、断私网、重启节点、慢客户端、双端审批和外部服务存活。
- [ ] 所有适配器 live 场景与受限网络场景分别报告，不把离线 fixture/本机 TLS/元数据握手视为替代证据。
- [ ] 新默认入口完成后迁移全部受影响配置、脚本、客户端与文档，移除被替代默认路径；不删除无关历史数据或功能。
- [ ] 完成真实证据页 `docs/verification/private-mvp.md`、手册和 README 状态；失败/未跑项目明确保留，不能关闭总验收。

运行：目标 `cargo xtask e2e --suite private-mvp`、`cargo xtask check`，以及获准的公网/浏览器/Emacs live 场景。模型和外部机器前提必须真实具备，否则只能完成对应包的可达部分，不能宣称 MVP 交付。

## 6. 验收覆盖及交付证据

| 基线 ID | 主责任 | 最终证据 |
|---|---|---|
| A01 私有认证 | P5 | 匿名/错误凭据/伪造主体拒绝 |
| A02 公网接私网 | P5/P9 | 两真实节点出站，公网入口实际转发 |
| A03 两路径及限制 | P5 | 同契约直连/转发、direct-only 三类拒绝 |
| A04 自治与换代 | P2/P5 | 停中心仍运行，恢复后旧绑定失效 |
| A05 OMP | P3 | 真模型会话、流式、取消、交互 |
| A06 Codex | P4 | 真模型会话，外部 app-server 未误停 |
| A07 串行与审批 | P1/P3/P4/P8 | busy、双端 CAS、未知执行不误空闲 |
| A08 双客户端 | P7/P8 | 实际浏览器和 Emacs 操作同会话 |
| A09 实体关系 | P3/P4/P6 | 来源与未知边界、Workspace/Tags/subagent |
| A10 三种视图 | P7 | 图谱/对话/服务器同身份联动 |
| A11 恢复 | P1/P5/P7/P8 | 重复去重、缺口/越界明确 resync |
| A12 取消与未知 | P1/P3/P4/P5 | 终态竞态及无自动重放 |
| A13 所有权 | P2/P3/P4 | 关闭客户端不停止业务，清理只作用自有资源 |
| A14 组合 | P2/P6/P9 | 真实输入派生能力被第三消费者使用 |
| A15 持久与隐私 | P1/P5/P9 | 重启恢复、正文/凭据 canary 无中心落盘 |
| A16 有界/撤权 | P1/P5/P9 | 慢消费者、授权到期与本地撤权 |

## 7. 通用检查与实施门槛

- [ ] 任何代码修改前重新确认索引状态并做 graph impact；公开符号查 LSP references。当前文档同步不修改函数或类。
- [ ] 每包开始先读取本包契约与详细计划。永久测试只覆盖消费者可见边界/竞态/安全不变量，不写字段复制、mock 回显或源码文案测试。
- [ ] 使用真实路径 smoke；fixture 仅用于确定性边界，不能代替模型、网络、浏览器或 Emacs live 证据。
- [ ] 并行工作期间不重复全仓 build/lint/format；集成所有者在完成后跑一次总门禁。失败来源与用户现有修改分开处理。
- [ ] 工作包完成时更新手册、变更口径与实际证据；不预先填通过结果，不留下脚手架、fake fallback 或可见假按钮。
- [ ] 提交需另外授权；提交前 fresh index + detect_changes，零变更但工作树有修改视为索引未看见，不能当作无影响。没有提交授权就交付文件与证据，不自行提交。

## 8. 现在的下一步

P0/P1/P2已按各分项证据交付；后续P3/P4进入编码前先确定适配器文件级计划，P5按[私有访问计划](2026-10-29-conex-mvp-p5.md)推进。进入代码修改前仍须影响分析及必要依赖授权；P3/P4必需live缺项不能因底座完成免除。不要先搭UI壳或部署公网；完整交付终点仍是A01–A16。
