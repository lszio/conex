# conex 设计

> 项目名：**conex**（connect + nexus）；工作协议名 `conex/1`。
> 状态：设计草案 v6（2026-09-15），在原 2026-09-14 文档上修订；尚未发布线协议。
> v6 变更：补「与既有实现的关系」；统一内容寻址函数与 source/blob CID 语义；冻结完整 ErrorCode 数值表与信封预留字段；统一聚合/ID/方法命名；拆分 ACP 与 MCP 原生映射；补 key epoch 分发、Change CID 与签名、流恢复信用锚定、blob 分块方法。
> v5 变更：补齐调用授权上下文、消息变体、握手引导、会话恢复与副作用边界；明确 CID/加密/持久性、ACL 定序与冲突语义；修正 ACP 映射和 TOML；收敛阶段范围与验收。
> 阅读顺序：§0 范围 → §2 模型与边界 → §4–8 核心契约 → §14 实施顺序。P2P 独立阅读 §10–11。
> 证据口径：未标注来源的规范性要求均为 **conex 的设计决定**，不是上游协议的保证。外部事实就近链接；2026-09-15 核对的规范见附录 B。§15 的 notez 记录继承 v4 对 `588bfb3` 的审查，本轮未复核该仓库，不作为 conex 已实现或已验证的证据。

---

## 0. 范围与非目标

**核心问题**：中心服务需要访问多个异构供给方，又要保留调用者身份、私有侧凭据和双向回调边界。conex 提供可嵌入的能力路由内核及可选独立进程。

- **场景 A：数据连接。** fs（Org/Markdown）、Anytype API、HTTP/REST 等通过 provider 暴露读取、搜索及明确支持的写入能力。
- **场景 B：ACP 网关。** 集中界面调用公网或内网 agent；workspace 回调落到指定私有侧 peer，用户交互落到指定界面 peer。
- **场景 C：可选 P2P 同步。** 自有空间通过独立 sync 模块实现加密存储、离线变更和节点迁移。A/B 不依赖 C 才能工作。

**硬要求**：多协议可扩展；二进制有独立通道；新增 provider 走注册和注入；身份、授权、错误、限额、恢复保证可以被验证。扩展在已定义契约内只新增实现与注册项；增加新语义需演化契约，不承诺所有轴的任意组合都可用。

**首个可交付切片**：P0 仅实现 broker 平面的只读数据连接、JSON/HTTP 与 inproc、最小 CID 类型和 env/file 凭据。用第二个 provider 验证边界，再推进反连与 ACP。P2P 不进入 P0 的必经执行路径。

**非目标**：

1. 不做 agent 编排、通用 API 网关或服务网格。
2. 不做冲突处理的产品化 UI；并发歧义保留并显式返回，不以最后写入者自动覆盖。
3. 浏览器不直接承担 workspace 执行和 ACP 上游连接管理；这是本产品的部署选择（§9.3）。
4. 不实现 any-sync 线协议或充当 Anytype 网络节点；Anytype 通过其受支持 API 接入。
5. 不提供跨任意上游的 exactly-once 执行保证，不承诺仅持密钥与 heads 即可恢复内容。
6. P3 首版不做多组织 ACL 共识：每个 Space 使用一个逻辑 ACL 权威；目录协调者不承担这一角色。

### 0.1 与既有实现的关系

本仓库 `dev` 分支存在前一阶段的 **CONEX** 实现（TypeScript/Bun，Anytype ↔ Apple Reminders 双向同步，Phase 0–3 已跑通，见该分支 `ARCHITECTURE.md`、`AGENTS.md` 与 `src/adapters/**`）。本设计（`conex`，工作协议 `conex/1`）是**取代它的下一代内核**：把「Anytype↔Apple 专用桥」提升为「可嵌入的双向能力路由内核 + ACP/MCP 网关 + 可选 P2P」。

- 既有实现不就地演化；其产物（Anytype API `v2025-11-08` 接入经验、状态映射、冲突日志、Apple 侧适配）只作为**领域参考**，通过 §9.1 的 provider/桥接契约重新实现，不复用其运行时与状态库。
- 本设计从当前 docs-only 树开始，构建独立 Rust workspace；`dev` 分支历史仅作证据，不构成 `conex/1` 的既有实现，其 TypeScript 代码不进入 P0–P4 的交付物。
- §15 的 notez 记录是外部参考与反面教材，与上述既有 CONEX 实现相互独立。
- 仓库继续使用 `conex` 名称；旧实现保留在 `dev` 分支，不发布新包、不迁移真实用户数据。

> 仓库基线（2026-09-15 复核）：当前分支 `refactor/arch`，HEAD `0414896` 只包含本设计与此前两份计划文档；`dev` 分支保留旧实现与 `ARCHITECTURE.md`。此前计划中「master 是 unborn branch / 没有提交记录」的表述作废。

## 1. 核心判断

**J1 双向能力路由。** Provider/Consumer 是 Peer 在一次交互中的角色。host 负责注册、调用上下文、策略、凭据、路由、会话和审计；内容存储、sync 与协议桥接通过独立端口接入。

**J2 数据导向分派。** 按注册键定位构造器、方法契约和处理器，不在业务主路径按 provider 名字分支。第二个真实 provider 必须走与第一个相同的装配路径。

**J3 分层但限制组合。** 协议语义、编码、分帧、承载分别建模，通过具名 `Profile` 声明合法组合。JSON 是编码，JSON-RPC 2.0 是消息协议；relay 是拓扑角色，不是独立字节编码。

**J4 二进制独立。** conex 链路优先传 `BlobRef`；与仅支持内联的上游通信时在协议桥接边界转换 base64。P0 即确定 CIDv1 的表示，分块通道在 P1 实现。

**J5 两个信任平面。** `broker` 的授权处理方可以读内容；`relay` 的存储与中继只能读密文及明确公开的元数据。平面绑定能力端点与 Session，不能在同一 Session 内隐式转换。

**J6 传输、执行与历史分开。** Stream ACK 确认接收，不确认业务执行；操作状态表达副作用结果；DAG 表达因果历史，不自动决定业务冲突如何解决。

**J7 身份保留来源。** 内部 Principal 映射自带命名空间的外部身份。OIDC 使用 `(issuer, subject)`；设备使用公钥身份。身份绑定不自动产生空间访问权。

**J8 分阶段授权。** 本地调用预检与地址准入先于业务拨号；对端身份验证和协商先于发送业务凭据与请求。每条回调、恢复和 blob 请求都重新经过对应授权规则。

**J9 能力与失败诚实。** 未实现或当前不可用的能力明确拒绝；真正的空查询结果可以为空。结果未知、传输中断、权限拒绝和版本冲突不能混为同一种错误。

**J10 一个类型源与明确语义。** `.proto` 是结构类型源，JSON 映射与状态机文档是行为规范，JSON Schema/SDK 类型由源生成。一致性向量验证规范，不反过来覆盖规范。

**J11 发现仅提供候选。** 静态配置、mDNS、协调者、可选 DHT 都不能签发业务权限。对端身份预期来自管理员配置、邀请或空间信任链，不能仅由目录应答决定。

## 2. 设计原则与模型

本文原则编号使用 `P1`–`P11`；§14 的 P0–P4 表示交付阶段，两者上下文不同。

| 原则 | 可执行含义 |
|---|---|
| P1 可加性 | 契约内新增实现不改内核分支；装配根注册、依赖声明和向量可增加 |
| P2 数据导向 | Registry 保存处理器和方法契约；能力名称不承担隐藏业务逻辑 |
| P3 清单是数据 | 清单不包含可执行脚本；插件代码与进程启动配置由管理员另行安装和授权 |
| P4 边界明确 | 各层通过类型与 Profile 约束组合；不把分层等同于任意组合兼容 |
| P5 显式语义 | 错误、部分结果、权限、状态、版本和未知结果可机器判别 |
| P6 先预检后业务连接 | 本地拒绝时零目标连接；握手后拒绝时零业务请求和业务凭据 |
| P7 最小明文暴露 | relay 不持内容密钥；broker 的明文接收者和凭据持有者显式授权 |
| P8 有界且可取消 | 限额涵盖帧、字节、队列、并发、重放窗口、存储与资源租约 |
| P9 内容与历史 | CID 标识不可变字节；历史保留受明示保留策略控制，不承诺无限存储 |
| P10 契约与验证 | 类型只维护一个源；状态机、映射及向量有明确优先级 |
| P11 发现与信任分离 | 地址和能力声明是线索；不能改变主体、ACL 或凭据目标绑定 |

### 2.1 核心模型

| 概念 | 形态与不变量 |
|---|---|
| `Principal` | 内部主体 ID；外部身份映射保留来源及租户边界 |
| `Peer` | 经过认证的通信参与方；可同时提供服务与发起调用 |
| `Manifest` | 能力端点自描述；管理配置决定它能注册哪些 provider、方法与资源 |
| `MethodContract` | 输入/输出 schema、授权提取器、所需能力、重试与取消语义 |
| `Profile` | 协议版本、编码、分帧、承载及传输能力的合法组合 |
| `Frame` | Profile 定义的有界传输单元；没有适用于所有外部协议的统一长度头 |
| `Message` | 请求、成功、失败、通知四种变体（§5.2） |
| `CallContext` | host 验证后生成的主体、资源、会话、授权与路由上下文（§8.3） |
| `Credential` / `Secret` | 凭据引用 / 秘密值；值不能进入清单、审计和普通消息元数据 |
| `Cid` / `BlobRef` | 标准内容地址 / 带访问上下文的内容引用（§5.3） |
| `Link` | 两个已认证 peer 间的一次逻辑连接；固定 Profile，断线后建立新 Link |
| `Session` | 固定主体、provider、平面、workspace/UI 绑定和授权边界的一次协作；可跨 Link 恢复 |
| `Call` | 一次方法调用；请求关联与可选的稳定操作 ID 分开 |
| `Stream` | Session 内单方向的有序流；独立序号、ACK、信用和重放窗口 |
| `Route` | 在授权边界内解析执行者；回调使用 Session 的固定绑定，不搜索全局任意同能力 peer |
| `Space`（P3） | 独立的成员 ACL、密钥 epoch、对象集合与保留策略 |
| `Object`（P3） | 稳定 `objectId` + 不可变变更 DAG + heads；对象身份不同于当前内容 CID |

`source/read` 可以返回上游资源及版本，不必把第三方系统改造成 conex Object。审计和 trace 默认使用有保留策略的日志；只有需要跨设备验证的历史才进入签名 DAG。

### 2.2 组件边界与扩展验收

| 组件 | 负责 | 扩展验收 |
|---|---|---|
| `conex-proto` | 类型源、JSON 映射、方法与错误契约、Profile 标识 | 多语言类型与向量一致 |
| `conex-core` | Registry、路由、授权接口、调用与会话状态机 | 第二个 provider 不改核心执行分支 |
| `conex-host` | 装配、监听、策略及凭据实现、审计出口 | 嵌入/独立部署使用同一安全路径 |
| 协议桥接模块 | 外部协议生命周期、ID 映射、转换、登录和上游 I/O | 新协议只增桥接器、注册与向量 |
| 承载/Profile 模块 | 字节收发、分帧、身份材料与传输能力 | 新组合通过声明的能力向量，不伪装缺失语义 |
| 内容模块 | CID、blob 上传、pin、持久性、GC | 替换后端仍满足同一持久性等级 |
| `conex-agent`（P1） | 私有侧反连、受限 workspace 与上游凭据 | host 不获得 agent-held 的秘密值 |
| sync 模块 / `conex-node`（P3） | 签名历史、ACL 检查、密文复制与存储 | 停用模块不影响 broker 路由 |
| `conex-coordinator`（P3） | 可选目录、租约、可达性线索 | 停机不撤销现有权限，不改变已有 Session 的执行绑定 |

每个阶段只引入已被切片验证的端口。新增协议语义可能要求扩展 `MethodContract` 或版本；用协议演化处理，不以隐藏例外维持“零修改”。

## 3. 命名与责任

- `Provider` / `Consumer` 是 Peer 的角色；`Node` 专指零知识存储/中继节点。
- `providerId`、`peerId`、`sessionId`、`attachmentId`、`requestId`、`operationId` 各有作用域，不能互换。attachmentId 标识 Session 内固定 peer/角色的参与绑定；ID 是标识，不是凭证。
- 公开方法采用 `namespace/action`，如 `source/read`、`agent/session.prompt`；扩展使用带所有者命名空间的 `_<owner>/action`。
- 能力描述使用 `provides` / `requires`，以方法名为基础，可附参数限制与传输要求；不再混用 `source.read` 与 `source/read`。
- `plane` 在每个能力端点固定；同一 provider 可注册 broker 与 relay 两个端点，各自建 Session、授权和缓存。
- 默认包名为 `conex-proto`、`conex-core`、`conex-host`、`conex-agent`、`conex-node`、`conex-coordinator`；SDK 暂用 `@conex/sdk`、`conex-sdk`，公开发布前检查包名可用性。

## 4. 注册、清单与握手

### 4.1 注册表

| 键 | 值 |
|---|---|
| `(kind, protocol, majorVersion)` | 已安装的 Peer/桥接器工厂 |
| `(protocol, majorVersion, method)` | `MethodContract` 与处理器 |
| `profileId` | 编码、分帧、承载实现与兼容约束 |
| `(contentCodec, formatVersion)` | 内容解码/校验管线 |
| `storageBackend` | 满足明示持久性等级的后端 |
| `discoverySource` | 产出 Candidate 的发现实现 |

重复注册默认报错；替换必须由装配根显式配置。清单声明不能自动加载代码，也不能授予工厂读取任意凭据或访问任意地址的权限。

### 4.2 清单示例

此例描述一个已安装的 Anytype 桥接 provider 对 **conex** 暴露的端点，不表示 Anytype API 原生支持 conex。所有根字段均放在 TOML 表之前。

```toml
manifestVersion = 1
id              = "work-anytype"
endpointId      = "work-anytype-broker"
kind            = "source"
display         = "Work Anytype"
plane           = "broker"
transport       = "auto"
provides        = ["source/list", "source/read", "source/search", "blob/get"]
requires        = []

[[profiles]]
id = "conex-jsonrpc2-wss-v1"

[[profiles]]
id = "conex-protobuf-wss-v1"

[auth]
mode       = "agent-held"
credential = "anytype/work"

[blob]
modes        = ["inline", "ref"]
maxBlobBytes = 2147483648

[discovery]
announce     = ["source/read"]
bootstrap    = []
coordinators = []
mdns         = false
dht          = false

[limits]
timeoutMs      = 8000
maxInflight    = 4
maxFrameBytes  = 1048576
maxQueuedBytes = 8388608
```

注册地址、预期对端身份、允许的 workspace 根目录、凭据后端以及子进程启动命令属于**管理员安装配置**，不能从远端清单直接采纳。`transport=auto` 只选择本地安装且双方提供的 Profile；不能探测任意新 URL。示例列出 P1 能力，不是 P0 的必需集。

### 4.3 能力方向与会话绑定

`hello` 双方分别声明 `provides`、`requires`、可选能力、Profile、平面和上限。每个能力还可携带输入内容类型、最大尺寸、资源限制等约束。

- A 要调用 B 的某方法，检查 A 的调用需求是否由 B 的 `provides` 满足，反向同理；不能把双方所有能力直接求交集。
- 所有硬性 `requires` 必须满足；可选能力不足进入 `rejectedCapabilities[{method,direction,reason}]`。编解码等共同选项才按兼容集合选择，上限取双方较小值。
- `session/open` 固定 provider 端点、plane、workspace peer、human peer 和能力快照；运行时每次调用仍检查在线状态、当前权限和限制。
- 某个回调目标失联，明确暂停或拒绝该操作；不把回调转到另一个用户的同能力 peer。增加权限或更换 workspace 需显式重新授权，首版创建新 Session。

### 4.4 可实现的握手引导

连接前的调用预检与地址准入见 §8.4。**在线协商发生在连接建立之后，业务就绪之前。**

- P0 HTTP 的 Profile 由已配置 URL、协议版本及 Content-Type 固定；`conex/hello` 只检查该 Profile 的能力与版本，后续调用使用返回的主体绑定 `bindingId`。binding 有效期默认 60 秒；过期需重新 hello，且每个 HTTP 请求独立认证。
- conex WSS 先使用固定引导格式：UTF-8 JSON-RPC 2.0、一个 JSON 消息占一个 WS text message、上限 64 KiB。引导格式本身不参与协商。
- WSS 顺序为 `hello → hello result → conex/ready → ready result`，均使用引导格式。`ready` 必须回传服务端生成的一次性协商 ID；双方核对选定 Profile、版本、plane 和能力摘要。
- 服务端写出 `ready result` 后切换选定格式；发起方收到该结果后才发送选定格式的第一条消息。此前禁止业务消息，重复 ready 或中途变更选择使握手失败。
- 承载为 WSS 时 `hello` 返回 `linkId`/协商信息；承载为 HTTP 时返回 `bindingId`（HTTP 无 Link 概念，每个请求独立认证并可用同一 binding 复用协商结果）。二者都不隐式创建业务 Session。Session 由 `session/open` 或经授权的 `session/resume` 建立。
- 外部 ACP/MCP 端点使用其原生初始化，不接受 conex 引导帧。桥接器在两侧分别建立连接状态。

无共同版本/Profile 返回明确支持列表，不做猜测式解码；握手超时默认 10 秒。T2 反连先经过已授权的 peer 注册路径，稍后的业务调用可以复用连接，但仍须独立授权。

## 5. 协议与内容契约

### 5.1 Profile 与协议桥接

| Profile | 语义 / 编码 / 分帧 / 承载 | 能力 | 阶段 |
|---|---|---|---|
| `conex-inproc-v1` | conex / 内部类型 / 无字节分帧 / inproc | 同一方法及授权契约；无网络恢复测试 | P0 |
| `conex-jsonrpc2-http-v1` | conex 方法 + JSON-RPC 2.0 / JSON / HTTP body / HTTPS | 同步请求响应；无异步回调 | P0 |
| `conex-jsonrpc2-wss-v1` | conex 方法 + JSON-RPC 2.0 / JSON / WS message / WSS | 双向回调、Stream、恢复 | P1 |
| `conex-protobuf-wss-v1` | conex 消息变体 / protobuf / WS message / WSS | 同上；二进制 blob 数据 | P1 |
| `acp-jsonrpc2-stdio-v1` | ACP v1 / JSON / newline / stdio | ACP 原生双向；无原生 conex resume | P2 |
| `mcp-streamable-http-2025-06-18` | MCP 指定版本 / JSON / HTTP 与 SSE / HTTPS | 按 MCP 原生规范实现 | P2 |

本表是首批支持范围，不承诺其他组合。WS 已提供消息边界，不再叠加 varint 长度头；未来字节流 Profile 可以使用长度前缀。HTTP 生产默认 TLS，本机开发可显式允许 loopback 明文。

协议桥接器有两层：

1. **纯转换层**：`decode/encode/normalize/denormalize` 处理字段、错误和内容类型；输入包含明确的版本和方向。
2. **有状态桥接层**：管理外部 initialize/authenticate、请求 ID 表、Session 映射、进程、blob I/O、取消和结果状态。

不支持的可选外部字段放内部 `foreign` 容器或保留原消息；外部 `_meta` 不被 conex 覆盖。影响授权或业务语义的未知字段不能靠透传默认放行。明确区分语义保真与原始字节保真；后者只有保留原字节时承诺。

### 5.2 Message、标识与错误

逻辑模型不随 JSON/protobuf 改变：

```text
Request      { kind: request, requestId, method, context, params, operationId?, meta? }
Success      { kind: success, requestId, result, meta? }
Failure      { kind: failure, requestId?, error, meta? }
Notification { kind: notification, method, context, params, meta? }
```

- conex `requestId` 是发送方生成的 ULID 字符串，protobuf 同样用 string；请求表以 `(sessionId, senderPeerId, requestId)` 关联，连接控制请求使用 linkId。通知没有 requestId。
- JSON-RPC 映射：Request 使用 `id/method/params`，Success 使用 `id/result`，Failure 使用 `id/error`，通知省略 `id`。conex 自有方法的 `params/result/error.data` 装入生成的 conex 数据；外部 ACP/MCP 消息由桥接器按其原结构处理。
- 外部 ID 以带类型的 `ForeignId` 保留；数值与字符串不能相互混淆。桥接器维护双向 ID 表，不能把外部字符串截断或强转为整数。
- `Failure.requestId` 缺失仅用于无法识别请求的协议错误，映射 JSON-RPC `id:null`；通知的业务错误进入审计或明确的流状态通知，不对通知伪造响应。
- 业务 `context` 是调用方提出的目标选择，不能直接当作可信 `CallContext`。授权上下文由接收侧生成/验证（§8.3）。
- 调用方提交 `timeoutBudgetMs`；每跳按本地单调时钟扣除已经消耗的预算，重试沿用剩余预算，不能重新获得完整超时。CallContext 的 deadline 是本地执行截止点，跨机器不直接比较单调时钟值；长时间 agent 调用按方法配置自己的上限。
- **信封与载荷分层**：消息信封（Request/Success/Failure/Notification）的 `params`/`result` 使用通用结构容器承载；每个方法的具体输入/输出是 `.proto` 生成的 typed message，由方法注册处的 `prepare`/`validate_output` 作为**唯一权威严格解码器**：拒绝未知字段、uint64 只接受规范十进制字符串、区分 optional 与缺省、oneof 至多一个。生成的 JSON Schema 只作为 SDK 文档与 TS 边界校验输入，不得成为第二套语义源；JSON 与 protobuf 两条路径对同一输入必须给出相同判定，不依赖 pbjson/prost 的宽松反序列化默认值。
- v1 信封为 `operationId`、`meta` 预留字段 tag；P0 不使用时也不得改号或复用。

这些映射遵循 [JSON-RPC 2.0 的请求、响应与通知规则](https://www.jsonrpc.org/specification)。默认不支持 JSON-RPC batch；收到 batch 明确拒绝，不能只执行其中一部分。

```text
Error { code, message, diagnosticId,
        execution: not_started | completed | unknown,
        retry: never | safe | with_operation_id,
        details?, foreign? }
```

错误码基线：`unauthorized`、`forbidden`、`unknown_provider`、`unknown_method`、`unsupported_capability`、`unavailable`、`timeout`、`cancelled`、`outcome_unknown`、`stale_revision`、`conflict`、`bad_request`、`payload_too_large`、`slow_consumer`、`quota_exceeded`、`bad_blob`、`plane_mismatch`、`peer_untrusted`、`session_lost`、`resume_unavailable`、`internal`。

`retryable` 不再作为独立错误类别。重试取决于方法契约及 execution，不以 HTTP 5xx 自动推断安全。外部错误保留原数值 code、message、data；JSON-RPC 标准协议错误保留其原码。conex 业务错误使用生成的固定数值映射，语义码保存在 `error.data.code`，不能全部改成 `internal`。**下列数值在 v1 冻结，任何阶段不得改号或复用；`-32019..-32099` 预留给后续业务码，`-32700/-32600/-32601/-32602/-32603` 保留 JSON-RPC 标准语义。**

| 语义码 | 数值 | 语义码 | 数值 |
|---|---|---|---|
| `parse_error` | -32700 | `unauthorized` | -32001 |
| `bad_request` | -32602 | `forbidden` | -32002 |
| `unknown_method` | -32601 | `unknown_provider` | -32003 |
| `internal` | -32603 | `unsupported_capability` | -32004 |
| `unavailable` | -32005 | `timeout` | -32006 |
| `payload_too_large` | -32007 | `quota_exceeded` | -32008 |
| `plane_mismatch` | -32009 | `peer_untrusted` | -32010 |
| `cancelled` | -32011 | `outcome_unknown` | -32012 |
| `stale_revision` | -32013 | `conflict` | -32014 |
| `slow_consumer` | -32015 | `bad_blob` | -32016 |
| `session_lost` | -32017 | `resume_unavailable` | -32018 |

控制方法按责任归属命名：`conex/hello|ready|ping|bye` 管 Link，`session/open|resume|renew|close` 管 Session，`stream/ack|flow|reset` 管流，`call/cancel` 与 `operation/get` 管调用状态；`blob/*`、`source/*`、`agent/*`、`workspace/*`、`human/*`、`sync/*`、`discover/*` 各自按方法契约注册。未进入当前交付阶段的方法不广告。

### 5.3 CID 与 BlobRef

**基础地址格式从 P0 固定**：CIDv1 二进制为 `version(1) || content-codec || multihash`；multihash 包含摘要算法、长度和 digest。文本默认小写 base32；接受的其他文本表示先规范化为二进制比较。P0 必须支持 `raw + SHA-256`；其他算法由内容能力明确声明，不因传输 codec 改变 CID。[CID 规范](https://specs.ipfs.tech/cid/) 是格式依据。

```text
BlobRef { cid, size, mime, access: { providerId, plane, spaceId?, resourceId } }
```

`cid` 证明字节完整性；`access` 只是授权定位信息，不是凭证。接收侧用资源元数据或授权句柄证明资源与 CID 的关联。`blob/get`、`blob/have`、`blob/pin` 都经过资源级策略；跨空间相同 CID 不继承权限。

**统一内容寻址函数（P0 固定）**：`contentCid(bytes, chunkSize=256KiB)` 定义为 `len(bytes) <= chunkSize` 时返回 raw/SHA-256 CID，否则返回按 §5.4 分块树规则得到的 manifest 根 CID。`source/read` 返回的 `cid` 与 `blob/*` 的根 CID 必须使用同一函数：同一份内容在 P0 与 P1 得到相同地址，不得对同一内容同时声明「整文件 raw CID」与「分块根 CID」两种可比较地址。

内容 codec 与传输 codec 分离：原始块使用 raw；结构化 manifest 使用确定性内容格式。签名、CID 的输入必须是明确的原始字节或协议规定的规范化字节，禁止解码后用任意序列化结果重新计算。protobuf 的 deterministic serialization 不保证跨版本规范化，见 [官方说明](https://protobuf.dev/programming-guides/serialization-not-canonical/)。

### 5.4 分块、加密与去重

**P1 broker 分块**：默认固定 256 KiB 数据块，末块可更短；按顺序组成 manifest，条目含子 CID、逻辑长度及总长度。manifest 超过 1 MiB 时分层，规定树形构造规则以保证同一内容产生同一根；发布 P1 Profile 前冻结该格式和跨语言向量。P1 暂不做 CDC，后续另增内容格式版本。直接使用 `Bytes` 共享所有权，避免不必要的全量复制；解码、加密和上游转换可进行有指标的必要分配。

**P3 relay 加密**：

- 业务内容块在成员设备侧加密；其存储 CID 对实际保存的**密文封装字节**计算。封装包含格式版本、加密套件、key epoch、nonce、密文及认证标签，不携带内容密钥或明文摘要。公开的签名 Change/ACL 记录则对其自身规范化记录字节计算 CID。
- 用于复制遍历的子 CID/长度、DAG parents、space/object 标识与作者等元数据可公开；它们必须被 CID/签名覆盖。零知识仅指业务内容不可读，不等于流量和关系元数据不可见。
- 相同密文块可以按 CID 去重；随机化加密下独立加密的相同明文不保证相同 CID。客户端可在同一空间、同一 epoch 复用已有加密块，不做跨空间明文去重，不默认采用收敛加密。
- 密钥轮换改变新内容的密文地址；旧历史是否重加密由保留/迁移策略决定。relay 根引用来自已验证的签名历史，不能只相信目录提供的 root CID。
- AEAD、签名、密钥派生、nonce 规则、规范化 manifest 与密钥封装组成一个具名加密 Profile；算法和已知答案向量是 P3-a 的发布前置条件（§16），不得以可任意拼接算法参数代替安全审查。

### 5.5 Blob 生命周期、成功语义与 GC

```text
blob/put(open) → uploading → verified → blob/commit → committed
                     ↘ cancel/expire → staging 回收
```

1. `blob/put` 创建绑定主体、资源、内容格式和大小上限的 uploadId，返回已持有块及续传信息；默认 staging 租约 1 小时，可在配额内续期。
2. 数据块通过 `blob/chunk{uploadId, chunkIndex, cid, bytes}`（或经 Stream 传送的等价帧）提交，`blob/put` 只创建/续开上传而不承载数据；收到块后先校验大小与 CID，再存入 staging；坏块返回 `bad_blob` 并结束当前传输，可保留此前已验证块供重新打开上传续传。
3. `blob/commit` 检查根 CID、全部可达块、长度和授权，并原子建立根与资源的持久引用；后台 GC 不能删除 commit 正在引用的块。
4. `committed` 只在约定持久性达成后返回；失败返回实际状态及缺块列表。上传 ACK、CID 校验成功、加入内存队列都不等于 committed。
5. `blob/pin` 返回绑定 principal/space/root 的 pinId 和到期时间；`blob/unpin` 只释放调用者有权管理的 pin。对象引用、显式 pin、上传租约和迁移保留共同决定 GC 可达集。
6. GC 只回收不被任何有效根引用且已过保留宽限期的块。历史只追加表示保留期内不原地篡改，不表示永不删除；历史裁剪须有明确策略，P3 首版不做自动压缩。

| 持久性等级 | commit 前提 | 不覆盖的故障 |
|---|---|---|
| `local` | 所有块及根引用完成本地持久写入，崩溃恢复可见 | 磁盘/整机丢失 |
| `replicated(n)`（P3） | 满足 n 个配置故障域的节点持久保存并出具绑定 root 的接收凭证 | 超出副本预算的故障、所有持有者串通丢弃 |

后端必须声明支持等级；无法满足请求等级应拒绝，不能降级后返回成功。接收凭证是承诺与审计依据，不是节点永远保存数据的密码学证明；需周期完整性检查与补副本。客户端只有 heads/密钥而无可用内容副本时，必须报告缺块。

### 5.6 外部二进制兼容

ACP 内容定义包含文本、图片、音频和资源等类型，部分二进制字段为 base64，见 [ACP Content](https://agentclientprotocol.com/protocol/v1/content)。兼容按锁定的 ACP/MCP schema 逐字段测试，不假设未来版本完全同构。

- `inline`：遵循外部格式，受双方消息大小限制；256 KiB 为内联选择建议，不是绕过对端限制的硬阈值。
- `ref`：仅用于协商支持的 conex 一跳；桥接器对原生 ACP/MCP 上游按其支持方式转换，不能要求未扩展的 agent 理解 BlobRef。
- 往返保证**解码后的内容字节相等**，并保留 mime、URI、annotations 和未知可选字段。只有保存原始表示才承诺 base64 文本或 JSON 字节完全相同。
- base64 的临时存储与内存计入桥接器配额。超大内联内容无法在资源预算内转换时返回 `payload_too_large`；1 GiB blob 验收不隐含 ACP 上游也支持 1 GiB 内联消息。

## 6. 承载与背压

### 6.1 选择规则

`auto` 在允许的已安装 Profile 中选择满足本次业务 `requires` 的组合。初始偏好 WSS；P0 的同步数据调用可使用 HTTPS。需要双向回调/活流的 Session 不能降级到普通同步 HTTP。

SSE+POST 作为可选 conex Profile 后续实现时，必须补齐双向请求关联、认证绑定、取消和恢复；不能仅因能够传输 JSON 就声明与 WSS 等价。MCP Streamable HTTP 按其[原生规范](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)桥接，不直接充当 conex 承载。QUIC/WebTransport 需各自声明 stream mapping；中继隧道仍承载某个已协商 Profile。

承载负责字节、分帧、I/O 生命周期和传输能力；业务 Session、授权和副作用语义由 core 管理。切换承载建立新 Link，并按 §7 恢复，不在活连接中静默改变格式。

### 6.2 Stream、ACK 与信用

流的数据帧逻辑形态为 `{sessionId, attachmentId, epoch, streamId, seq, message}`；每条 Stream 单方向，两个方向各用独立 streamId。seq 从 1 单调递增，ACK 是已连续接收的最大序号。epoch 属于本帧经过的参与绑定，不能用一个全局 Session epoch 同时替换所有 peer 的连接。

- **ACK** 仅表示接收端已将数据放入承诺的有界接收状态；是否已完成业务执行由 Call 结果表示。P1 默认重放状态在进程内，host 重启不承诺该状态仍在。
- **信用**：`stream/flow{streamId, consumedBytes, windowBytes}` 使用累计已消费字节数和固定接收窗口。发送端累计发送字节不能超过二者之和；重复或因重放产生的旧 flow 忽略，不重复加信用，消费计数超过实际发送量使流失败。窗口在流建立时固定，不能用后续 flow 扩大到协商上限之外。零字节数据帧禁止，防止绕过字节信用消耗 CPU。
- **计费单位**：不可变的 `seq + message` 经选定 Profile 编码后的字节数，业务元数据同样计入；有严格大小上限的路由头和传输头不计入信用，完整帧仍受 maxFrameBytes 限制。重放复用原消息字节和累计位置，不消耗第二份逻辑信用，也不因 epoch 头变化重新计费。
- **资源边界**：默认最大帧 1 MiB、每流窗口 4 MiB、每 Session 接收队列与发送重放缓存各 8 MiB。Session 总预算优先于每流预算，另有主体和进程总限额。
- **控制通道**：ACK、flow、cancel、ping 使用独立有界控制队列和速率限制，不能因数据信用归零而死锁；控制队列耗尽时断开 Link 并记录错误。
- credit 耗尽先停发；持续阻塞默认 30 秒后可按方法策略取消该流并返回 `slow_consumer`。从不能暂停的上游接收数据时必须选择有界落盘或中止，不能无界缓存。

`maxInflight` 是并发 Call 上限，不能替代字节信用。所有数值上限由握手取兼容的更严格值。重放缓存默认最长 120 秒，实际可恢复范围由时间与字节预算共同决定并向对端报告。

**恢复时的信用锚定**：`session/resume` 成功后，接收端必须按断线前的 `consumedBytes` 重新广告信用，且新窗口不得小于断线时该流尚未确认的已发送字节；否则发送端会越过 `consumed + window`。若新 Link 协商出更小窗口，接收端必须先确认足够字节或显式进入 `stream/reset` 并把未确认部分标为需重放，不能直接用更小窗口把流判为 `slow_consumer`。

### 6.3 保活与认证材料

默认每 20 秒 ping，40 秒无响应判 Link 失联；Session 另有资源租约。来自未认证或被撤销 peer 的流量不能续租。

非浏览器 WSS 使用已绑定目标的 Authorization 或 mTLS；浏览器 ticket 见 §8.5。反连 agent 必须通过 PKI/预置 pin 验证 host；bootstrap secret 仅用于在已验证信道上注册，不能代替服务端身份验证。

## 7. 会话恢复、执行状态与取消

### 7.1 Session 状态机

```text
opening → active → suspended → active
             ↓          ↓
           closing ← lease_expired / revoked
             ↓
            closed
```

`session/open` 返回 sessionId、固定绑定、能力快照、租约和可支持的恢复等级。每个必需参与绑定的租约默认为距该参与者最近一次已认证续约 120 秒；任一必需绑定失联使 Session 进入 suspended，其租约到期则进入 closing。provider 的心跳不能代替失联 consumer 续租；一个 Link 上的续约必须列明参与绑定，不能延长无关 Session。显式 close 或撤销也进入 closing，终态不能用旧 ID 复活。

Session 绑定至少包含 `principalId, tenantId, providerEndpointId, plane, workspacePeerId?, humanPeerId?`。P2 首版一个 conex Session 对应一个独立 ACP 进程，初始化身份、workspace 根和登录存储均隔离；不会因为一个 Session 结束而杀掉其他用户共用的进程。进程池属于后续独立优化。

### 7.2 恢复与连接代次

`session/resume{sessionId, attachmentId, expectedEpoch, streams:[{streamId,lastReceivedSeq,consumedBytes}]}` 只能在重新认证完成的新 Link 上调用。

1. host 校验主体、租户、peer 角色与 Session 绑定；重新检查当前授权、撤销和租约。ID 可被猜到也不能取得会话。
2. 成功恢复时按 expectedEpoch 原子递增该 attachment 的 epoch，旧 Link 在该 attachment 上的帧被 fencing 拒绝；并发恢复只接受一个胜者，不影响其他参与者的有效连接。双方确认可恢复的每流范围与固定编码格式，再按原 seq 重放；首版恢复要求原 Profile，改变编码导致的字节信用差异不做隐式换算。
3. 接收端按 `(sessionId, attachmentId, sender, streamId, seq)` 识别重复，并先检查本次 epoch 是否有效。epoch 是连接所有权代次，不属于逻辑去重键；恢复不能清空去重记录。对端提交的 ACK/消费位置不能越过实际发送范围，也不能让已丢失缓存重新被宣称可恢复。
4. 窗口不足返回 `stream/reset` 结果（携带 `streamsReset` 字段）和可用的业务恢复方式，不能把最后一帧之后的数据当作完整历史继续显示。
5. P1/P2 保证进程存活且窗口内的网络重连。host 重启或 ACP 进程退出时返回 `session_lost`；只有实现并声明持久恢复的部署才可承诺跨进程恢复。

恢复方式按业务区分：watch 重新读取快照与游标；blob 用 uploadId 与缺块集合续传；agent 输出若有持久 transcript 则补读，否则标记输出不完整。**禁止用重发 prompt 来代替 agent 输出恢复。**

### 7.3 副作用与幂等

每个 MethodContract 必须选择一种执行类别：

| 类别 | 示例 | 重试规则 |
|---|---|---|
| `read_only` | list/read/search | 在原授权、目标与期限内可重试 |
| `idempotent` | 按相同 CID 写入同一资源 | 同一操作和参数可重试；仍检查策略 |
| `deduplicated` | 上游支持幂等键的写入 | 同一个 operationId；有效期内可取回原结果 |
| `non_replayable` | 普通 shell 执行、无幂等保证的 prompt/外部写入 | 发送后结果不明时不自动重试 |

operationId 与 requestId 分离，采用稳定 ULID；去重键为 `(tenant, principal, providerEndpoint, spaceId?, resourceId, method, operationId)`，同时存参数摘要。相同键但不同参数返回冲突。新的传输请求不能生成新的 operationId 来绕开未知结果。`execution` 类别到 `retry` 的推导固定为：`read_only`→`safe`；`idempotent`→`safe`（仍重查策略）；`deduplicated`→`with_operation_id`；`non_replayable` 且已发送→`never` 且 `execution=unknown`。

```text
operation: accepted → running → succeeded | failed | cancelled | unknown
```

- 支持 `deduplicated` 时，在执行前持久记录操作，并保存结果；默认保留 24 小时且必须覆盖声明的重试期限。返回明确有效期，过期状态不可查时不能静默重新执行。
- 去重记录与本地副作用可用同一事务提交时才提供该边界内的原子保证；外部上游必须支持同一幂等键或可查询操作状态。单独写一张 host 去重表不能消除外部调用的崩溃窗口。
- 上游可能成功但响应丢失时，标记 `unknown`，通过 `operation/get` 查询或人工核对；不能声称失败即无副作用。状态查询也必须授权。
- `expectedRevision` 在上游执行点原子检查才有条件写语义。只做“先读再写”的桥接不得广告条件写能力。

### 7.4 取消与回收

`call/cancel` 指向 requestId/operationId；通知已送达不等于取消已完成。结果与取消并发时，已确定的结果优先保留。只有执行器确认未开始或已停止，才返回对应终态；无法判断是否产生副作用则返回 `outcome_unknown`。

Session close/撤销停止新调用，取消在途操作并关闭流；ACP 进程先发取消/终止，默认 5 秒宽限后强制回收进程组。强制回收不能回滚已写文件或上游操作，审计保留其最后已知状态。

## 8. 身份、授权、凭据与审计

### 8.1 身份映射

| 入口 | 外部身份键 | 验证与映射 |
|---|---|---|
| 浏览器 OIDC | `(oidc, issuer, subject)` | 验证签名、issuer、audience、期限与登录事务；映射内部 Principal |
| 服务 OAuth/bearer | `(service, issuer-or-config, serviceId)` | 验证目标 audience 和权限；不伪装为自然人 |
| 设备 | `(device, keyAlgorithm, publicKeyFingerprint)` | 签名挑战绑定本次信道、nonce、受众与期限 |
| 私有侧 peer | 管理员登记的 peerId 与凭据/证书 | 仅可注册被允许的 provider、能力与资源 |
| 本地/inproc/stdio | `(local, machineId, OS-user-id)` | 启动方传入已建立的主体上下文；本地执行仍受策略约束 |

所有键再映射到内部 `principalId` 和明确的 `tenantId`。不同 issuer 的同名 sub 不能合并；OIDC 唯一性依据是 `(iss, sub)`，见 [OIDC §5.7](https://openid.net/specs/openid-connect-core-1_0.html#ClaimStability)。token 类型和用途必须验证，不能把任意 ID token 当作任意 API 的访问 token。

设备与 OIDC 绑定凭证由受信任的身份绑定权威签发，包含 issuer、subject、tenant、设备公钥、audience、bindingId、有效期和撤销引用。签发前同时验证新鲜 OIDC 登录和设备对一次性挑战的签名；解绑/换钥撤销旧 binding。凭证表达身份关联，空间访问仍由 ACL 授予。离线场景不能承诺实时获知绑定撤销，按空间 ACL 的有效性规则处理。

### 8.2 出站凭据

| mode | 持有者与语义 |
|---|---|
| `none` | 不需要上游秘密；不意味着绕过 conex 授权 |
| `static-secret` | 安装配置指定 host 或私有 peer；无法单凭该 key 区分上游用户 |
| `oauth-cc` | 服务凭据持有方交换服务访问 token；审计同时保留原调用主体 |
| `oauth-user` | 被明确授权的 host 或 peer 存储用户授权及 refresh token |
| `delegated` | 按上游支持的 token exchange 获取限定受众/范围的临时 token |
| `mtls` | 指定运行环境持有证书与私钥 |
| `agent-held` | 上游秘密仅在私有 peer，host 转发经授权的调用 |
| `space-key` | 内容密钥仅在空间成员设备；不承担普通上游登录 |

Credential 的解析键至少包括租户、持有者、provider、目标受众和引用名；用户授权凭据还绑定 Principal。连接/token 缓存必须包含这些身份和权限边界，不能仅以 providerId 缓存后跨用户复用。清单只能引用安装阶段允许的凭据名。

入站控制链路的注册凭据与出站业务凭据分开。后者在对端身份、路由和调用权限确认后才能解析和发送；agent-held 在私有 peer 完成此步骤。凭据刷新、撤销和重新授权不能扩大原 Session 的 scope。

### 8.3 授权上下文与方法契约

```text
CallContext {
  principalId, tenantId, actorPeerId, providerEndpointId, plane,
  sessionId?, workspacePeerId?, humanPeerId?, parentCallId?,
  resource: { spaceId?, resourceId, action, scope },
  policyVersion, deadline, credentialBinding?
}
```

host 不必内置所有 provider 业务类型，但必须调用已安装的 `MethodContract`：

1. 校验参数 schema，提取并规范化授权资源；例如 workspace 路径相对哪个根、搜索限制在哪些 source、blob 归属哪个资源。
2. 从认证连接、Session 绑定与管理配置派生主体和执行者；将调用者提出的 context 与实际参数交叉验证。未知方法或无法提取授权资源时 fail closed。
3. 计算 Claim 并作策略判定；将最小范围和剩余 deadline 传给执行方。跨进程的内部授权材料绑定 audience、会话/操作和参数摘要，接收方验证且仍执行本地资源策略。
4. 实际执行时重新验证路径/句柄与授权对象一致，防止路径穿越、符号链接替换及检查后资源变化。清单中的能力声明不是授权证据。

回调只能使用原 Session 的 workspace/human 绑定，继承原用户并记录发起回调的 agent；父调用、权限选项、参数摘要和到期时间共同绑定一次用户批准。其他窗口、其他用户或重放的批准不能被消费。UI 断开时暂停到明示期限或拒绝，不能默认允许。

### 8.4 分阶段调用流程

```text
认证调用者
 → 规范化参数并生成 CallContext
 → 本地方法/资源授权 + 平面检查 + 目标地址准入
 → 选择满足 requires 的已安装 Profile
 → 建立或复用已认证 Link，校验独立于发现结果的预期 peer 身份
 → hello/ready（新 Link）及能力检查
 → 当前策略复核，解析绑定目标的业务凭据
 → 执行 / 跟踪结果 / 审计
```

本地预检可以读取策略与认证基础设施；“零触网”验收精确定义为**零业务目标拨号、零业务凭据解析**，不包含必要的 JWKS/身份服务访问。对目录、DNS 和注册的访问有独立控制面策略。

地址准入检查 scheme、端口、DNS 解析后的 IP、允许的网络范围与实际连接地址；重定向必须重新检查且不自动转发凭据，抵御目录投毒和地址重绑定。复用连接也必须核对身份、受众和授权边界。服务端证书证明域名控制权，不自动证明该域名被授权充当某个 provider。

`relay` 仅开放经过逐项登记的零知识方法：blob 存取、签名 DAG 同步及 ACL 证明验证等。`source/*` 明文查询必须在 broker Session 中执行。两平面之间的解密/索引是显式导出授权，不能凭同一 providerId 或相同 CID 自动打通。

### 8.5 浏览器登录与 ticket

浏览器采用 authorization-code + PKCE。host 的 Web 登录会话 cookie 使用独立、随机且不可预测的值，并设置 Secure、HttpOnly、SameSite；它与业务 sessionId 是不同概念。登录校验 state/nonce；修改状态的 HTTP 请求和 `/tickets` 实施 CSRF 与 Origin 校验。

`POST /tickets` 在已认证 Web 会话中签发 30 秒、一次性 ticket，绑定主体、Web 会话、目标 host、peer 角色、能力上限及可选 Session。WS 握手校验 Origin 并原子消费 ticket；断线后重新获取，不延长或重复使用已消费 ticket。

WS URL 可以携带短期 ticket，但接入代理、访问日志和诊断必须脱敏 query。IP/UA 只作异常信号，不作为身份凭证或强制绑定条件，避免 NAT/移动网络变更破坏合法重连。长期 token 不进入 URL。

### 8.6 审计与可观察性

审计字段基线：`eventId, ts, traceId, principalId, tenantId, actorPeerId, providerEndpointId, method, resourceScope, plane, policyVersion, phase, outcome, errorCode, diagnosticId, credentialRef, sessionId, requestId, operationId, execution, latencyMs`。发现拒绝追加 source、经脱敏的 evidence 摘要和目标身份预期。

不记录 token、ticket、参数原文或内容密钥。CID/内容摘要本身也可能暴露关联信息：默认省略，需要内容追踪时采用租户隔离访问与保留策略，不能把低熵参数的裸 hash 当作安全脱敏。

必须可观察：每主体/provider 的调用延迟与拒绝、在途数、队列和重放字节、可恢复范围、流 reset、未知操作数、子进程数、base64 转换内存、上传 staging、GC 与副本健康。审计出口有界；安全审计无法可靠接收时拒绝新敏感操作，已开始操作不因审计重试被重新执行。P0 使用本地审计出口，P4 增加查询与外送。

## 9. 场景落地

### 9.1 数据 provider

P0 统一 `source/list`、`source/read`、`source/search`，每个方法定义资源范围、分页、deadline、大小上限及 revision 语义。搜索结果返回来源、资源标识与可获得的版本；上游不提供可靠版本时显式声明，不能伪造 fresh 或条件写能力。

聚合返回 `items` 与 `providerResults[{endpointId,providerId,status,error?,nextCursor?}]`，区分真正空结果与部分失败；`endpointId` 是路由与审计键，`providerId` 只是展示用归属，两者不得混用。聚合语义只属于 core（`Host::invoke_many`）与显式声明的聚合端点；SDK 的 `searchMany` 只是对多个单目标调用的逐项封装，不是第二种聚合语义。每个 provider 有独立并发、超时和队列预算；一个失败不得中止所有已成功结果。查询 scope 必须在各 provider 执行前约束，不能先读取全量敏感数据再过滤。

P1 增加 `source/write` 与可选 `expectedRevision`；后者只有在上游能原子检查时可用。Anytype 通过已安装的 API 桥接器接入，凭据默认放私有侧；其可用端点与具体版本在接入时锁定，不实现 any-sync 线协议。

### 9.2 ACP 网关

| 来源 | 外部方法 | conex 方法 | 固定方向/归属 |
|---|---|---|---|
| ACP | `initialize` / `authenticate` / `logout` | `agent/initialize` / `agent/authenticate` / `agent/logout` | 桥接器管理的 ACP 连接与登录状态 |
| ACP | `session/new` / `session/load` / `session/prompt` / `session/set_mode` | `agent/session.new` / `.load` / `.prompt` / `.set_mode` | consumer → 指定 agent provider |
| ACP | `session/cancel` | `agent/session.cancel` | consumer → agent，通知 |
| ACP | `session/update` | `stream/update` | agent → Session 绑定的 consumer |
| ACP | `session/request_permission` | `human/request_permission` | agent → Session 绑定的界面 peer |
| MCP | `elicitation/create` | `human/elicit` | 上游 server → Session 绑定的界面 peer |
| MCP | `elicitation/*`（具体方法名待 P2-01 按锁定 schema 核实） | `stream/elicitation_complete` | 同一界面 peer，通知 |
| ACP | `fs/*` / `terminal/*` | `workspace/fs.*` / `workspace/terminal.*` | agent → Session 绑定的私有侧 peer |

表中简写 `.load` 等共享 `agent/session` 前缀；`来源` 列区分 ACP 原生与 MCP 原生方法，两者不得混排。P2-01 必须先按锁定的外部 schema 逐条核实方法名、方向与存在性（尤其 `elicitation/*`、`session/set_mode`、`logout`），核实前不得把上表当作已冻结契约。具体可用方法、参数和扩展按冻结的 ACP/MCP schema 定义，未知必需能力拒绝。桥接器是 ACP Client；UI 请求由桥接器的生命周期状态机校验，不能任意重置一条已有 ACP 连接的 initialize/auth 状态。

- ACP client 能力由该 Session 实际绑定的 workspace/human 功能推导；不能广告整个 host 上其他用户可用的能力。ACP 的初始化区分 client 与 agent 能力，见 [Initialization](https://agentclientprotocol.com/protocol/v1/initialization)。
- `logout` 是连接认证方法，不是 `session/logout`。默认 agent 类型认证由 agent 处理，协议调用提交 `methodId`；terminal 类型需要私有侧按已配置的 agent 程序启动独立交互进程，成功后重新连接和初始化，不能发送 terminal methodId 给 `authenticate`。见 [Authentication](https://agentclientprotocol.com/protocol/v1/authentication)。
- `api_key`、device-code 或浏览器跳转属于具体 agent 支持的登录流程；conex 不假定所有 agent 都支持通用凭据表单。agent-held 模式的秘密通过私有侧登录入口收集，集中 UI 仅接收进度或非秘密交互信息。
- 原生 ACP stdio 采用 UTF-8 JSON-RPC 换行分帧，stdout 只承载协议，stderr 单独有界采集；conex 帧不写进 ACP stdin。见 [Transports](https://agentclientprotocol.com/protocol/v1/transports)。
- 会话到期或关闭回收独占进程组；网络短暂失联先保留到租约期。无法恢复的输出显式标记缺口，不重跑 prompt。权限批准的重放遵循 §8.3。

### 9.3 浏览器与私有执行边界

本产品把界面交互放浏览器，把文件系统、shell、登录凭据与 ACP 连接生命周期放受控私有 peer/桥接器。浏览器只暴露界面能力；workspace 根目录与进程启动许可来自管理员配置和用户授权。

这不意味着 ACP 规范禁止浏览器实现 Client，也不意味着“浏览器不是 Client”本身能证明安全。真正的不变量是：回调与批准绑定正确的主体、会话、workspace 和参数；页面不能通过注册能力获得任意本机文件或命令执行权。

## 10. 可选 P2P：历史、权限与持久性

### 10.1 参考边界与双平面

any-sync 将加密历史同步与设备侧对象处理分开，节点保存密文，索引/查询受设备能力约束；这是 conex 借鉴的机制。conex 的节点格式、ACL 和网关协议独立定义，不宣称线协议兼容或相同保证。参考 [any-sync Overview](https://sync.any.org/)。依赖清单只能证明库被引用，不能证明任意承载组合可用、默认算法或具体运行路径。

| | relay | broker |
|---|---|---|
| 数据 | 密文封装、CID、签名与公开关系元数据 | 明文或明确授权可解的内容 |
| 处理方 | 成员设备解密；节点存储、复制、验证公开结构 | 授权 provider/host 读取、搜索、转换 |
| 授权 | 签名 ACL、设备身份与新鲜度证明 | Principal、资源策略与上游凭据 |
| 隔离 | 无内容密钥，不能隐式转为明文方法 | 不能沿用 relay 权限直接获取解密能力 |

双平面统一检索要求设备或受信任导出方显式发布 broker 投影，记录授权范围、导出版本、派生索引及撤销处理。撤销停止未来导出并触发受控缓存清理，不能追回已被合法接收者复制的明文。该能力放 P4。

### 10.2 Object、变更 DAG 与冲突

P3 首版只支持具名的 `object-mv-v1` 合并规则；不提供任意 CRDT 插件在同一对象上互换的承诺。

```text
ChangeHeader { formatVersion, spaceId, objectId, actorKeyId,
               parents[], aclVersion, keyEpoch, payloadCid }
Change       { header, signature }
ObjectState  { objectId, heads[], properties, conflicts[], deletionState }
```

objectId 在创建时生成并保持稳定；Change 的 CID 对完整签名变更记录计算。为使同一逻辑变更（同 header、同 actor）产生稳定 CID，P3-a 加密 Profile 必须选定确定性签名，或规定签名结果的确定性编码；否则 CID 只覆盖 header，签名作为并列字段参与校验，不得把随机化签名的字节计入去重键。签名使用独立 domain 标识并覆盖 header 的规范化字节，包括 payloadCid 与 parents，防止跨空间、跨对象重放。业务属性操作在加密 payload 中，设备验证签名、权限、完整依赖和内容后投影；缺少父变更时暂存为不完整，不提前应用。

- 变更集合按 CID 去重，parents 定义因果顺序；不能用设备墙钟决定赢家。
- 属性是多值寄存器：有因果关系的新值替代它已观察到的旧值；并发同属性写入保留多个值并返回 conflict。不同属性的无冲突写入可共同投影。
- 解决冲突必须提交显式变更，引用所有被解决分支；读取方不能各自选择不同的“最后值”而声称一致。
- 对象删除用 tombstone 变更表达；删除与更新并发时保留显式冲突，不静默复活或丢弃更新。重复同名创建使用不同 objectId 并共存，不以名称去重。
- schemaVersion/mergeVersion 随对象创建确定。未知版本可保留和转发原始字节，但不能投影成空对象；schema 迁移使用明确操作，新语义不得追溯改变已有历史。
- `expectedRevision` 对本地已验证 heads 的比较只保证本地条件写；离线期间不可能阻止远端并发分支，重连后仍可能产生 conflict。

DAG 的集合合并和业务状态投影分别验收：输入顺序、重复和分批方式不影响最终 heads 与多值冲突集合。首版不实现文本序列 CRDT、自动语义合并或后台历史压缩。

### 10.3 ACL 权威、定序与撤销

每个 Space 的 genesis 固定 ACL 权威公钥、成员起点和加密 Profile。首版使用单个逻辑权威签发严格递增的 ACL 版本链：

```text
AclEntry { spaceId, version, prevAclCid, members, roles,
           keyEpoch, cutoffHeads?, authorityKeyId, signature }
```

- 权威只定序成员、权限、密钥 epoch 和撤销切换的边界，不定序每次数据写入；其密钥与目录协调者独立。
- 权威必须持久化版本链并防止双主签发；备份切换先 fencing。看到同一 version 的不同有效签名内容视为权威分叉，冻结新的在线授权，不用“取较大时间戳”裁决。
- 权威离线期间不进行成员变更。设备可保留本地离线编辑；重新发布前必须同步到最新 ACL 并验证，不能将本地临时成功误报为全网接受。
- 在线节点的 ACL 新鲜度证明由权威签发，绑定 space、ACL head、签发与到期时间；默认最长 60 秒，允许时钟偏差最多 5 秒，超限或不能确认时间时 fail closed。节点保留已见最高 ACL 版本，拒绝回退。
- 撤销生效以各遵循协议的节点获取新 ACL 或旧证明到期为界；最坏新授权窗口为 60 秒加容许偏差。过期节点拒绝新的下载、写入及续租，并暂停/终止受影响的在途流。已传输字节不能撤回。
- 撤销读权限必须切换 key epoch，并仅向保留权限的成员封装新密钥；只更新 ACL 不足以阻止持有旧密钥的成员读取旧密文。
- key epoch 的分发协议必须与 ACL 版本链一同定义：成员设备公钥的来源与签名绑定、新 epoch 内容密钥对哪些成员公钥封装、离线成员重新上线时如何以已验证 ACL 换取新密钥、以及撤销成员不能凭旧 proof 取得新封装。该协议与封装套件同为 P3-a 发布门槛，缺任一项不得广告 Space。
- 每次缩减成员权限都记录认可旧历史的 `cutoffHeads`，包括只撤销写权限而不换内容密钥的情况。旧 ACL 版本下的已认可祖先仍可验证与读取；其余迟到离线写入不进入新的共享状态，保留为本地分支。有权限的成员可显式重新提交并引用新 ACL 版本及当前 key epoch，已撤销成员不能通过回填时间戳恢复写权。ACL 版本与内容 key epoch 是不同序列，不可混用。

这是对离线写入与及时撤销的明确取舍。成员的旧明文/密钥不能远程擦除；恶意节点可以拒绝服务或继续分发已持有字节。内容机密性、历史完整性与存储可用性分别陈述和验收，不用“节点不能封禁用户”概括三者。

### 10.4 节点迁移与数据恢复

Space 配置目标持久性等级；跨节点迁移必须按以下次序执行：

1. 固定待迁移的根/heads 快照，并在源或完整客户端副本上取得覆盖迁移期的保留租约。
2. 向新节点复制全部可达变更、manifest、blob 和验证所需 ACL 历史；按 CID 校验并取得目标持久性接收凭证。
3. 追赶迁移期间新写入的增量；在切换检查点确认目标完整，或暂停写入后完成最后一次追赶。
4. 原子更新受信任的节点配置/首选位置，验证从新节点可读取内容；最后释放旧副本 pin。

并发 GC 不得删除上传/迁移租约覆盖的块。若旧节点提前消失，只有其他完整副本才能补齐；否则返回明确 missing CIDs，不能以“已持 heads”报告迁移成功。客户端备份同时包含内容、密钥、ACL 与根引用，单独导出密钥不算数据备份。

## 11. 发现与公共协调

### 11.1 候选来源

| 层 | 机制 | 默认与阶段 |
|---|---|---|
| L1 | 静态安装配置/邀请中的地址与预期身份 | P0 可用；不需要独立目录服务 |
| L2 | mDNS `_conex._tcp` 的本地广播 | P3-b，默认关，可限定网络接口 |
| L3 | 公共或自建 `conex-coordinator` | P3-b，显式配置后启用 |
| L4 | DHT | P4 后按规模需求评估，不阻塞 P3 |

```text
Candidate { peerId, endpointId, addresses[], profileIds[], provides[], plane,
            spaceId?, expiresAt, source, evidence }
```

Candidate 是未授权声明。来源证据可包括 peer 对声明的签名、目录应答签名、来源接口与观测时间；TLS 证书和设备签名仍在实际连接时校验。空间的权威成员状态来自 §10.3，目录只能返回带来源的缓存提示。

缓存键至少包含租户/发现策略域、plane、目标身份约束、方法需求与 space。不能把一个租户已授权的发现结果直接复用于另一个租户。结果过期重新查询，失败节点有限退避；不能因失败改用未授权地址或 provider。

### 11.2 协调者职责与方法

协调者负责目录及可达性线索，不持业务内容密钥/上游凭据、不签数据变更、不承担 ACL 权威。它可以持自己的服务签名密钥与注册认证记录；“不持凭据”不表示服务无需认证。any-sync coordinator 可作机制参考，不能因同名就推断职责完全相同，见其[实现仓库](https://github.com/anyproto/any-sync-coordinator)。

| 方法 | 行为与授权 |
|---|---|
| `discover/announce` | 已认证 peer 注册/续租自身被允许公布的端点；绑定身份、声明序号、期限与签名 |
| `discover/query` | 在查询者允许的目录范围内按方法、Profile、plane、space 查询候选 |
| `discover/space` | 返回节点候选和带 ACL 版本/来源的成员提示，不能作最终授权依据 |
| `discover/hint` | 提供中继/可达性候选，不授权自动连接或凭据发送 |
| `discover/revoke` | 声明所有者或目录管理员移除目录记录；不撤销业务权限 |

默认续租周期 60 秒，目录租约 180 秒；revoke 后用带单调声明序号的墓碑拒绝旧 announce 重放。多目录可各自持有记录，revoke 需发给相关目录；客户端已缓存记录不会被保证瞬时清除，只能在期限内失效。目录时钟异常需显式报错。

### 11.3 从发现到调用

```text
候选
 → 本地 plane / 身份预期 / 出站地址策略 / 调用权限预检
 → 拨号并验证对端身份
 → hello / ready 协商
 → Session 绑定与当前业务授权
 → 解析目标绑定的业务凭据并调用
```

三种拒绝分别验收：

- 预检拒绝：零候选目标拨号、零业务凭据解析。
- 身份或握手拒绝：允许已经发生连接，但必须零业务请求、零业务凭据泄露。
- 业务授权拒绝：连接可保留供其他已授权用途，本次业务不执行，拒绝完整审计。

### 11.4 威胁与可替换性

目录被攻破可能泄露注册元数据、隐藏候选、重放未到期地址或影响可达性。内容真实性和授权只有在客户端独立验证身份、可信根、签名、ACL 与新鲜度时才不依赖目录；目录签名只证明来源，不能自证其内容可信。

查询/注册需要范围控制、限流、响应大小与租约数量上限。mDNS 默认只公布粗粒度服务信息，不广播空间成员/对象清单。`mdns=false` 表示本进程不广播、不监听该发现渠道，不承诺其他静态目录或网络扫描无法看到它。

杀掉全部协调者后，已建立且仍获授权的 Link/Session 继续工作；缓存过期后的新发现失败，重新拨号是否成功取决于仍有效的地址。ACL 证明续期是独立依赖，不能因目录可替换而绕过 ACL 新鲜度检查。

P2P 第一版以静态可达地址和受控 WSS 中继为连通性基线；打洞候选不保证 NAT 穿透成功，失败明确回退到已授权中继或返回 unavailable。中继只转发受端到端保护的流量；它的地址即使由目录签名也必须过独立地址策略。

## 12. 部署拓扑

| 拓扑 | 形态与适用 | 关键边界 |
|---|---|---|
| T1 内嵌 | host + provider 同进程 | 同一授权/审计入口；inproc 不绕过策略 |
| T2 反连 | 私有 agent 主动连接公网 host，主场景 | 先验证 host；私有侧保存凭据与 workspace |
| T3 直连 | host 访问受信任 provider/API | 出站地址和凭据目标绑定 |
| T4 sidecar | 私有 peer 与上游同机 | 只安装所需适配器，限定目录和进程 |
| T5 节点复制 | P3 存储/中继节点 | 明示副本数、故障域和恢复前提 |
| T6 双平面导出 | P4 relay Space 显式导出 broker 索引 | 新授权、新 Session、派生内容撤销策略 |
| T7 公共目录 | P3-b 跨网候选与可达性提示 | 目录不是 ACL 或 provider 身份信任根 |
| T8 局域网发现 | P3-b 用户开启 mDNS | 免手工地址配置，仍需预置邀请或身份授权 |

P0–P2 默认单 host；运行态会话不承诺跨 host 漂移。生产 HA 必须单独提供共享状态/会话归属、fencing 和持久恢复实现，不能仅在前面增加负载均衡就广告无损恢复。

## 13. 类型源、版本与一致性验证

### 13.1 唯一类型源与语言映射

conex 自有规范类型源放 `schema/conex/v1/*.proto`；JSON Schema、Rust/TS/Python/Go 类型和字段文档均由其生成，禁止同时手改 JSON Schema。外部 ACP/MCP 类型仍以锁定的上游 schema 为准，不由 conex 重新定义。状态机、授权、JSON-RPC 映射和持久性要求由本文及相应 Profile 规范定义。类型源与行为规范出现冲突时阻止发布并修正，不允许实现自行选一边；向量不覆盖规范。

- 标识统一 string；计数/序号使用无符号 64 位整数，JSON 映射为十进制字符串，禁止经 JS Number 丢精度。
- optional 与缺省值有区别；字段为 0/false 不等于未提供。null 只在 schema 明确允许的位置出现。
- 可扩展枚举保留未知值并显式拒绝无法理解的必需语义；不能反序列化成默认 allow/ok。
- 错误、元数据和外部原值都有字节/深度上限。签名内容使用专门内容格式，不由 ProtoJSON 的普通映射决定。
- 类型、schema、映射和黄金向量作为一个版本集合发布；生成结果差异进入 CI 检查。P0 至少验证 Rust 与 TypeScript 两个独立实现，其他 SDK 在各自发布时加入。

### 13.2 协议演化

`conex/1` 主版本尚未冻结；发布前允许 v6 后续修订。冻结后，任何使既有合法报文变非法或改变可观察行为的变更都要兼容迁移或升主版本，包括授权范围、字段类型/必需性、ID、流、取消、错误、CID 和平面语义，不限于四个字段类别。

新增可选能力/方法通常可兼容，但必须明确旧客户端收到新错误/未知枚举的行为。内容格式版本、加密 Profile、外部协议版本与 conex 主版本分别标识。外部 ACP/MCP schema 必须保存来源版本或 commit、下载日期及内容摘要；不能用“站点最新”作为固定 conformance 输入。

### 13.3 一致性向量与故障模型

`conformance/vectors` 按核心、Profile、内容格式、协议桥接与可选模块分类。每个实现必须通过核心及其声明能力的向量；不要求同步 HTTP 伪装支持回调，不要求纯 consumer 实现全部 provider 方法。

| 类别 | 必须验证的性质 |
|---|---|
| 消息 | 请求/响应/通知、双向 ID 碰撞、外部字符串/数值 ID 保真、未知字段边界 |
| 协商 | 固定引导、非法切换、必需能力缺失、方向性能力、不可接受降级 |
| 授权 | 不同 issuer 同名 sub 隔离、跨租户凭据隔离、回调/恢复归属、资源参数一致性 |
| 发现 | 本地拒绝零拨号、握手拒绝零业务数据、恶意目录不能替换信任根 |
| 流 | 乱序/重复/窗口边界、重复 flow 不增加信用、慢消费者、控制通道可取消 |
| 操作 | 响应丢失、执行后崩溃、同幂等键不同参数、过期重试、未知结果不自动执行 |
| 内容 | CID 已知答案、规范化字节、逐块校验、续传、commit/GC 竞态、隔离的访问权 |
| sync | 因果缺块、多值冲突收敛、删除与更新并发、ACL 分叉/回退/过期、撤销及迁移 |
| 桥接 | 固定外部 schema 的示例往返、真实上游流程、进程退出和输出缺口 |

本地 CI 负责崩溃注入、时钟、网络与存储故障；`conex conformance --target` 只验证端点支持的黑盒能力，不冒充能检测端点内部事务或恶意存储节点。故障与部分恢复结果必须进入审计/指标。

## 14. 阶段与验收

### P0 — 可用的 broker 契约主干

范围：最小 `conex-proto/core/host`、inproc 与 JSON-RPC/HTTP、只读 fs provider、第二个只读 provider、静态安装注册、方法级授权提取器、env/file 凭据、本地审计、CIDv1 raw/SHA-256 类型。P0 不提供 Session 恢复、blob 服务或 sync；不提前实现未来全部端口。

验收：

1. fs 与第二个 provider 通过同一 Registry 调用；新增适配器、注册、依赖和向量即可，core 执行路径没有新增 provider 特判。
2. Rust 与 TS 的请求/响应/通知、ID、错误、数值与 schema 生成向量一致。
3. 未授权读取在业务目标拨号/业务凭据解析前拒绝；不同 issuer 同 sub、不同租户凭据名不串用。
4. fs 的资源路径授权与实际读取一致；未知/未实现能力明确拒绝。
5. 聚合中单 provider 超时返回部分结果与来源错误；真正无结果返回成功空列表，二者可区分。
6. CID 已知答案向量通过；只有已实现的方法/Profile 被注册和广告。

### P1 — 二进制、反连、会话与条件写

范围：两个 WSS Profile、conex-agent 反连、blob 分块/持久提交/pin/GC、有界流与网络恢复、浏览器 ticket、Session 与操作状态、具备真实原子前提的条件写。需要持久性的上传与操作状态使用本地持久存储；不因此承诺运行中会话跨进程恢复。

验收：

1. 1 GiB 文件在声明至少 1 GiB 上限的端点间经 protobuf blob 通道弱网续传；内存/临时磁盘/队列受限且指标可见。
2. 坏块被拒；上传或 commit 时崩溃不会得到缺块却 committed 的根；并发 GC 不删除有效上传/资源引用。
3. 窗口内断网 30 秒恢复同一 Session，旧连接被 fencing；超窗明确 reset，host 重启返回 session_lost。
4. 重复 ACK/flow 不放大信用；慢消费者停止发送且控制取消仍可达，超限返回 slow_consumer。
5. 伪 host 无法取得 agent 注册/业务凭据；日志无 token/ticket；其他主体不能凭 sessionId 恢复。
6. 模拟上游写成功后响应丢失：有上游幂等支持时只执行一次并取回结果，无支持时 outcome_unknown 且不自动重试。
7. inline/ref 往返的解码内容字节相等；受限上游不能承受大内联时明确拒绝。

### P2 — ACP 网关与 MCP 桥接

范围：ACP stdio 桥接、workspace/human 固定绑定、独占子进程、agent/terminal 登录流程、MCP 指定版本桥接；host-held OAuth 仅按被接入上游的实际需求增加。

验收：

1. 真跑 ACP CLI：initialize → 支持的认证 → session/new → prompt → update → permission → 完成。
2. 两用户/两 workspace 并行运行，能力、回调、文件、凭据和权限批准均不串用；缺 terminal 不广告。
3. 网络窗口内恢复不重跑 prompt；输出超窗标记不完整；ACP 进程退出不声称恢复成功。
4. terminal 登录使用已配置程序的独立交互进程并重新初始化；logout 映射正确。
5. Session 关闭/过期回收进程组；一个会话退出不影响其他会话。
6. ACP/MCP 各自固定版本向量通过，包括外部 ID、错误、_meta 与内容；原生上游不接收 conex 帧。

### P3-a — 独立 sync 与零知识存储

前置：冻结内容/加密 Profile 及向量、ACL 权威与截止历史格式、object-mv-v1 规则、持久性/GC 模型。这些是独立评审的发布门槛，不阻塞 P0–P2。

范围：Space、签名变更 DAG、多值属性冲突、单逻辑 ACL 权威、密钥 epoch、conex-node、WSS 可达节点、显式副本与迁移。

验收：

1. 离线并发写同属性保留多值冲突；不同属性合并；删除/更新并发和重复/乱序输入收敛到同一状态。
2. 检查节点持有的密钥、日志、存储和网络边界：无内容解密密钥/业务明文；公开元数据符合约定。该检查与加密 Profile 向量共同验证，不仅依靠“看起来是乱码”。
3. 两个独立故障域副本持久提交后损失一个节点仍能恢复；仅剩 heads 而缺所有副本时明确报告不可恢复缺块。
4. ACL 撤销按 60 秒加容许时钟偏差的上界在遵循协议的在线节点生效；过期证明拒绝、旧版本不能回退、旧 ACL 下截止历史外的迟到分支不会自动进入新状态。仅撤销写权限、不换内容密钥的场景同样通过。
5. 原节点运行 GC 时迁移仍完整；目标未达到持久性前不能释放源保留。relay 不执行明文 source 方法。

### P3-b — 发现与可达性

范围：静态邀请/身份配置、可选 mDNS、公共/自建协调者、受控 WSS 中继。可以独立于 sync 用于 broker peer 发现。

验收：

1. 假目录返回越界地址/错误 plane 时预检零拨号；返回允许连接但身份/能力不符的节点时零业务请求和秘密泄露。
2. 杀掉目录后已有会话在自身授权/租约内继续；新发现无可用来源时明确失败。
3. mDNS 开启且身份已授权时自动发现并调用；关闭时没有该进程的 mDNS 广播/查询，不宣称整个网络互不可见。
4. 租约到期记录从目录查询消失；单目录 revoke 即时移除本地记录并拒绝旧序号重放，多目录/客户端缓存遵守各自期限。
5. 打洞失败能使用已授权中继或返回 unavailable；不能绕过地址与身份验证。

### P4 — 按实际需求生产化

OS keyring/凭据 broker、审计查询、部署 HA 与持久恢复、双平面授权索引、QUIC/WebTransport、SSE+POST conex Profile、可选 DHT。各能力单独声明前置契约和验收，不作为一次性“生产化”打包承诺。P0–P3 的基本认证、配额、有界队列和审计不延后到 P4。

## 15. 与 notez 的关系

以下路径与结论继承 v4 对 notez `588bfb3` 的审查记录，本轮未复核。它们用于说明待验证的工程风险；实现时应在固定提交上重新核对，不以行号记录替代测试。

notez 作为参考和候选接入方，不作为 conex 的强制依赖。P0 的第二个 provider 不依赖 notez 接入进度。

### 15.1 借鉴

| 借鉴点 | 证据（`588bfb3`） |
|---|---|
| 端口 + 注入纪律、无静默默认方法 | `crates/ports/src/lib.rs` |
| 五维授权形状 `Principal×Space×Source×Action×Scope` | `crates/engine/src/authorization.rs:11-37` |
| "一个操作一个 struct + schema 派生，绝不手写" | `crates/protocol/src/request.rs:1-19`（42 变体全 typed） |
| 错误必须机器可读 | `crates/protocol/src/error.rs`（16 变体） |
| streamable HTTP 承载形态 | `crates/mcp/src/http.rs` |
| OIDC client-credentials 令牌缓存 | `crates/api/src/client.rs:30-41,217-277` |
| 真实 OIDC 校验（JWKS 缓存 + issuer pinning + 仅 RS256） | `crates/api/src/auth/oidc.rs:103-202`、`crates/api/src/auth.rs:196` |
| 诚实的未实现适配器（报错而非假数据） | `crates/core/src/source/anytype.rs` |

### 15.2 反面教材（conex 不得重复）

| 反模式 | 证据 | 后果 |
|---|---|---|
| 有注册表但生产路径绕过 | `crates/core/src/application/use_cases_impl/scan.rs:44-53`；`crates/core/src/application/service.rs:502-506`；`crates/core/src/source/registry.rs:40-46` | "支持扩展"是假的 |
| 假内容寻址 | `crates/core/src/sync/engine.rs:137`；`crates/core/src/sync/object.rs:65-71` | 合并与恢复不可信 |
| 死码当接口 | `crates/core/src/sync/relay.rs:103-144,146-163`；`crates/core/src/source/protocol.rs:90-92` | 误判可用能力 |
| 身份在边界丢失 | `crates/api/src/auth.rs:189`；`crates/core/src/application/service.rs:551-556` | 审计与授权无从谈起 |
| 两套互不相关的秘密 | `crates/api/src/config.rs:55-57`；`packages/web/src/routes.rs:187` | 运维与审计口径分裂 |
| 中间件挂载不一致 | `packages/web/src/host.rs:90-92` vs `crates/cli/src/host.rs:538` | 同一功能因部署方式改变安全属性 |
| 单点故障传播 | `crates/core/src/application/use_cases_impl/scan.rs:197-218` | 一个坏 provider 拖垮全部 |

---

## 16. 已定默认与后续决策门槛

### 16.1 本轮确定的默认

| 决策 | 默认 | 取舍 |
|---|---|---|
| 主干范围 | broker 路由先行，sync 独立扩展 | 减少 P0 未经验证的端口和依赖 |
| 内容标识 | P0 即支持 CIDv1/raw/SHA-256 | 先固定互操作结构；BLAKE3 后续可扩展，不改变已有 CID |
| 类型源 | `.proto` + 明示 JSON/状态机规范，派生 JSON Schema | 避免两套手工类型源漂移 |
| 平面粒度 | 每能力端点固定，Session 固定 | 同 provider 双平面需要两个端点，减少缓存/凭据串用 |
| 初始凭据 | env/file；私有上游默认 agent-held | 不以 keyring 集成阻塞主干 |
| 恢复 | 网络恢复与跨进程恢复分别声明 | P1/P2 不假装具备持久会话恢复 |
| ACP 进程 | 每 Session 独占 | 先保证身份/workspace/登录隔离，再评估复用 |
| P2P 冲突 | 多值保留、显式解决 | 无自动最后写入覆盖；UI 后置 |
| ACL | 单逻辑权威 + 版本链 + 60 秒新鲜度窗口 | 可离线编辑，但离线发布与即时撤销不能兼得 |
| 发现 | 静态起步；mDNS 默认关；目录按需 | DHT、QUIC、WebTransport 不阻塞首版 |

### 16.2 各阶段开始前必须产出的具体契约

这些是明确的后续交付项，不是当前已存在的代码或 schema。本文用于约束实现计划；具体 wire 文件和安全 Profile 通过对应门槛后才允许发布能力。

| 门槛 | 必须完成的材料 | 阻塞范围 |
|---|---|---|
| P0 类型冻结 | 核心 `.proto`、JSON 映射、错误数值表、三个 source 方法的输入/输出/授权提取规则、HTTP binding 契约、第二个 provider 选择 | P0 实现计划中的相应任务 |
| P1 Profile 冻结 | 分块树/规范化 manifest 黄金字节、blob 消息与持久状态 schema、WS 序号/信用/恢复向量、操作记录后端及崩溃恢复策略 | P1 对外发布 |
| P2 上游锁定 | 实际 ACP CLI 与 MCP server、版本/commit/schema 摘要、支持的登录流程、workspace 资源模型 | P2 对外发布 |
| P3-a 安全与内容冻结 | AEAD/签名/派生/nonce/密钥封装套件及已知答案向量；ACL cutoff、对象多值操作、GC/迁移事务与存储后端设计 | P3-a 实现与发布 |
| P3-b 信任与运营 | peer 邀请/信任根、协调者签名轮换、目录元数据可见范围、中继连通性与成本预算 | P3-b 公共部署 |
| P4 单项评审 | HA 故障模型、持久恢复、双平面导出撤销、额外承载与 DHT 的各自收益/成本 | 对应可选能力 |

S3 兼容存储或本地后端必须用同一持久性接口验收，后端品牌不能代替提交语义。集中 UI 和 notez 接入角色可在 P2 选择；不以 UI 选择倒推或扩大协议核心。

## 附录 A：术语对照

| conex | ACP | MCP | any-sync 参考概念 |
|---|---|---|---|
| Provider | Agent | Server | 提供同步/存储的 node |
| Consumer / 协议桥接器 | Client | Client | client / SDK |
| `conex/hello` | `initialize`，独立状态机 | `initialize`，独立状态机 | handshake |
| Stream | `session/update` 等活消息 | 通知/流式响应 | 不等于历史 DAG |
| Object / heads | 无直接对应 | 无直接对应 | object / tree heads |
| Space | 无直接对应 | 无直接对应 | space |
| BlobRef | 桥接为协议支持的内容块 | 桥接为协议支持的内容块 | 文件块/内容地址 |
| human 方法 | permission / elicitation | elicitation | 无直接对应 |
| workspace 方法 | fs / terminal | 无直接对应 | 无直接对应 |
| coordinator | 无直接对应 | 无直接对应 | 目录/网络配置相关角色；职责不宣称相同 |

## 附录 B：参考与证据

规范核对日期为 **2026-09-15**。以下“本轮核对”只表示用于本轮设计修订的材料已阅读，不表示 conex 通过这些规范的一致性测试。实现阶段应按 §13 固定快照；本轮未下载外部完整 schema 或重跑 notez。

### B.1 本轮核对的协议与数据规范

- [JSON-RPC 2.0](https://www.jsonrpc.org/specification)：请求 ID、响应、通知、错误。
- [OpenID Connect Core §5.7](https://openid.net/specs/openid-connect-core-1_0.html#ClaimStability)：`iss` 与 `sub` 的唯一性边界。
- [CID 规范](https://specs.ipfs.tech/cid/)：CIDv1、multicodec、multihash 与文本表示。
- [Protobuf Serialization Is Not Canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/)：确定性序列化不等于规范化内容字节。
- [ACP Initialization](https://agentclientprotocol.com/protocol/v1/initialization)：双方能力与初始化。
- [ACP Authentication](https://agentclientprotocol.com/protocol/v1/authentication)：agent/terminal 认证流程及 `logout`。
- [ACP Transports](https://agentclientprotocol.com/protocol/v1/transports)：原生 stdio；Streamable HTTP 在该版本页面仍为 draft。
- [ACP Content](https://agentclientprotocol.com/protocol/v1/content)：内容类型与二进制字段。
- [ACP Session Setup](https://agentclientprotocol.com/protocol/v1/session-setup) 与 [Tool Calls](https://agentclientprotocol.com/protocol/v1/tool-calls)：会话与权限交互。
- [MCP Streamable HTTP 2025-06-18](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)：首个 MCP HTTP 桥接基线；不是“最新版本”声明。
- [any-sync Overview](https://sync.any.org/) 与 [coordinator 仓库](https://github.com/anyproto/any-sync-coordinator)：同步/设备处理的机制参考及独立实现边界。

### B.2 实现阶段的参考入口

以下为 v4 保留或补充的参考入口，使用时按目标版本重新核对：

- ACP：[文档索引](https://agentclientprotocol.com/llms.txt)、[Schema](https://agentclientprotocol.com/protocol/v1/schema)、[文件系统](https://agentclientprotocol.com/protocol/v1/file-system)、[终端](https://agentclientprotocol.com/protocol/v1/terminals)、[Elicitation](https://agentclientprotocol.com/protocol/v1/elicitation)、[扩展](https://agentclientprotocol.com/protocol/v1/extensibility)。
- MCP：[2025-06-18 Schema](https://modelcontextprotocol.io/specification/2025-06-18/schema)、[版本入口](https://modelcontextprotocol.io/)。
- 认证与承载：[OAuth 2.0](https://datatracker.ietf.org/doc/html/rfc6749)、[PKCE](https://datatracker.ietf.org/doc/html/rfc7636)、[Token Exchange](https://datatracker.ietf.org/doc/html/rfc8693)、[WebSocket](https://datatracker.ietf.org/doc/html/rfc6455)、[mDNS](https://datatracker.ietf.org/doc/html/rfc6762)、[ULID](https://github.com/ulid/spec)。
- anyproto：[any-sync](https://github.com/anyproto/any-sync)、[node](https://github.com/anyproto/any-sync-node)、[filenode](https://github.com/anyproto/any-sync-filenode)、[consensusnode](https://github.com/anyproto/any-sync-consensusnode)、[any-store](https://github.com/anyproto/any-store)、[go.mod](https://github.com/anyproto/any-sync/blob/main/go.mod)。依赖不作为运行行为证明。
- 内容：[multihash](https://github.com/multiformats/multihash)、[multicodec](https://github.com/multiformats/multicodec)、[multibase](https://github.com/multiformats/multibase)、[IPLD](https://ipld.io/)、[BLAKE3](https://github.com/BLAKE3-team/BLAKE3)。
- 可选承载：[libp2p](https://docs.libp2p.io/)、[yamux](https://github.com/hashicorp/yamux)、[QUIC 实现参考](https://github.com/quic-go/quic-go)、[WebTransport 实现参考](https://github.com/quic-go/webtransport-go)、[iroh Go 绑定](https://github.com/tmc/go-iroh)。它们不是 conex 的既定 Rust 依赖。
- 抽象方法：[SICP](https://sarabander.github.io/sicp/) 的原语、组合、抽象屏障与数据导向思想；不能由该方法论推导出任意协议组合天然兼容。
