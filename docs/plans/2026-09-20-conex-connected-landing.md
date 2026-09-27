# conex 中心服务与交互落地页 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` or `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户启动一个中心 Host 和多个私有侧 Agent 后，打开 Host 的落地页即可认证、完成真实握手、看到获准访问的服务端点，并通过中心 Host 调用异地 Agent 的只读能力。

**Architecture:** 采用用户确认的 A 拓扑：浏览器只连接中心 Host，Agent 主动反连中心 Host；浏览器不连接 Agent，也不接触 Agent 的本地路径和上游凭据。沿用 Registry / MethodContract / Host::invoke / Handler 的授权执行路径；静态配置决定哪些端点可以接入，在线连接决定端点当前能否调用。页面与 HTTP/WSS 同源部署，展示真实连接事实，不用模拟节点或假握手。

**Tech Stack:** Rust 2024、Tokio、Axum、现有 rustls / tokio-tungstenite / ring、prost/pbjson；浏览器使用 TypeScript、原生 DOM/CSS/SVG 和 Bun 构建，复用 `sdk/typescript`，不引入前端框架、图形库、数据库或消息队列。

**状态：** 计划，尚未实施。用户已确认 A 拓扑；本文中的新增接口、文件、命令和默认值均为实施约定，不代表现有能力。2026-09-20 的调查结果仅证明下述基线。

**权威上游：** [设计 v6](../design/2026-09-14-conex-design.md) §2、§4、§8、§9.3、§12；[路线图](2026-09-15-conex-roadmap.md)。沿用仓库现有 `docs/plans/`，不另建计划目录。

---

## 1. 交付范围与完成定义

### 1.1 必须交付

- 一个中心 Host，同端口提供页面、登录/ticket HTTP 入口和 `/wss`。
- 至少两个真实 Agent 进程，主动出站连接 Host；各自只开放管理员指定的目录。
- 浏览器完成 `hello → hello result → ready → ready result`，然后取得按当前身份过滤的端点目录。
- 页面显示当前浏览器、中心 Host、服务端点及所属 Agent；点击端点可执行 `source/list`、`source/read`、`source/search`。
- 请求确实经过中心 Host 转发，由选中的 Agent 执行，并沿原连接返回结果。
- Agent 停止、异常断开、重新上线时，目录和页面更新；其他 Agent 的调用不受影响。
- 认证失败、权限不足、端点离线、请求超时分别展示；不伪造在线、地域、延迟、调用成功或恢复成功。
- 提供可重复启动的本机多进程演示，以及真实跨主机部署与验收步骤。

### 1.2 本计划不做

- 浏览器同时直连多个 Host、跨 Host 联邦、HA 或运行会话跨进程恢复。
- mDNS、公共协调者、DHT、NAT 打洞、P2P、全网自动发现。
- ACP/MCP、shell、任意命令执行、写文件、blob 上传的页面入口。
- 多租户账户管理后台、公开匿名演示站、自动部署到公网、完整 OIDC 重定向登录。
- 为展示添加专门的 ping RPC、历史指标平台、持久事件库或地球动画。

这里的“自动出现”是**预授权端点的 Agent 建连并注册后，目录自动更新**，不是发现未知机器。这里的“异地”是部署事实，地域标签由管理员提供，不能由本机两个进程伪造。

### 1.3 已验证基线

| 项目 | 2026-09-20 证据 | 实施含义 |
|---|---|---|
| 构建 | `cargo build -p conex-host --locked` 成功 | 可以复用当前 Host |
| HTTP | 真实 Host 的 hello 成功，source/read 返回 `hello conex\n` | 只读契约和执行主干存在 |
| WS | 带 bearer 的非浏览器客户端完成 hello/ready | 复用已有 WS 引导和编码 |
| 浏览器 ticket | POST /tickets = 201；仅带 ticket 的 WS upgrade = 401 | ticket 尚未接入升级认证 |
| 页面/跨域 | GET / = 404；OPTIONS /rpc = 405，无允许来源响应头 | 补页面；默认同源，不开放任意 CORS |
| WSS 能力 | hello 返回了客户端传入的 `ui/probe` | 必须纠正回显，不能据此展示服务能力 |
| Agent | main.rs 的 register/heartbeat/resolve 只输出 JSON | 必须交付真实反连和调用，不以现有台账代替 |

上次探针使用本机显式 loopback 明文；不构成公网 TLS、真实浏览器登录或跨地域部署证明。历史 P1 文档有互相冲突的完成表述，执行时以源码、实际命令和新增验收记录为准。

## 2. 固定设计决策

### 2.1 拓扑与路由

```text
浏览器 -- HTTPS / WSS --> 中心 Host
                             ^
                             | Agent 主动建立并保持 WSS
                         Agent A / Agent B
                             |
                         受限本地目录

调用方向：浏览器 → Host 授权执行 → 已认证 Agent Link → 本地 provider → 原路返回
```

Host 静态登记 endpointId、tenantId、允许的 agentId、agent 身份、能力和用户资源策略。Agent 注册只能激活既有授权范围，不得创建任意端点、扩大能力或替换其他 Agent。

远端端点仍使用现有 `source/*` MethodContract。Host 装配一个实现现有 `Handler` 的远端执行器，调用连接表；不能在 `broker.rs` 提前返回以绕过 Host::invoke 的授权、限额、输出校验和审计。Registry 启动后不必随连接增删：路由静态，连接动态。

### 2.2 页面与接入身份

首轮采用**管理员签发的专用 UI 访问凭据换取 Web 登录会话**，不把尚未闭环的 OIDC 标为可用：

1. 页面使用同源 `POST /web/login`，通过 Authorization 头提交 UI 专用凭据。
2. Host 复用静态 bearer 校验与主体映射，凭据必须具有 ui 角色；建立随机 Web 会话。
3. 浏览器获得 HttpOnly / SameSite=Strict cookie；HTTPS 使用 Secure，只有显式 loopback 开发模式允许非 Secure。
4. `/web/session` 返回当前主体显示信息及本会话 CSRF nonce；不返回长期凭据。页面认证后清空输入，凭据不存 localStorage、URL 或日志。
5. `/tickets` 从 Web 会话派生主体和目标，验证 Origin、CSRF，签发随机、30 秒、一次性 ui ticket。
6. 浏览器通过 `/wss?ticket=...` 建连；升级入口原子消费 ticket，并验证 Origin、目标 Host、会话仍有效和角色。日志脱敏整个 ticket 查询参数。
7. `POST /web/logout` 撤销会话、未消费票据及其活动 UI Link；同一身份的其他 Web 会话不被误注销。

Web 会话默认绝对有效期 8 小时，每主体最多 8 个、全局最多 1024 个；不以业务 Session ID 或 bindingId 充当登录 cookie。随机材料统一使用现有 ring 的系统随机源，至少 32 字节。票据每 Web 会话最多 8 个、全局最多 4096 个，签发时清理过期项；认证失败、超限必须返回明确状态，不退回匿名模式。

ui、agent、service 凭据明确区分，执行时迁移全部示例和测试中的 token 配置；agent 凭据不能登录页面，ui 凭据不能注册 Agent。UI Link 只允许目录及 `source/list|read|search`，逐次业务调用仍检查资源策略；其他现存 P1 方法不是页面可调用面的默认授权。

这与设计 §8.5 的“浏览器采用 OIDC”存在范围差异：任务 L01 必须在权威设计中明确新增“受控部署的静态 UI 凭据兑换模式”，保留 OIDC 为另一登录适配方式。禁止实现后才用文档解释绕过认证。公开匿名试用和完整 OIDC 均不在此计划的交付声明内。

### 2.3 能力、目录与展示状态

- WS hello 的服务端 provides 来自实际可达的方法注册及当前连接角色；硬 requires 不满足即拒绝。双方能力按方向检查，不能简单求交集，更不能回显客户端作为服务端能力。
- hello 表示协议层能力，不表示每个资源均获授权或每个远端端点在线。
- 新增 `endpoint/list`，只返回当前主体能看到的端点和操作范围，默认拒绝。列表从安装配置、统一策略与实时连接表投影；不得暴露根目录绝对路径、上游地址、凭据引用或秘密。
- 每个可见操作包含 method、root、subtree；部分目录授权不能因为用户无权访问资源根 `""` 而整端点消失，也不能把局部授权显示为全端点授权。
- 请求包含 limit（默认 50，最大 100）和可选 afterEndpointId；按 endpointId 稳定排序，返回 nextAfterEndpointId。连接/权限变化时刷新整批目录，不承诺跨页强一致快照。
- endpointId 必须与现有 Registry 的全局唯一规则一致；用户界面不通过拼接 tenant 名称另造 ID。

拟新增的目录响应语义如下；实际结构只定义于 `.proto`，TS/Rust/JSON Schema 全部生成：

```text
EndpointSummary {
  endpointId, displayName, providerId,
  attachmentKind: embedded | outbound_http | reverse_agent,
  agentId?, region?,
  connectionState: not_applicable | offline | ready,
  availableMethods[],
  authorizedScopes[{method, root, subtree}]
}
EndpointListResult { endpoints[], nextAfterEndpointId? }
```

region 是管理员标签。内嵌和 HTTP catalog 不因为“有配置”而显示在线，连接状态为 not_applicable，业务可用性由最近一次调用结果表达。Agent 的 ready 表示已认证、完成握手且注册获准，不代表文件一定可读。

不新增目录订阅协议：页面在线可见时每 3 秒获取目录，不重叠请求；页面隐藏时暂停，恢复可见时立即刷新。浏览器调用时间线只表示本客户端观察，不伪装为服务器内部 trace。

### 2.4 Agent 连接与请求关联

- Agent 是 WS 发起方：验证 Host TLS 身份后，带 agent 凭据 upgrade，发送 hello/ready，再注册获准端点。
- 私有侧只信任配置的 Host URL、CA 与 server name；默认 WSS，明文只允许显式 loopback 开发模式。禁止 insecure/跳过证书验证选项。
- Agent 注册映射到认证主体、tenant、agentId、端点清单和**当前真实 socket**。不再接受独立 HTTP agent/register 创建“在线”连接。
- 每次有效注册分配新的连接代次 generation；新连接替换旧连接时关闭旧 Link。旧 reader 退出不得清除新连接，旧响应不得完成新代次请求。
- Host → Agent 的调用使用现有 conex 请求/结果信封；复用 Request.context 中的端点与 plane，只传已校验的 source 输入和剩余超时，不传浏览器 bearer、Host 凭据或可由页面自报的主体。
- Agent 将中心端点映射到本地安装项，在本地再执行能力/资源边界校验，使用现有受限 fs provider；不是任意文件代理。
- 两端各只有一个 reader 和一个受控 writer。reader 必须分别分发请求、响应、通知及控制帧，不能等待某个业务调用结束才继续读 socket，否则会死锁。
- 主机侧待响应表按 `(connection generation, requestId)` 关联，收到未登记/重复/其他代次响应不得交付给调用方。断连、取消、deadline 后移除记录。
- 初始每 Link 在途请求上限 4，发送队列字节上限 8 MiB，帧上限 1 MiB；沿用已有 Limits，按编码后大小计费，不能只有无限通道外加计数。超过限制返回 quota_exceeded，不无限排队。
- 使用 WS 控制帧保活：每 20 秒 ping，40 秒无有效回应关闭；实际 socket 关闭立即标离线。Agent 心跳不能代替浏览器会话续租。
- Agent 断线重试间隔 1、2、4、8、16、30 秒封顶并加小范围抖动；重连必须重新认证/握手/注册，不把旧 ticket、旧 generation 或未完成调用当作可恢复状态。
- 页面不自动重放用户调用；断线中的只读调用明确失败，恢复连接后由用户重试。本计划不广告 Session/Stream 级无损恢复。

## 3. 页面信息架构

1. **首屏**：标题“把分散的服务，连接成可调用的能力”；登录/连接按钮；同源 Host 的显示名称。未认证不展示端点、节点计数和租户信息。
2. **连接阶段条**：认证 → 建立连接 → hello → ready → 就绪。`WebSocket.onopen` 只能点亮传输阶段，不能直接标就绪。
3. **拓扑区**：当前浏览器、中心 Host、按 Agent 分组的端点；内嵌/HTTP 端点单独分组。只绘制真实关系，离线线条不动画，不把一次轮询当作持续业务流。
4. **服务列表**：名称、接入方式、管理员地域标签、连接状态、可用操作；空状态区分“无授权端点”和“获准端点均离线”。
5. **交互面板**：列表/读取/搜索表单，默认从获准 root 开始；结果显示内容、来源、客户端往返耗时，CID/协议详情可展开。所有返回文本通过安全文本节点呈现，不执行 HTML/Markdown 内嵌脚本。
6. **事件时间线**：有界保留最近 200 条连接/握手/调用事件；超过上限淘汰最旧项；内容结果与元数据分开展示，导出功能不在本轮。
7. **失败与重连**：认证失效回到登录；连接异常可自动重新取票建链，但不重放调用；主动断开停止重连和目录轮询。
8. **响应式与可访问性**：桌面并列拓扑和面板；窄屏纵向排列、拓扑可横向滚动；按钮/表单可键盘操作、焦点可见，状态不只用颜色表达，遵守 prefers-reduced-motion。

不增加“体验演示服务”的死按钮。示例部署就是一套真实的受限环境；若没有公开演示地址，只展示本 Host 的连接入口。

## 4. 文件与职责

所有“新增”路径属于本计划的后续实现，当前只创建本计划文档。

| 边界 | 现有文件 | 新增文件 |
|---|---|---|
| 类型/契约 | `schema/conex/control.proto`、`schema/conex/rpc.proto`、生成目录 | `schema/conex/endpoint.proto`、`schema/conex/agent.proto` |
| 共享握手/安装 | `crates/conex-core/src/transport_ws/mod.rs`、`registry.rs`、`policy.rs` | 不新建框架 crate |
| Host 装配/认证 | `crates/conex-host/src/{config,auth,serve,http,tickets,agent,ws_transport,lib}.rs` | `crates/conex-host/src/web_auth.rs` |
| 反连路由与目录 | `crates/conex-host/src/serve.rs`、`broker.rs` | `crates/conex-host/src/remote.rs`、`catalog.rs` |
| Agent 进程 | `crates/conex-agent/src/main.rs`、`Cargo.toml` | `crates/conex-agent/src/config.rs`、`runtime.rs` |
| SDK | `sdk/typescript/src/client.ts`、`index.ts` | `sdk/typescript/src/ws-client.ts` |
| 页面与构建 | 根 `package.json`、`bun.lock` | `web/{package.json,tsconfig.json,build.ts,index.html}`、`web/src/{main.ts,view.ts,style.css}` |
| 静态页面入口 | `crates/conex-host/src/{http,config,serve,lib}.rs` | `crates/conex-host/src/web.rs` |
| 演示/门禁 | `xtask/src/{main,e2e,check}.rs` | `xtask/src/landing_demo.rs`、`examples/landing/{host,agent-a,agent-b}.toml` |
| 文档 | 设计 v6、路线图、文档导航 | `docs/contracts/connected-landing.md`、`docs/runbooks/connected-landing.md`、`docs/verification/connected-landing.md` |

源码模块按表归属，确需新增文件时必须有独立职责，不提前创建空目录或空 trait。Rust 依赖优先复用 workspace 及现有 host 的版本；页面不添加 UI 框架。

## 5. 工作包与依赖

| 编号 | 工作包 | 依赖 | 可观察交付 |
|---|---|---|---|
| L01 | 契约与配置冻结 | 无 | 可评审的类型、状态、信任边界及生成产物 |
| L02 | 真实能力握手与双向 WS 调度 | L01 | 两端可同时收发请求/响应，错误能力不能就绪 |
| L03 | 浏览器登录与 ticket 闭环 | L01、L02 | 原生浏览器可认证并完成握手 |
| L04 | Agent 实际反连与连接归属 | L01、L02 | 两个真实 Agent 注册，断开/重连代次正确 |
| L05 | 统一执行路径的远端调用 | L04 | Host 的 source 调用在指定 Agent 执行 |
| L06 | 按权限投影的端点目录 | L03、L05 | UI 主体只看到获准端点与资源范围 |
| L07 | 浏览器 SDK 与连接状态 | L02、L03、L06 | 页面使用稳定 API，不另造 wire |
| L08 | 可交互页面与同源托管 | L07 | 实际页面连接、列举、读取、搜索、断开 |
| L09 | 多进程与跨主机联合验收 | L04–L08 | 可重复演示并记录故障/隔离证据 |

顺序里程碑：M1 = L01–L03 浏览器真握手；M2 = L04–L06 两个 Agent 真调用；M3 = L07–L09 页面与部署闭环。M1/M2 是中间检查点，均不得作为整个需求完成。

默认串行推进。若执行时启用并行，先完成 L01/L02；L03 与 L04 可按 `web_auth.rs` / Agent runtime 分工，但 `config.rs`、`serve.rs`、`ws_transport.rs` 由一个集成负责人串行合入。并行子任务不运行全仓格式化、lint 或测试，统一集成后验证。

### L01 — 冻结契约、身份和配置

**修改/新增：** 表中类型/契约、Host/Agent 配置及权威设计；`docs/contracts/connected-landing.md`。

- [x] 更新设计 §8.5，记录 UI 静态凭据兑换模式及其和 OIDC 的边界；记录本轮只读远端执行、不提供会话恢复。
- [x] 以 `endpoint.proto` 定义 §2.3 的目录请求/响应、接入与连接状态枚举，以 `agent.proto` 定义注册/注册结果；枚举使用现有 proto 风格，时间/大整数沿用当前 wire 规则。
- [x] 扩展现有 `control.proto` 的 WS 协商类型，HTTP binding 与 WS link/negotiation 不混用；L01 冻结生成类型与向量，L02 负责迁移现有手写 hello/ready 结构和全部调用方，不保留第二套独立结构源。
- [x] Host 增加明确的 web origin/web root、token role、预授权 Agent/endpoint 配置；Agent 配置明确 host URL、TLS 信任、token env/file 引用、agentId 和本地 endpoint→目录映射。未知字段继续拒绝。
- [x] 将配置中的 source-remote 端点纳入受支持安装类型，禁止与内嵌端点冲突；注册方法必须是 source 三个只读方法的子集。
- [x] 运行生成命令并新增跨语言契约向量：空目录、局部资源授权、非法枚举、缺必填身份字段、重复端点、WS hello/ready；向量必须驱动真实编解码/校验，不只检查存在 expect 字段。

**命令：** `cargo xtask generate`；`cargo xtask generate --check`。

**通过条件：** Rust/TS/JSON Schema 来自同一 proto；配置正例可解析，未知角色/重复端点/跨租户 Agent 绑定被拒绝。生成产物检查应报告 up to date。

### L02 — 修正能力握手与 WS 双向调度

**修改：** `crates/conex-core/src/transport_ws/mod.rs`、`crates/conex-host/src/ws_transport.rs`；现有 `crates/conex-host/tests/wss_profiles.rs`。

- [ ] 先补失败用例：客户端声明 `ui/probe` 时服务端不得回显为自身能力；硬 requires 不满足不能完成握手。
- [ ] ServerHandshake 接收由装配/角色计算的真实服务端能力；生成随机 link/negotiation 标识，双方核对 profile、plane、协商 ID、能力摘要与上限后才就绪。迁移 JSON 与 protobuf 两种 Profile。
- [ ] 给现有握手循环加真实 10 秒 deadline，错误响应写出后再关闭；拒绝 ready 前的业务调用、重复 ready 和超大引导帧。
- [ ] 拆分单 reader/有界 writer 与 pending 关联表：请求交给业务执行，响应交给等待者；不能让现有 `broker_frame_from_message` 丢掉 response 类型。复用现有 Message/Request/Success/Failure，不增加私有信封。
- [ ] 集成 §2.4 的在途/字节限制和保活。关闭时完成所有等待者的失败结果，释放通道与 pending，不保留悬挂任务。

**命令：** `cargo build -p conex-host --locked`；`cargo test -p conex-host --test wss_profiles`。

**新增检查文件：** `crates/conex-host/tests/ws_duplex.rs`，执行 `cargo test -p conex-host --test ws_duplex`。

**必须断言的行为：** 双向相同 requestId 不串单；请求等待期间仍可处理响应；超过 deadline 后迟到结果不复活调用；断连所有等待者结束；慢接收方不能造成无界内存；原有两 Profile 调用仍成功。

### L03 — 浏览器登录、ticket 与退出

**修改/新增：** `web_auth.rs`、`auth.rs`、`tickets.rs`、`http.rs`、`ws_transport.rs`、`serve.rs`；`crates/conex-host/tests/web_auth.rs`。

- [ ] 先保存 ticket-only WS 401 的真实复现；新增一次消费/过期/跨 Origin/跨角色/注销后拒绝的行为用例。
- [ ] 实现 §2.2 的四个 Web 入口：`POST /web/login`、`GET /web/session`、`POST /web/logout`、既有 `POST /tickets`。状态修改要求精确允许的 Origin；持 cookie 的修改同时校验 CSRF nonce。
- [ ] 替换 ticket 的可预测 hash 拼接为系统随机值，绑定 Web 会话、主体、tenant、host、ui 角色、source/目录方法上限；不允许请求体改写主体或自行扩大授权。
- [ ] WS upgrade 接入 ticket 认证并原子消费；保留非浏览器 agent/service 的 bearer 入口，但角色边界统一执行，不允许 ticket/bearer 冲突时任意选一个。
- [ ] 将 UI 允许方法限制应用到入口实际分派路径；浏览器无法调用 agent 注册、operation、blob 等非本轮入口。
- [ ] 登出、会话到期或撤销时关闭所属 UI Link、拒绝在途后的新请求；不影响其他用户或会话。

**命令：** `cargo test -p conex-host --test web_auth`。

**真实冒烟：** 用原生浏览器 WebSocket API，不添加自定义 Authorization 请求头，执行登录→取票→hello/ready→退出；确认退出后不能用旧 cookie/ticket/连接发起新业务。此步骤不以 Bun/Rust WS 客户端代替浏览器。

### L04 — Agent 真反连与连接生命周期

**修改/新增：** `crates/conex-agent/src/{main,config,runtime}.rs`、Agent Cargo.toml、`crates/conex-host/src/{agent,remote,serve}.rs`。

- [ ] 用实际进程验证当前 CLI 不连 Host，再删除打印假 register/resolve 的行为，CLI 改为 `conex-agent <config.toml>`。
- [ ] 复用现有 TLS/WS 依赖，按 §2.4 实现主动连接、双向握手、注册、保活、重连；默认验证 CA/server name，env/file 秘密只在本进程解析。
- [ ] Host 的 AgentRegistry 与真实连接表使用同一身份绑定。HTTP 上的 agent/register 明确拒绝，WS 注册仅限 agent 角色，实际授权清单与声明逐项比对。
- [ ] 连接表按 agent 身份与 generation 管理；新连接替换、旧连接关闭、心跳超时必须原子处理，任何 cleanup 必须 compare generation 后删除。
- [ ] 实现静态 Agent 配置加载本地 fs provider，注册范围不能超过本地安装能力；不开放未授权路径和命令执行。

**新增检查：** `crates/conex-host/tests/agent_link.rs`。

**命令：** `cargo build -p conex-host -p conex-agent --locked`；`cargo test -p conex-host --test agent_link`。

**通过条件：** 两个独立 Agent 在线；错误 CA/主机名在发送注册与凭据之前失败；未知 Agent、冒用另一 Agent、注册越权端点均拒绝；旧连接退出不使新连接离线；断开被正确检测，重连不冒充 Session 恢复。

### L05 — 远端 source 调用纳入统一授权路径

**修改/新增：** `crates/conex-host/src/remote.rs`、`serve.rs`、`crates/conex-core/src/registry.rs`；复用 `conex-source` contracts 和 `conex-provider-fs`。

- [ ] 为远端端点构造现有 Route，handler 使用共享连接表。现有 FactoryFn 无法捕获连接表，因此只在 Registry 增加 `install_routes(installation, routes)`，将原 install 的路由校验/入表代码提取共用；原 install 仍调用 factory 后进入同一校验函数。禁止第二套宽松安装入口。
- [ ] 实现 RemoteHandler 的现有 Handler::execute：确认获准 agent 当前代次 ready；转换为标准请求，计算剩余 deadline，经有界连接发送并等待匹配响应。权限检查必须发生在发送前。
- [ ] Agent 仅接受已认证 Host Link 上的请求，校验 endpoint 映射与方法，再通过本地 Host/受限 provider 执行；本地身份是配置的 Host 调用主体，不相信报文自报用户拥有本机权限。
- [ ] Agent 使用标准成功/失败信封返回，Host 继续走既有输出校验与审计终结；端点离线返回 unavailable，不退回另一个同能力 Agent。
- [ ] 搜索/列表/读取的分页和错误语义复用现有契约。UI 不自动重放；待响应取消、超时、断连都释放 pending。

**新增检查：** `crates/conex-host/tests/remote_source.rs`。

**命令：** `cargo test -p conex-core`；`cargo test -p conex-host --test remote_source`。

**通过条件：** A/B 各自目录有不同内容，同一资源名读取结果正确；跨租户/未授权 root 请求在 Agent 收到业务请求前被拒绝；路径穿越在私有侧也拒绝；停 A 不影响 B；超时后迟到 A 响应不能交付给新调用。

### L06 — 端点目录与授权投影

**修改/新增：** `crates/conex-host/src/catalog.rs`、`serve.rs`、`broker.rs`、`crates/conex-core/src/policy.rs`；`crates/conex-host/tests/endpoint_catalog.rs`。

- [ ] 在现有 StaticPolicy 上提供可见资源范围投影，复用现有资源范围规范化和规则匹配；不维护独立的“页面 ACL”。最终执行继续以 authorize 为准。
- [ ] 实现 `endpoint/list` 的 MethodContract、严格解码与输出校验，注册到实际 RPC/WS 分派；只读目录权限不等于获得业务权限。
- [ ] 按 §2.3 合并安装元数据、可见 scopes 与实时连接；页面只能看到配置和授权都允许的条目，不接受 Agent 自报地理位置覆盖管理员标签。
- [ ] 覆盖稳定分页、上限、空列表、局部资源授权、跨租户隐藏、连接离线/恢复；返回体不得含本地绝对路径或秘密。

**命令：** `cargo test -p conex-host --test endpoint_catalog`。

**通过条件：** 两个用户看到不同端点/范围；只获准 `team/` 的用户仍能看到对应服务，默认 root 为 team；目录可见不使越权调用成功；Agent 状态变化出现在下一次查询。

### L07 — 浏览器 SDK

**修改/新增：** `sdk/typescript/src/ws-client.ts`、`client.ts`、`index.ts`；`sdk/typescript/tests/ws-client.test.ts`。

公开接口按以下调用形态实现；类型来自 L01 生成物，连接状态是客户端本地模型：

```typescript
const client = new ConexWsClient({ origin: location.origin });
const unsubscribe = client.onEvent(event => renderConnectionEvent(event));
await client.connect(); // 取票、hello、ready；ready result 后才 resolve
const page = await client.listEndpoints({ limit: 50 });
const result = await client.read(endpointId, { resourceId });
client.close(); // 停止重连，关闭 socket，结束 pending
unsubscribe();
```

`renderConnectionEvent` 是页面传入的事件回调，不是 SDK 全局依赖；source list/read/search 的参数/结果与现有 HTTP SDK 相同。

- [ ] 实现状态 `idle/authenticating/connecting/negotiating/ready/reconnecting/closed/failed`；事件含阶段、客户端时间、requestId、方法及脱敏结果摘要。
- [ ] 复用生成消息类型和既有错误映射，连接成功保存完整协商结果，不像当前 HTTP SDK 那样只保留 bindingId；给 HTTP SDK 增加可读取的协商结果，不改变其懒握手语义。
- [ ] pending 按请求 ID 关联；连接关闭时拒绝所有 pending。自动重连重新取票/握手，但不重放原调用；主动 close 永久停止该实例重连。
- [ ] JSON WS Profile 为页面默认；原 protobuf Profile 不删除、不降级。页面不维护另一套信封编码。
- [ ] 用已有 Bun 测试习惯覆盖断线、过期 ticket 重新获取、迟到响应、主动关闭不重连；不写“字段被复制”的实现型断言。

**命令：** `bun run typecheck`；`bun test sdk/typescript/tests/ws-client.test.ts`。

**通过条件：** 原生浏览器实际使用 SDK 完成远端读取，connect promise 只在 ready 后完成；失败阶段能准确显示，事件中没有 cookie/ticket/token。

### L08 — 页面与同源静态托管

**修改/新增：** §4 的 web 文件、Host `web.rs`、根 package.json 和 bun.lock。

- [ ] 根 workspace 增加 `web`；`web/build.ts` 用 Bun 编译 `web/src/main.ts` 为 `web/dist/app.js`，复制 index.html 和 style.css；定义根命令 `bun run build:web`。typecheck 覆盖 SDK 和 web。
- [ ] Host 配置 `web_root`；启用时启动前验证 index.html/app.js/style.css 存在，缺文件明确失败。只按固定文件表提供 `/`、`/app.js`、`/style.css`，未知路径 404，不做任意目录文件服务或把 API 404 回退成 HTML。
- [ ] 静态资源输出正确 Content-Type，首页不缓存登录信息；响应设置适配同源连接的 CSP、nosniff、禁止第三方 frame 嵌入。不得把配置秘密打包进前端。
- [ ] main.ts 管理生命周期、SDK、3 秒目录轮询和 200 条事件上限；view.ts 负责安全 DOM 渲染和交互绑定；CSS 管理布局/状态/减少动画偏好。
- [ ] 实现 §3 的全部页面区域。调用操作从授权 scopes 生成；选中离线端点可查看详情，但调用按钮明确禁用且说明原因，状态更新后可恢复。
- [ ] 真实打开 1440px 与 390px 视口，完成键盘登录、握手、选端点、列目录、读取、搜索、断开；截图检查布局与错误状态。

**命令：** `bun install`；`bun run build:web`；`bun run typecheck`；`cargo build -p conex-host --locked`。

**通过条件：** 首页和资源 200，无白屏/控制台异常；刷新页面从 Web 会话恢复登录后建立新 Link，不重放读取；API 404 仍为 API 错误；页面动态反映 Agent 离线，不把旧内容标为新成功。

### L09 — 演示、隔离、跨主机验收

**修改/新增：** xtask、examples/landing、`crates/conex-host/tests/landing_e2e.rs`、运行手册与验证记录。

- [ ] 新增 `cargo xtask landing-demo`：生成临时目录/随机凭据/三个实际配置，启动一个 Host 和两个 Agent，输出本机页面 URL 及仅该临时环境的 UI 登录凭据；Ctrl-C 排空并结束全部子进程。展示凭据只出现在主动启动的本地终端，不进入服务访问日志。
- [ ] 示例静态配置使用 env/file 引用，不提交可直接用于生产的固定秘密。A/B 的本地目录包含同名但不同内容的文件，保证演示能识别路由串线。
- [ ] 新增 `cargo xtask e2e --suite connected-landing`，真实启动三进程，验证 source 三方法、权限隔离、断开/重连、ticket 边界及 Host 重启要求重新登录/连接；调用现有工具链，不用假 provider 替代 Agent。
- [ ] 在浏览器完成整条交互；关闭 A，40 秒失联上限内且下一次目录轮询后页面显示离线；直接 socket 关闭应更快。B 仍可调用；重启 A 后恢复 ready，旧请求仍保持失败而不是自动成功。
- [ ] 运行一组 WSS 部署验收：可信证书成功、错误 server name/不可信 CA 拒绝、Origin 不符拒绝、日志不泄密。
- [ ] 在不同主机部署至少一个 Agent，记录实际网络位置、Host URL、TLS 配置、原生浏览器访问和远端读取结果。尚无真实跨主机环境时，完成本机验收并将该项保留未完成，不能改写为“跨地域已验证”。不擅自申请公网资源或开放防火墙。
- [ ] 集成完成后一次运行 `cargo xtask check`，将新增 headless 多进程套件纳入门禁；浏览器视觉验收单独记录，不能以 Rust/Bun 测试替代。
- [ ] 冒烟通过后同步 docs 导航、路线图、runbook 和新增 verification；修正与本次实际实现直接冲突的 P1 旧表述，历史证据保留时间与基线。删除临时脚本、临时凭据和截图之外的不需要产物。

**命令：** `cargo xtask e2e --suite connected-landing`；`cargo xtask check`。

**完成判据：** 下节 E01–E12 均有真实证据，不能仅以编译通过或 L08 页面完成宣告交付。

## 6. 最终验收矩阵

| 编号 | 场景 | 通过条件 |
|---|---|---|
| E01 | 启动与同源页面 | Host + 两个 Agent 可重复启动；页面资源无 404；错误配置明确拒绝 |
| E02 | 浏览器认证 | 原生浏览器无需自定义 WS 请求头；登录、取票、握手、退出均真实工作 |
| E03 | 握手可信 | 服务端不回显客户端为自身能力；requires 不满足/重复 ready/提前业务/超时均拒绝 |
| E04 | 端点可见性 | 同一 Host 下两用户看到不同端点/资源范围；不泄露私有路径/秘密 |
| E05 | 路由正确 | 两 Agent 同名文件返回不同预期内容；source 三方法均经指定 Agent 执行 |
| E06 | 授权不旁路 | 跨租户/越界请求在远端收到业务数据前被拒绝；私有侧路径逃逸仍拒绝 |
| E07 | Agent 身份与 TLS | 错误 Host、错误凭据、Agent 冒名和越权注册被拒绝，秘密不泄露 |
| E08 | 断开与隔离 | A 下线不影响 B；旧代次响应/清理不影响新连接；无 pending 泄漏 |
| E09 | ticket 与会话 | 一次性/30 秒/Origin/角色/注销撤销成立；不能重复使用或扩大权限 |
| E10 | 有界与失败 | 队列/帧/在途上限生效；超时/慢消费者/断连有明确结果，无自动重放调用 |
| E11 | 页面可用性 | 桌面/移动、键盘、减少动画、空状态/失败/重连可操作；显示内容不执行脚本 |
| E12 | 部署事实 | 本机多进程和真实跨主机证据分别记录；不把位置标签当作跨地域证明 |

## 7. 执行纪律与风险

- 修改任何已有符号前，遵循仓库要求做 GitNexus upstream impact；导出符号还需 LSP references。记录直接调用方、受影响执行流与风险，HIGH/CRITICAL 先向用户说明。当前会话未挂载 GitNexus 工具，执行阶段必须先恢复工具或获得替代检查方式的明确许可，不能谎称已完成分析。
- 本轮仅编写计划，不修改运行代码，不提交 git，不运行实现验收。不把本计划中的 Expected 当作实际通过。
- 每个安全/并发边界先补能失败的具体行为用例，再实现最小改动；UI 以真实浏览器路径和截图为证。不要用 source-text 断言、mock echo 或属性复制测试凑数量。
- Registry 共用安装验证、WS reader/writer 改造、认证角色迁移是负载较重的边界。先完成各自负例，再接页面；不为尽快出图绕开这些工作。
- 最值得防的错误：Agent 注册但未挂 socket；Host 跳过策略直接转发；旧连接关闭新连接；ticket 发出但无人消费；WS open 冒充 ready；hello 能力冒充端点权限；离线后页面静默重放调用。
- 本轮只读路由不需要业务 Session。未来增加 ACP、回调审批或写操作时另行实施 Session/operation 的归属与恢复契约，不能把本轮连接恢复扩写为无损业务恢复承诺。
- 完成后按契约、浏览器认证、Agent/路由、目录/SDK、页面、联合验收边界提出提交切分；只有用户要求提交时执行，并先运行 detect_changes。计划生成阶段不自动提交。
