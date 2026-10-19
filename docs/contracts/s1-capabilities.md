# S1：双向能力与状态订阅契约

> 状态：待评审的专项设计，基于已确认的 [目标架构](../design/2026-10-17-conex-architecture.md) S1；尚未实现、生成 schema 或执行运行验收。
> 范围：一个中心、TS/JS 与 Rust SDK、静态批准的节点和模块；不依赖工作台、插件运行时、完整用户管理或外部 SaaS。
> 本文新增 S1 方法与行为规则，不声明其已经生效。现有方法仍以各自 wire 契约为准；S1 不回退成 hello 问候或文件专用调用。

## 1. 决定与非目标

S1 用通用模块能力完成 TS→Rust、Rust→TS 的真实调用与状态订阅。两种 SDK 都可以同时提供、消费能力。中心只做认证、授权、契约校验、目录、寻址和中继，不实现计数器或其他领域逻辑。

采用以下保守决定：

- 提供端和状态订阅使用 `conex-jsonrpc2-wss`；先保证一条共同路径，不要求 S1 同时完成 protobuf 业务绑定。
- HTTP `POST /rpc` 继续存在，S1 允许查询目录、单次调用和取消；HTTP 不伪装成持久订阅，也不承担提供端注册。
- 一个受批准的 Node 同时只有一个活动提供端连接。断线不恢复正在执行的调用或订阅，重连重新注册并重新取快照。
- 状态发送完整值，不发送 patch，不合并中间更新；队列超限显式结束订阅。
- S1 仅接受 `read_only` 与 `non_replayable` 执行类别。不增加持久操作日志或去重执行承诺；已有 operation 方法的行为不由本契约替换。
- 不实现事件回放、P2P、动态 schema 下载、插件代码加载、自动路由迁移、卡片或审批引擎。

WSS 在远程使用 TLS；明文仅可按既有开发配置在 loopback 使用。S1–S3 仅受限开发环境使用，S4 安全管理面完成前不得作为公开多用户应用发布。

## 2. 原语与命名

| 名称 | 含义与作用域 |
|---|---|
| Principal | 认证所得主体；客户端不能通过消息指定自己是谁 |
| Node | 管理配置绑定的稳定执行参与方，不等于用户、显示名或连接 |
| Link | 一次已认证且完成 ready 的连接；连接关闭即失效 |
| Space / Integration | S1 由管理员静态批准的空间与接入标识，不提供成员管理 API |
| providerEndpointId | 沿用现有 wire 字段；在一个 Space 内精确标识一个 Module 实例，不另加同义的 moduleId 字段 |
| contractId | 中心和 SDK 预安装的不可歧义契约标识；注册声明不是安装契约或授予权限 |
| registrationId | 中心为每次成功模块发布生成的 ULID；绑定 Node、Link 代次、Space、接入和契约 |
| requestId | 每跳请求关联 ULID，位于 JSON-RPC id；不是幂等键 |
| subscriptionId | 发起方生成的 ULID，只在其认证连接内有效；每跳可转换，不是授权凭据 |
| revision | 某注册实例内特定状态源的 uint64 修订号，JSON 用十进制字符串 |
| seq | 某订阅内的连续更新序号，JSON 用十进制字符串；快照后从 `1` 开始 |

标识区分大小写。现有 ULID、严格解码和单一 `.proto` 结构类型源约定继续使用。JSON uint64 必须是 `0` 或不带前导零的正十进制字符串；不使用 JS Number 表示 revision/seq。

注册重建后 revision 可以重新从 `0` 开始；比较键必须包含 registrationId。revision 不等于时间戳，seq 不等于 revision。

## 3. 组合、抽象屏障与分派

消费者选择具体模块实例和契约，中心将其解析为已授权注册，再映射到当前连接。提供端通过 `(spaceId, providerEndpointId, contractId, method/state)` 表定位实现，不根据 `linux`、`github`、`counter` 名称建立内核特殊分支。注册表的模块主键为 `(spaceId, providerEndpointId)`，registrationId 由中心全局唯一生成。

| 屏障 | 上层知道 | 下层可变 | 可加性验收 |
|---|---|---|---|
| B0 契约 / handler | typed 输入输出和执行类别 | TS 或 Rust 实现 | 同契约换宿主语言，消费者不改业务数据结构 |
| B1 模块 / 路由 | endpoint、registrationId、Space | Link 和转发请求 ID | 新连接不能冒用旧注册，迟到回复不能完成新调用 |
| B2 状态 / 传输 | 快照、连续更新、结束 | socket 收发和队列实现 | 并发修改不产生快照窗口丢失；慢消费者显式中断 |
| B3 权限 / 发现 | 已验证授权和契约 | 目录索引与静态配置装载 | 知道 endpoint 或 registrationId 仍不能越权 |

S1 不实现 Plugin 生命周期。未来派生模块复用同样的发布与消费边界，不需要新的路由协议。

## 4. 认证、协商与静态批准

### 4.1 静态安全边界

中心配置分别声明：

1. 凭据摘要 → Principal、Node、允许的来源和连接角色。
2. Node 可发布的精确 `(spaceId, integrationId, providerEndpointId, contractId)` 集合。
3. Principal 可发现、调用和订阅的目标、契约方法及资源约束。
4. 各提供节点本地允许暴露的模块与方法。

未匹配即拒绝；不通过注册参数创建 Space、接入、节点或额外权限。组名、显示名、endpoint ID 和凭据引用都不是授权凭证。

中心在转发前检查调用者权限并严格校验输入；提供端只信任已认证中心连接附带的调用上下文，且再次检查本地权限上限。直接客户端提交 principalId/tenantId 视为 `bad_request`，不得只因字段存在就信任。S1 不声称静态配置等于 S4 用户管理。

### 4.2 握手

沿用 `conex/hello` → `conex/ready`，ready 完成前不能发布或调用业务。S1 中心支持以下请求能力：

`module/register`、`module/unregister`、`module/list`、`module/invoke`、`module/cancel`、`state/subscribe`、`state/unsubscribe`。

SDK 把实际需要的中心方法放入 hello.requires；提供端还在 hello.provides 中声明其实现的转发处理能力：`module/invoke`、`module/cancel`、`state/subscribe`、`state/unsubscribe`。支持订阅即同时支持本契约定义的 update/end 通知。

声明仅参与协商，不能替代批准或模块注册。必需方法被拒绝、S1 限额字段缺失或不合法时，SDK 停止 S1 连接，不降级成应用专用协议。HTTP 通过现有 hello/binding 协商其支持的方法。

同一 Node 的新提供端 Link 完成 ready 后先处于 staged，不立刻停掉健康旧连接。首个 module/register 通过全部批准与安装检查时，中心原子激活新代次、失效该 Node 的所有旧发布并结束旧订阅，再发布新模块；其余模块在新代次逐个注册。在途调用按 §7 处理。首个注册失败不影响旧连接；staged 在 timeoutMs 内未成功发布则关闭。旧连接迟到注册、heartbeat、回复和 cleanup 都必须检查内部代次归属，不得覆盖或删除新代次。代次由中心分配，节点不能自报抢占值。

## 5. 方法与信封

### 5.1 方法表

| 方法 | 形态 | WebSocket | HTTP | 作用 |
|---|---|---|---|---|
| module/register | 请求 | Node→中心 | 不支持 | 原子发布一个已批准模块 |
| module/unregister | 请求 | Node→中心 | 不支持 | 撤销当前连接拥有的精确注册 |
| module/list | 请求 | 消费者→中心 | 支持 | 列出当前可见的模块 |
| module/invoke | 请求 | 消费者→中心→提供端 | 支持入站 | 调用契约方法 |
| module/cancel | 请求 | 消费者→中心→提供端 | 支持入站 | 请求停止一个在途 invoke |
| state/subscribe | 请求 | 消费者→中心→提供端 | 不支持 | 建立快照和后续更新 |
| state/unsubscribe | 请求 | 消费者→中心→提供端 | 不支持 | 释放订阅 |
| state/update | 通知 | 提供端→中心→消费者 | 不支持 | 完整状态更新 |
| state/end | 通知 | 提供端或中心→消费者 | 不支持 | 一个订阅的终止原因 |

HTTP 对不支持的方法返回 `unsupported_capability`，不建立后台假订阅。列表、注册和取消等控制动作不能伪装成无 id 的通知；有效通知只有 update/end，绝不返回 JSON-RPC 响应。

### 5.2 信封与上下文

沿用 [P0 wire](p0-wire.md) 的四种 JSON-RPC 变体、ULID 请求 ID、无 batch、重复/未知字段拒绝和错误结构。本文定义 S1 新方法的严格字段集合，不放宽旧方法的解码。

请求 params 为 `{context, timeoutBudgetMs, input}`，timeoutBudgetMs 必须大于 0 且不超过协商上限。中心扣除已经消耗的预算后转发；不得跨跳重置成完整预算。

- 模块调用/订阅的 context 必须包含 `providerEndpointId`、`plane:"broker"`、`spaceId`、`registrationId`。HTTP 另带有效 bindingId。
- 注册、撤销与目录的目标在 input 中，context 只包含 `plane:"broker"`；HTTP 目录另带 bindingId。
- cancel/unsubscribe 的 context 与原操作相同；中心以原调用/订阅记录为准，拒绝不匹配目标。
- 中心到提供端还附带验证后的 principalId、tenantId；它们不由业务 input 传递或覆盖。
- update/end 的 params 为 `{context,input}`，省略 timeoutBudgetMs；它们没有请求截止时间。这个规则只适用于 S1 通知，schema 的默认 0 不代表一个合法的 S1 请求预算。

新增 context 字段占用现有 RequestContext 空闲 tag：`space_id=6`、`registration_id=7`。不复用 rpc.proto 中预留的 operationId/meta tag。信封仍用既有字段，S1 消息类型在 `schema/conex/capability.proto` 定义，计数器领域消息在 `schema/conex/counter.proto` 定义；这两个文件本轮不创建。

## 6. 注册、撤销和发现

### 6.1 注册

`module/register` input：

- `spaceId: string`
- `integrationId: string`
- `providerEndpointId: string`
- `contractId: string`

中心先验证 Node/连接代次、批准项、契约存在、本地方法支持和限额，再原子安装整个模块。成功返回 `{registrationId, providerEndpointId, spaceId, integrationId, contractId}`。

契约由管理员随软件安装，声明 callable 方法、状态源、typed schema、授权提取规则及执行类别；不接受节点上传脚本、schema URL 或自行降低执行类别。提供端发布前必须确认所有已批准契约成员都有 handler，中心只能校验声明和批准；提供端谎报无法由注册证明消除，实际分派错误仍需显式返回和记录。

同一活动代次重复注册同一 Space 内同一 endpoint 返回 `conflict`，不覆盖。已获批准的新 staged 代次可以在 §4.2 的原子激活中替换同一 Node 的旧发布，但不能接管另一 Node 的实例。注册成功回复丢失时，不盲目重发；重连通过同样的 staged 激活规则重新注册。

### 6.2 撤销

`module/unregister` input：`{providerEndpointId, registrationId}`。只有当前拥有该发布的 Node/Link 可撤销。旧 registrationId 返回 `stale_revision`，不得删除新注册；对无权主体不泄漏注册是否存在，返回 `forbidden`。

撤销先禁止新路由，再结束订阅、请求取消在途调用并移除注册，成功返回 `{removed:true}`。已经发送的副作用不因此被当作未执行；调用终结仍按 §7。响应只确认注册已撤销，不确认所有外部操作被回滚。

### 6.3 发现与固定绑定

`module/list` input：`{spaceId}`；result：`{modules:[{providerEndpointId,registrationId,spaceId,integrationId,contractId,methods,states}]}`。

只有当前活动、且调用者有权看到的注册进入列表；methods/states 按调用者授权投影。注册上限保证列表有界，编码结果超过 maxFrameBytes 时返回 `payload_too_large`，不静默截断或伪造完整列表。S1 不提供分页或目录变更订阅。

调用者带发现得到的 registrationId 发起交互；它与当前注册不符时返回 `stale_revision`，不得自动切到新提供者。先做目标权限检查再披露未知或过期情况。节点重连由消费者显式重新发现，SDK 不静默改绑。

## 7. 调用、超时、取消与未知结果

### 7.1 调用

`module/invoke` input：`{method:string, argument:typed-value}`。中心查询 `(spaceId, endpoint, registrationId, contractId, method)` 的契约并验证输入和权限后转发。

中心为每次转发生成新的每跳 requestId，映射键包含源连接/HTTP binding、源 requestId、目标 Link 代次和 registrationId。提供端只用转发 ID 回复；中心还原调用者 ID。不同消费者使用相同 requestId 不会相互完成请求。

每个发起方不得在同一 scope 复用请求 ID；并发重复 ID 在转发前以 conflict 拒绝。ID 不提供执行去重，已完成请求 ID 的重发不享有幂等保证。SDK 不重发业务请求或在重连时重放 pending。

成功 result 是契约结果；中心和消费者均校验输出。对于 non_replayable，提供端成功执行后返回畸形结果属于 `outcome_unknown`，不能改报未执行或自动重试。

### 7.2 执行边界

```text
received → validated → queued → dispatched → succeeded | failed | cancelled | unknown
                  └─────────────→ rejected / timeout(not_started)
```

中心仅在确认业务请求尚未交给传输、没有可能被提供端接收时，才能报告 `execution=not_started`。socket send 成功不是执行成功；send 失败也不一定证明未发送。越过可发送边界而无法确认提供端结果时，non_replayable 必须保守地报告 unknown。

| 情况 | code | execution | retry |
|---|---|---|---|
| 输入/权限/限额在发送前拒绝 | 对应现有码 | not_started | never |
| 排队期间预算耗尽 | timeout | not_started | safe |
| read_only 发出后超时或连接丢失 | timeout 或 unavailable | unknown | safe |
| non_replayable 可能已发出，超时/连接丢失/输出不可信 | outcome_unknown | unknown | never |
| 提供端确认业务失败且结果已确定 | 契约现有码 | completed | never |
| 未发出时确认取消 | cancelled | not_started | never |
| 提供端确认已停止且执行结果已确定 | cancelled | completed | never |

SDK 默认不自动重试任何调用；safe 仅表示调用者可以在重新授权、重新发现并取得有效绑定后重试。non_replayable 结果未知时不得换 requestId 自动补发。

调用者失去中心连接时也必须按同样保守规则在本地产生错误；不能因为收不到中心 error 就把它当作 not_started。超时与断线必须 settle 所有 pending。中心崩溃不承诺恢复执行历史。

### 7.3 取消

`module/cancel` input：`{requestId}`，指原消费者在同一连接或 HTTP binding 中的 invoke 请求 ID。中心只查该主体/scope 的记录，不能取消他人的调用。没有活动记录返回 `{requested:false}`，不泄漏其他 scope 状态。

有活动记录时返回 `{requested:true}`，含义仅是接受停止请求，不是“已撤销业务”。尚未发送则直接终结原调用；已发送则向固定的提供端代次转发 cancel，并等待原 invoke 的确定结果，至原截止时间为止。

取消按原记录定位，不重新解析当前目录；目标注册已经撤销时仍可向原代次发送清理，但绝不能发送给新注册。主体失去新调用权限不阻止其停止自己的在途请求；它不因此重新获得读取业务结果之外资源的权限。

提供端把取消信号交给 handler。它不能确认结果时，不得伪造 cancelled；到期按 unknown 终结 non_replayable。已经确定的成功/失败优先于随后到达的取消，原调用最多有一个终结响应。cancel 使用自己的请求 ID，不占用原 invoke 的 ID，不延长原调用预算。

SDK 的取消 API 必须区分“取消请求已受理”和“原操作已确定停止”。AbortSignal 不得直接合成 cancelled/not_started；若 SDK 必须立即结束本地等待，已发送的 non_replayable 只能给出 outcome_unknown。

迟到业务回复不能重新 resolve 已终结调用。失联调用者触发中心尽力取消并释放路由记录，但不据此承诺外部副作用未发生。

## 8. 状态：快照、更新与结束

### 8.1 建立订阅

`state/subscribe` input：`{subscriptionId,state:string,argument:typed-value}`。订阅归属固定为发起 Link、目标注册、状态源和参数。并发重复 subscriptionId 返回 conflict；SDK 在同一 Link 不复用已用 ID。

请求预算只约束订阅建立，不给成功订阅设置 8 秒寿命。中心为转发分配自己的 subscriptionId，将它与消费者 Link/原 subscriptionId 一一映射；提供端只能用该转发 ID 发通知。消费者在发送请求前登记本地 opening 状态。

建立失败、超时、取消或消费者失联时，中心移除 opening 映射并尽力向原提供端代次发送 unsubscribe；迟到快照/更新不得重新激活订阅。提供端对 opening 状态的 unsubscribe 同样有效；提供端 Link 关闭时释放全部观察者。消费者可在快照返回前调用 unsubscribe：中心仍在 opening 时终结原 subscribe 为 cancelled/not_started；若成功响应已经入队则保留该响应，但客户端已停止回调且不再激活订阅。两种情况都释放上游观察者，不产生第二个 subscribe 响应。

成功 result：`{subscriptionId,registrationId,snapshot}`。snapshot 为：

- `revision: uint64-string`
- `value: typed-value`，完整状态
- `sampledAtMs: uint64-string`，源的墙钟采样时间，仅用于展示，不用于排序
- `quality: "current" | "stale"`，源声明的数据有效性，不等于连接在线

源状态为 stale 时仍可返回快照，但不能向消费者标为正常新鲜数据。业务采样周期和有效期属于领域契约；S1 不以收到一帧就保证机器指标的真实采样年龄，也不依赖跨设备时钟一致。

提供端必须在同一序列化边界中读取快照 revision 并挂接更新观察者。该边界之后的每个发布更新都进入此订阅的有界队列；不允许先读取快照、过一段时间才开始监听。

提供端先把成功响应排入有序发送队列，再发更新；中心同样先向消费者排入成功响应再发更新。两跳都必须为 opening 状态分配有界缓存；建立期间超限则失败并清理上游订阅，不能返回成功后静默丢掉窗口内更新。

### 8.2 更新

`state/update` input：`{subscriptionId,seq,sample}`，sample 与 snapshot 结构相同。

第一条更新 seq=`"1"`，以后必须逐个加一；revision 必须大于上次接受的 revision，但不要求加一。每次修改先分配 revision 再广播，不能对同一 revision 发布不同值。相同值重新采样也必须增加 revision。uint64 即将溢出时结束相关订阅并要求重新注册，禁止回绕。

消费者必须先观察到快照，再观察更新。中心验证提供端连接、注册、订阅映射、seq、revision 和 value 契约后转发；消费者再次检查顺序。没有本订阅的通知不得触发回调。

发现 seq 缺口、重复/倒退或 revision 不递增，视为协议失序：停止交付该订阅的新值，标记过期，并以 `session_lost` 结束该订阅。S1 不静默去重、修补或合并。若中心无法可信地归属一个通知，按不可信帧处理，不据此创建订阅。

revision 只保证一个已注册状态源内部的顺序。不同模块不能比较 revision；中心不按 sampledAtMs 给不同设备排出全局历史。

### 8.3 结束与取消订阅

`state/end` input：`{subscriptionId,error}`，error 是生成类型 Error 的 JSON 表示，字段为 `code/message/diagnosticId/execution/retry/details`；其中 code 使用既有枚举名称（如 `ERROR_CODE_SLOW_CONSUMER`）。这与 JSON-RPC Failure 的数值 code + data 映射不同，但共享同一 Error 类型和冻结错误编号，不另造字符串异常表。它没有 JSON-RPC id，也不要求回复。

`state/unsubscribe` input：`{subscriptionId}`，响应 `{removed:boolean}`。中心从同 Link 的活动表移除后停止交付、转发上游清理；未知或已结束的本地 ID 返回 false，不影响他人订阅。客户端从调用 unsubscribe 起停止用户回调，不等待网络清理才停止。

每个订阅对用户恰好有一次本地终结事件。提供端退出/断线、模块撤销、代次变化、授权撤销和队列溢出都会终结；消费者连接断开时 SDK 本地产生终结。整个连接丢失不要求再收到线上 end。

终止码固定为：提供端失联或主动撤销→unavailable，注册代次替换→stale_revision，授权撤销→forbidden，队列超限→slow_consumer，序列失序→session_lost。state/end 的 execution=completed 表示订阅已终结，不表示某项业务副作用完成；forbidden 的 retry=never，其余 retry=safe 只允许重新发现并建立新订阅。主动 unsubscribe 由响应和 SDK 本地终结确认，不要求额外发送 end。

S1 不恢复 subscriptionId、不重放旧更新。重新发现并新建订阅必须先得到新快照；UI 或其他消费者保留最后值时必须将其标记过期。自动重连通信连接可以存在，但不能静默恢复一个看似连续的订阅。

## 9. 队列与控制面有界

沿用 Limits 前四个字段，并新增 `max_subscriptions=5`、`max_modules=6`。S1 JSON 字段为 maxSubscriptions、maxModules，必须为正整数。各方协商取可支持的下限，不能用 0 表示无限。

受限开发配置的建议初值如下；是可降低的上限，不是所有部署都必须接受的固定性能承诺：

| 字段 | 默认上限 | 约束 |
|---|---:|---|
| maxFrameBytes | 262144 | 编码后单帧字节 |
| maxInflight | 32 | 每 Link 或 HTTP binding 同时在途请求 |
| maxQueuedBytes | 1048576 | 每 Link 尚未发送的编码帧总字节 |
| timeoutMs | 8000 | 每次请求最大预算 |
| maxSubscriptions | 64 | 每 Link 同时 opening/active 的订阅 |
| maxModules | 64 | 每 Node 活动注册数 |

中心还须受静态批准的节点和注册总数约束，HTTP binding 有既有过期时间。pending、opening 缓存和订阅数据必须记入限额，不能在 socket 队列外再建无界缓冲。编码超过帧上限的状态不能被任意切成不透明片段，返回 payload_too_large 或结束已建立订阅。

为保证满负载仍能取消，maxInflight 必须至少为 2，每个连接/HTTP binding 预留其中一个在途槽给 cancel/unsubscribe；普通请求不能占用该槽。控制处理不能等待被取消的业务 handler 返回才被调度。超过控制槽上限时明确拒绝或由调用方稍后重试，不建无限控制队列。

每条 Link 的 maxQueuedBytes 中预留 maxFrameBytes 作为有界控制空间，要求 maxQueuedBytes 至少为 maxFrameBytes 的两倍。数据不能消耗保留空间；控制空间仍共享总上限，不另开无界优先队列。

状态数据无法入队时，该订阅以 slow_consumer 结束并释放队列；不暂停其他节点或丢弃部分值假装连续。必要的 end/取消控制也无法入队时关闭该 Link，SDK 将剩余订阅统一终结。控制帧有优先级，但同一订阅的快照、update、end 必须保持因果顺序；丢弃未发送数据时只能终止，不能再发送后续 update。

请求排队无法在预算和限额内受理时返回 quota_exceeded 或 timeout，前提是该请求尚未可能发送给提供端。消息接收方也必须实施帧、对象深度及解析资源边界，不因认证成功信任任意大输入。

## 10. 贯穿验收的真实能力

S1 使用两个独立的进程内计数器模块，分别由 TS 与 Rust 实现同一预安装契约 `conex.counter`。它们是真正修改进程状态的样例能力，不是伪造 Linux/GitHub 数据，也不承诺重启持久化。

| 成员 | 输入 | 输出 / 状态 | 执行类别 |
|---|---|---|---|
| counter/read | `{}` | `{count:int32,revision:uint64-string}` | read_only |
| counter/add | `{delta:int32}` | `{count:int32,revision:uint64-string}` | non_replayable |
| counter/value | `{}` | 状态 sample.value=`{count:int32}` | 状态源 |

每个注册初始 count=0、revision=`"0"`。一次成功 add 原子修改 count、增加 revision 并入队发布新状态；即使 delta=0 也增加 revision。int32 相加越界时在修改前返回 bad_request，count 与 revision 均不变。

读取、修改和订阅快照共享同一序列化边界。成功 add 返回的 revision 对应已提交值，但不等待每个订阅者处理完成；慢订阅结束不能回滚 add。状态 sample.quality=current，sampledAtMs 是最近一次提交或初始采样时间。

演示程序可以按受控测试屏障在“提交之后、回复之前”暂停，以真实观察副作用未知；屏障不进入正式能力契约或发行 SDK API。正式验收不得用模拟 RPC 回包替代两个进程和真实路由。

## 11. 完整消息示例

以下是 JSON WSS 的有效数据示例；静态批准项已存在。示例中不同类别 ID 独立，不得将 registrationId 当凭据。

### 11.1 调用 Rust 计数器

```json
{
  "jsonrpc": "2.0",
  "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
  "method": "module/invoke",
  "params": {
    "context": {
      "providerEndpointId": "counter-rust",
      "plane": "broker",
      "spaceId": "dev",
      "registrationId": "01ARZ3NDEKTSV4RRFFQ69G5FAW"
    },
    "timeoutBudgetMs": 8000,
    "input": { "method": "counter/add", "argument": { "delta": 1 } }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
  "result": { "count": 1, "revision": "1" }
}
```

### 11.2 订阅快照与第一条更新

```json
{
  "jsonrpc": "2.0",
  "id": "01ARZ3NDEKTSV4RRFFQ69G5FAY",
  "method": "state/subscribe",
  "params": {
    "context": {
      "providerEndpointId": "counter-rust",
      "plane": "broker",
      "spaceId": "dev",
      "registrationId": "01ARZ3NDEKTSV4RRFFQ69G5FAW"
    },
    "timeoutBudgetMs": 8000,
    "input": {
      "subscriptionId": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
      "state": "counter/value",
      "argument": {}
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "id": "01ARZ3NDEKTSV4RRFFQ69G5FAY",
  "result": {
    "subscriptionId": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
    "registrationId": "01ARZ3NDEKTSV4RRFFQ69G5FAW",
    "snapshot": {
      "revision": "1",
      "value": { "count": 1 },
      "sampledAtMs": "1792195200000",
      "quality": "current"
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "state/update",
  "params": {
    "context": {
      "providerEndpointId": "counter-rust",
      "plane": "broker",
      "spaceId": "dev",
      "registrationId": "01ARZ3NDEKTSV4RRFFQ69G5FAW"
    },
    "input": {
      "subscriptionId": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
      "seq": "1",
      "sample": {
        "revision": "2",
        "value": { "count": 2 },
        "sampledAtMs": "1792195201000",
        "quality": "current"
      }
    }
  }
}
```

该更新以快照后另一次 delta=1 的成功提交为前提；它不是第一条 invoke 被重复执行。

### 11.3 提交后失联

```json
{
  "jsonrpc": "2.0",
  "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
  "error": {
    "code": -32012,
    "message": "provider outcome is unknown",
    "data": {
      "code": "outcome_unknown",
      "diagnosticId": "01ARZ3NDEKTSV4RRFFQ69G5FAZ",
      "execution": "unknown",
      "retry": "never"
    }
  }
}
```

这是 §11.1 成功响应的互斥故障分支，不是同一调用的第二个响应。SDK 不自动补发 add。

## 12. 验收场景

以下均为实施后的通过条件，不是本轮测试结果。应保留最小、确定性的消费行为回归；竞态用屏障控制，不以 sleep 猜顺序。

| 编号 | 场景 | 必须观察到的结果 |
|---|---|---|
| A01 | TS 消费 Rust，Rust 消费 TS | 各自 read=0，add(3) 后 read=3；两个模块状态不混用；无需浏览器 |
| A02 | 读取快照边界并发 add | 每个边界前提交包含在快照，边界后提交依序交付；首个 seq=1，无静默窗口 |
| A03 | 同一 endpoint 的旧注册和迟到 cleanup | 新 registrationId 生效；旧调用 stale_revision；旧 cleanup 不删除新注册 |
| A04 | 不同连接使用相同请求 ID、伪造 principal 或目标 | 不串回包；伪造身份拒绝；越权请求不转发任何业务帧，不执行 handler |
| A05 | 排队时取消和排队超时 | 原 invoke not_started，计数器不变；取消受理与原调用结果分别关联 |
| A06 | handler 可确认停止；取消与已提交成功竞争 | 可确认停止才 cancelled；已确定成功不被迟到 cancel 覆盖；仅一次终结 |
| A07 | add 已提交，屏障阻止回复，切断连接 | 调用方 outcome_unknown/retry=never；同进程直接观察 count 只增加一次；SDK 不重放 |
| A08 | 提供端断线、撤销或换代 | 订阅终结且最后值过期；重连新订阅先快照，不接收旧代次更新 |
| A09 | 限额较低的慢订阅 | 总队列不超过协商字节上限；slow_consumer 或连接终结；其他独立节点仍可读写 |
| A10 | update seq 缺口/重复、revision 回退、迟到或伪造订阅 ID | 无错误新值回调；受影响订阅结束或不可信连接关闭；不影响他人订阅 |
| A11 | HTTP 与字段边界 | list/invoke/cancel 使用真实 binding；subscribe/register 明确不支持；未知/重复字段、缺失限额、uint64 越界拒绝 |
| A12 | 领域数值边界及输出校验 | int32 溢出不修改状态；TS/Rust 保留大于 JS 安全整数的 revision；副作用成功后的畸形结果不能标为未执行 |

A07 的提交屏障观测来自本地执行节点，不需要未知的 SaaS 行为。A09 验证应用层记账上限和超限处理，不把进程总 RSS 等同于协议队列字节。

## 13. 当前实现证据与迁移边界

已有全局背景见 [迁移盘点](../design/2026-10-17-conex-migration-assessment.md)。本契约仍需新增通用 provider API、注册代次路由和状态订阅，而不是把已有名称改成 S1。

| 当前范围 | 静态可见事实 | S1 处置 |
|---|---|---|
| [control.proto](../../schema/conex/control.proto#L7-L60) | hello/ready、提供/依赖能力、LinkIdentity 已定义 | 复用握手，新增 S1 限额协商，不声称旧连接已支持 S1 |
| [rpc.proto](../../schema/conex/rpc.proto#L38-L56) | 有剩余预算及可信转发 principal/tenant，tag4/5 在 CallParams 被预留 | 给 RequestContext 加 Space/注册字段，不复用预留 tag |
| [common.proto](../../schema/conex/common.proto#L16-L57) | 已有 cancelled/outcome_unknown/slow_consumer、四个 Limits 字段 | 保留错误编号，扩展 Limits 与严格生成映射 |
| [SDK 握手](../../sdk/typescript/src/ws-client.ts#L410-L417) 与 [通知 API](../../sdk/typescript/src/ws-client.ts#L585-L625) | 空 provides、通用通知 listener 和具体业务调用机制 | 新增 handler/发布/取消/状态 API；通知 listener 不能代替订阅状态机 |
| [SDK 重连](../../sdk/typescript/src/ws-client.ts#L653-L718) | 现有实现把 pending 移入队列并在重连后发送，假定当前调用主要为读取 | S1 必须移除该重放策略在通用调用上的适用性；不能原样复用，否则 add 等副作用可能执行两次。通信重连与业务重试必须分开 |
| [agent 注册结构](../../schema/conex/agent.proto#L9-L28) | Agent 专有 endpoint/root/methods 注册 | 不把它当模块契约；将可复用的认证、代次、pending 机制接入 S1 通用路由 |
| [注册激活](../../crates/conex-host/src/ws_transport.rs#L1330-L1347) 与 [连接替换](../../crates/conex-host/src/remote.rs#L248-L283) | 先验证注册再激活 staged，替换时关闭旧连接并结束其 pending | 保留“坏注册不踢掉健康旧连接”和代次隔离；增加逐模块 registrationId，不能只按 Node/连接标识路由 |
| [远端超时](../../crates/conex-host/src/remote.rs#L136-L147) 与 [agent 限制](../../crates/conex-host/src/ws_transport.rs#L1379-L1383) | 超时仅移除本地 pending；agent 链路当前只接受注册与心跳控制 | 新增真实线缆取消、双向业务分派及执行类别错误映射；不能把本地删除 pending 当作提供端已停止 |
| [operation.proto](../../schema/conex/operation.proto#L14-L35) 与 [stream.proto](../../schema/conex/stream.proto#L22-L41) | 已有执行类别、操作状态与流 ACK 结构 | 不混淆业务完成、字节送达和状态快照；S1 不宣称复用即获得持久去重或状态回放 |

实施范围涉及 schema 与生成管线、conex-proto/core/host、Rust SDK 边界、TS SDK 和跨语言验收程序。旧文件共享及二进制内容路径不在本契约中替换；已公开的旧方法不因文档出现而删除。

每个实际修改符号先做 impact；切换调用方后删除旧默认耦合，不增加兼容别名、平行版本路由或把 hello 当回退。S1 实施计划必须给出确切文件、验收命令与受影响调用方，本文不代替该计划。

对应 [S1 实施计划](../plans/2026-10-17-conex-s1.md) 已录入 Paca（S1 CNX-3，T01–T09 为子任务）；当前只有计划，没有 schema 或 SDK 实现交付。

## 14. 评审与来源

### 14.1 需要评审的取舍

本专项采用：JSON WSS 共同提供端路径、静态批准而非完整 Space 管理、全量状态与显式中断而非 patch/回放、两种执行类别而非持久去重。这些已在正文给出具体默认行为，没有未定义的线协议分支；本文件需用户评审后才能进入 schema 和代码实施。

HTTP 流式订阅、protobuf S1 业务映射、事件回放、动态插件契约打包、真实 Linux/GitHub 接入和 UI 可以分别设计，不阻塞 S1 的上述闭环。S4 仍是公开多用户使用的前置安全条件。

### 14.2 外部来源

于 2026-10-17 实读 [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification)：采用请求/响应关联、通知无响应和双向角色可并存。ULID-only、无 batch、严格字段、认证、订阅、取消和执行类别是 conex 自身约束，不是 JSON-RPC 自动提供的保证。

SICP 与 xMatrix 的产品参考取舍沿用目标架构；本契约不依赖其实现或聊天命令语法。
