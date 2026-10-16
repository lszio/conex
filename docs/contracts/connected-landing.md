# Connected landing contract

状态：**L01 契约冻结 + L10 连接面板契约冻结（2026-09-21）+ M0 内容契约冻结（2026-10-02，§8）+ L11 客户端 hello 契约冻结（2026-10-04，§9）**。L10 在 `schema/conex/dashboard.proto` 新增 `ConnectionListRequest` / `ConnectionListResponse` / `UiLinkSummary` / `AgentLinkSummary`；L11 新增 `ClientProfile` / `ClientSummary` / `ClientList*` / `ClientHello*`。Rust/TypeScript/JSON Schema 由 `cargo xtask generate` 生成。L02–L09 边界不变。

## 7. 连接面板契约（L10）

落地页在 ready 之后持续调用 `connection/list`，host 返回当前主体的所有浏览器 UI 链接 + 所有已注册 Agent 链接。Broker 在 `lib: /rpc` 上严格只接受 `role=ui`；`service` / `agent` 调用直接返回 `Forbidden`，不进入 registry。

`UiLinkSummary` 每条包含：`linkId`（8 字节随机 base64url，登录时分配）、`principalId` / `tenantId`（来自会话）、`connectedAtMs`（web session 创建）、`lastSeenAtMs`（每帧 ready / 业务帧自上次心跳后的最新时间）、`ticketsIssued`（ticket 被消费的累计）、`callsTotal`（业务调用累计）、`callsInFlight`（进行中调用，未排空时大于 0）。

`AgentLinkSummary` 来自既有 `AgentRegistry`：agentId、principalId、tenantId、registeredAtMs、lastHeartbeatAtMs、providerIds、resources、methods。

要点：
- WSS upgrade 收到 ticket 后 host 立即 `increment_tickets`；连接进入 ready 后 `touch` 更新 `lastSeenAtMs`。
- 业务帧 dispatch 路径在 `dispatch_business` 包裹 `begin_call` / `end_call`，保证 `callsInFlight` 排空为 0、`callsTotal` 严格递增。
- WS 关闭后链接保留在 registry 中，`lastSeenAtMs` 不再前进；下一次 `connection/list` 仍返回同一行。
- 列表对当前主体隔离：`list_for_principal(Some(caller))`；service / agent 角色被 Broker 直接拒绝。
- 票据、token、cookie 不出现在面板任何字段；面板只投影已存在的会话元数据 + 计数器。

未在本轮覆盖：跨主机面板投影、agent 跨进程的代次差分面板；详见 `verification/connected-landing.md` §未验证项。

## 1. 拓扑与信任边界

浏览器只连接中心 Host（HTTPS/WSS）。私有侧 Agent 是 WebSocket 发起方，主动反连中心 Host。浏览器从不连接 Agent，不接触 Agent 的文件路径、上游地址或凭据。Host 是唯一的认证、授权、目录投影和远端路由边界。

Host 配置预授权的 `agentId`、`tenantId`、端点、只读方法和资源根；Agent 注册只能激活既有配置，不能创建端点、换租户、扩大方法集合或替换另一个 Agent。Agent 的本地配置再把每个端点映射到一个受限根目录，并逐项声明 `source/list`、`source/read`、`source/search` 的子集。两端都必须校验，Host 的授权结果优先于 Agent 自报内容。

`embedded` 和 `outbound_http` 是静态安装信息，连接状态为 `not_applicable`；`reverse_agent` 只有认证、能力协商和注册完成的当前 socket 才是 `ready`。断开立即变为 `offline`，不把静态配置或最近心跳伪装成在线。

## 2. HTTP 登录与会话

受控部署的默认 UI 入口使用管理员签发的专用静态 UI 凭据：

1. 浏览器同源 `POST /web/login`，在 `Authorization: Bearer <ui-token>` 提交凭据。
2. Host 校验静态 token 的 hash、`role=ui`、主体、租户和 audience，创建随机 Web 会话。
3. 响应只设置 `HttpOnly; SameSite=Strict` 会话 cookie；HTTPS 使用 `Secure`。响应不回显长期 token。
4. `GET /web/session` 返回当前主体的显示信息和一次 CSRF nonce，不返回 token。
5. `POST /web/logout` 撤销当前会话、它签发但未消费的 ticket 和所属 UI link；不影响同主体的其他会话。

状态修改和 `POST /tickets` 必须验证精确配置的 `web_origin` 与 CSRF nonce；不开放任意 CORS。登录失败、角色错误、过期会话和超额会话返回明确的认证错误，不降级为匿名访问。token 不进入 URL、localStorage、页面日志或审计内容。
每个预授权 Agent 必须同时配置非空 `credential_name`、非空 `credential_backend`（`env:NAME` 或 `file:PATH`），并存在同 `principal_id=agentId`、同 `tenantId`、`role=agent` 的 Host token；缺任一项拒绝加载配置。策略引用不存在的 endpoint ID 也拒绝加载。

完整 OIDC authorization-code + PKCE 仍是可替换的登录适配器，不是本轮默认可用路径。OIDC 身份必须校验 issuer、subject、audience、nonce、时效和签名；OIDC token 不能被当作 Agent 注册凭据，UI 静态 token 也不能注册 Agent。

## 3. Ticket 与 WSS

已认证 Web 会话调用 `POST /tickets`，Host 签发绑定主体、租户、目标 Host、角色、允许方法和会话的随机一次性 ticket。ticket 有效期 30 秒，原子消费一次；过期、重复消费、Origin 不匹配、会话注销或绑定目标不匹配均拒绝。ticket 可以出现在 `/wss?ticket=...`，但代理、访问日志和诊断必须脱敏整个 query。

WSS 引导顺序固定为：

```text
conex/hello → hello result → conex/ready → ready result → business frames
```

`hello` 的服务端能力来自实际角色和可达注册，不回显客户端的 `provides`。服务端生成 link/negotiation identity；双方核对 profile、plane、能力和 limits。收到 `ready result` 前禁止业务调用；硬性 `requires` 不满足则 link 不进入 ready。HTTP binding 与 WS link 身份分开，不能用 `bindingId` 冒充登录 cookie 或 Link ID。
L01 deliberately freezes `LinkIdentity` and `NegotiatedCapabilities` in `control.proto` without changing the current runtime handshake structs. L02 is the integration owner: it must consume these generated types when migrating `conex/hello`/`conex/ready`, preserving the existing JSON and protobuf profiles and removing any duplicate wire source. Until that migration, these messages are contract vectors, not an advertised runtime capability.

## 4. 端点目录与只读范围

`endpoint/list` 使用 `EndpointListRequest` / `EndpointListResult`。返回内容仅是当前 Web 主体可见的 `EndpointSummary`：端点 ID、展示名、provider ID、管理员地域标签、接入方式、连接状态、可用方法和授权范围。不得返回绝对路径、上游地址、credential 引用、token 或其他秘密。

默认 limit 为 50，最大 100；按 `endpointId` 稳定排序，使用 `afterEndpointId` 和 `nextAfterEndpointId` 分页。`AuthorizedScope` 的 `root` 和 `subtree` 表示实际可见范围；局部授权不能提升为根目录授权。目录可见不等于业务调用必然成功，`source/*` 调用仍逐次通过 Host 的 MethodContract、租户和资源策略。

页面本轮只允许 `source/list`、`source/read`、`source/search`。不提供 shell、写文件、blob、Agent 注册、operation 或任意命令入口。页面不自动重放失败调用；Agent 离线、超时和权限拒绝保持不同错误语义。

## 5. Agent 注册与反连

Agent 先验证 Host 的 TLS CA 和 server name，再使用 `role=agent` 的 bearer 凭据升级 WSS；默认禁止明文，只有 `allow_loopback_ws=true` 且 authority 精确为 `localhost`、`127.0.0.1` 或 `[::1]` 时允许 `ws://`。Agent 配置中的 `token_name` 是逻辑凭据名，`token_backend` 必须是非空的 `env:NAME` 或 `file:PATH` 来源；二者都不把秘密写入配置报文。凭据只在 Agent 进程内解析，不经浏览器或业务报文传递。

通过已认证 Link 发送 `AgentRegisterRequest`，其中包含 agent ID、租户、端点映射和声明方法；请求不包含 secret。Host 将其与静态 AgentConfig 和 EndpointConfig 逐项比较，成功返回 `AgentRegisterResult` 的 agent ID、当前 link ID、接受的端点 ID 和被拒绝能力；响应绝不包含任何凭据或本地路径以外的秘密。

每次有效注册分配新的连接代次；新连接替换旧连接并关闭旧 Link。旧 reader 的 cleanup 必须比较代次，不能把新连接误标离线。Host→Agent 业务请求只携带已校验的 source 输入、目标端点和剩余 deadline，不转发浏览器 bearer、Host 凭据或页面自报主体。Agent 只接受当前已认证 Host Link 上、已配置端点和已声明只读方法的请求。

## 6. 审计与恢复边界

审计可以记录主体、租户、Agent/端点、方法、范围、Link/请求 ID、阶段、结果、错误码和延迟，但不得记录 token、ticket、参数原文、凭据或内容密钥。Agent 断线时未完成的只读调用明确失败；重连须重新认证、握手和注册，不恢复旧 ticket、旧代次或旧调用。跨进程会话恢复不属于本契约。

## 8. 内容契约与范围语义（M0）

本节冻结多主机数据服务的统一资源语义；实现按里程碑（M1–M5）逐项贯通，类型先行。

**资源定位与租户。** 资源由 `endpointId + resourceId` 唯一确定；租户始终来自认证上下文，不信任请求自报。`provider_id` 是端点的配置身份，不得代替唯一端点定位。`BlobAccess.endpointId` 为空表示 Host 本地已提交内容。

**读取结果二选一。** `source/read` 返回 `text ⊕ content`：小型受支持 UTF-8 文本内联 `text`（可附已计算 `cid`）；附件与大文本返回 `content`（`BlobRef`：可选 `cid`、十进制字符串 `sizeBytes`、`mime`、`access`、可选 `revision`）。`cid` 仅当字节已按既有 CID 格式计算；远端可变文件未计算 CID 时缺省，禁止用路径或 revision 假充 CID，也不得为读取而先整文件取回再计算 hash。

**blob/get 互斥目标。** 请求目标是 `committed`（已授权根下的已提交块，整块返回）或 `remote`（端点 + 资源 + 可选 revision + 字节范围）二者之一，互斥结构由 MethodContract 强制；同一 offset 不允许既解释为块内偏移又解释为文件偏移。响应中 `chunkCid` 与 `revision` 同样互斥：committed 返回块 CID；remote 返回实际服务的 revision 与 `eof`，任意 range 不冒充完整块 CID。

**范围语义。** offset/length 为字节、十进制字符串（64 位安全，JS 不丢精度）；`length >= 1`，`offset+length` 溢出拒绝，非法与超配额范围拒绝（`bad_request` / `payload_too_large` / `quota_exceeded`）。remote 读取结束返回实际长度与 `eof`；请求携带 revision 且与当前不符时返回 `stale_revision`（-32013），不拼接两个版本。无法保证一致快照的 provider 必须拒绝该保证。

**元数据。** `ResourceSummary` 统一包含：`title`（文件名 basename，不是路径）、`mime`、`sizeBytes`（十进制字符串）、`revision`（不透明令牌，仅等值比较；无法提供时缺省）、`kind`（目录项类型）。未知类型 MIME 固定 `application/octet-stream`；MIME 不等于允许执行内容，活动内容不得在 Host 同源内联执行。

**授权覆盖面。** 目录、读取、范围读取、have、上传恢复、提交、pin/unpin、cancel 全部走同一资源授权；CID、upload ID、pin ID 都不是授权凭据，不能凭知道标识绕过租户或资源策略。物理内容按 CID 去重不合并逻辑授权。本节的强制执行在 M1（blob 所有权与资源授权）落地。

**wire 调整。** `BlobGetRequest` 旧 `chunkCid/rangeOffset/rangeLength` 字段号 reserved，由 `committed/remote` oneof 取代；`ResourceSummary.sizeBytes`、`BlobRef.cid`、`BlobGetResponse` 同步调整并统一迁移 Host、Agent、SDK、页面与向量，不保留双轨契约。旧字段（`providerId`）从 wire 结构中删除。


## 9. 客户端 hello 契约（L11）

落地页收敛为「hello」：连上即列出在线客户端，逐个发送 hello，每个访客自述名称／组／可见性。契约类型在 `schema/conex/dashboard.proto`。

**客户端的定义。** 一个客户端就是一条浏览器 UI 链接，加上它自述的 `ClientProfile`。访客仍共享 `web_guest.principal_id`；会话级显示名不是身份，不参与授权，也不唯一。

**一个标签页就是一个客户端。** 链接在 **`POST /tickets` 时**铸造，不在签发 session 时：
cookie 说明「谁在浏览」，链接说明「哪个标签页在说话」。同一浏览器开两个标签页共享一个
cookie，因此必须得到两条链接、两行 `client/list`、两个独立客户端——否则一个 cookie 会
把两个窗口挤成同一行，两个标签页会互相顶掉对方的名字与文件归属。三条规则成立：

- 每次取 ticket 都铸造新链接，响应回 `linkId`；重连的标签页拿到新链接，因为旧 socket
  的清理路径已经移除了它那一行，复用 id 会让半开的链接替一个已经不存在的 socket 应答
- 会话记录自己拥有的链接集合（上限 32 个标签页），`POST /tickets` 登记、WSS 升级校验、
  session revoke 回收都走这一个集合；**请求声称的 linkId 一律与集合比对**，标签页不能
  冒用同一会话里另一个标签页的身份
- 同源 HTTP 路由（`/web/files*`）在 header `x-conex-link-id` 上收调用方的链接；下载因为
  是普通 `<a href>` 收不了 header，允许 `?linkId=`，同样比对会话集合——这是传输方式，
  不是第二条入口

**资料是自述的。** `displayName` ≤ 40 字符、`group` ≤ 24 字符，入库前去除控制字符并截断；`visible` 默认 `true`。资料不授予任何能力，Host 不得因名称或分组改变授权结果。列表中的 `displayName` 为空时由 linkId 前 6 位回退（`client-xxxxxx`），不伪造身份。

**显示名全局唯一。** 名称是读者区分客户端的唯一依据，因此同名必须可区分：Host 在
`client/profile` 落库前，用**其他**在线客户端的显示名集合把 `displayName` 去重，冲突时
依次追加 `-2`、`-3`……（上限 999，超出则保留原名并让列表显示冲突）。去重在注册表锁内
完成，两个客户端并发抢同一个名字不会都拿到它。调用方**排除自身**：重连时页面会重发同一份
资料，若不排除自己，`alice` → `alice-2` → `alice-3` 会无限增长。名字为空时回退到
`client-<linkId 前 6 位>`，天然唯一，不参与去重。

**可见性是隐私开关。** `visible = false` 的客户端不进入任何 `client/list` 结果，且 `client/hello` 对其返回 `unavailable`——隐藏即不可寻址，不是仅隐藏标签。

**列表只含可寻址的链接。** `client/list` 只返回已挂上出站通道（handshake 完成）的链接。握手前崩溃或未 clean close 的死链会留下无 writer 的条目，必须既不出现在列表里，也按 60s 宽限后回收，避免注册表无界增长。

**hello 往返。** `client/hello` 携带 `targetLinkId` 与 ≤ 200 字符文本；Host 经目标链接既有的出站通道推送 `conex/client-hello`（JSON-RPC notification），目标回 `conex/client-pong`。Host 测量从入队到收到 pong 的往返毫秒数并返回 `roundTripMs`。目标离线、隐藏或 10s 未答均返回 `unavailable`，不静默丢弃。pong 以 notification 形式抵达，Host 必须在 broker 帧路径之前处理（无 id 的 notification 会被该路径丢弃）。

**hello 可以是回调。** `ClientHelloRequest.payload` 是发送方想被回答的字段，`ClientHelloResult.answer`
是目标原样回传的回答。区分二者的是推送帧上的 **`ask`**：

- `payload` 为空或缺省 → `ask: false`，**这是问候**。目标的 SDK 在派发监听器之前就自动回
  pong，于是测到的往返是 socket 的往返，不是页面的往返——这正是延迟样本要的东西
- `payload` 非空 → `ask: true`，**这是提问**。目标的 SDK **不**自动回 pong：自动确认会让
  发送方拿到一个没人测过的往返数字。回答只能来自页面，回调形式是
  `client.answerHello(replyId, answer, reply?)`；没人回答就是发送方超时

两侧的边界都在 Host 上：payload 按序列化后 ≤ 1 KiB 拒绝，**退给发送方自己**（否则目标只会
看到一次超时，没人知道是谁的参数太大）；answer 按 64 节点 / 4 层封顶，超限整个丢弃而不是
替对方走完一棵深树。`answer` 缺失时返回空对象 `{}`，形状恒定，客户端不必区分「没有回答」
与「这个 Host 不支持回答」。

**收方看到的是名字，不是 linkId。** 推送帧除 `from`（发送方 linkId）外还带
`fromName`：由 Host 用注册表把 linkId 解析为发送方当前的显示名。页面不把自己的名字拼进
`text`，否则未设名的客户端会以 `client-xxxxxx` 的形式向所有人问好。

**方法白名单。** `client/list`、`client/profile`、`client/hello` 加入 UI 角色白名单（`allowed_ui_method`）与 UI ticket 的 capability caps。页面不再提供 shell、写文件、blob 提交、Agent 注册或任意命令入口。

**推送与字节预算。** Host 主动下发的帧必须与 WSS writer 使用同一出站字节计数器；绕过计数会让 writer 的 `fetch_sub` 下溢 `usize`，其后所有预算检查失败并关闭该 socket。

## 10. 分组隔离、状态与文件共享契约（L12）

落地页从「hello 单场景」扩展为项目介绍 + 两个场景 + 状态。契约类型在
`schema/conex/dashboard.proto`（`ClientSummary.groupKey`、`ClientSummary.lastRoundTripMs`、
`ClientStatusRequest`/`GroupStatus`/`ClientStatusResponse`）。

**分组是隔离键，不是排序桶。** 访客填写的 `group` 是标签；Host 用 SHA-256 前 8 字节
（16 位十六进制小写）派生 `groupKey`，隔离一律按 key 比较：

- `client/list` 只返回**同组**客户端；`client/profile` 每次写入都重算 key，改名组后
  客户端立即离开原分组，不留在旧 key 下
- `client/hello` 跨组返回 `Forbidden`（-32003），且该检查在 `ClientRegistry::send_hello`
  内部，不能由调用方绕过
- 「未分组」是 key `e3b0c44298fc1c14`（空串的哈希），是一个真实分组而不是与所有组匹配的空值

**组名是加入凭据，只对本人可见。** 知道组名是进入同一分组的唯一条件，因此 Host 永远
不把 `profile.group` 文本投影给任何他人：`client/list` 的每一行 `profile.group` 一律
为空字符串（含调用者自己的行），`client/status` 的 `GroupStatus.label` 恒为空（字段保留
仅为线上兼容）。组文本只在 `client/profile` 的 `self` 响应里回显给所有者本人。页面把
组名默认为 `default`，提供随机组名生成与「复制/分享组名」入口——主动把组名发给谁，
谁才能进组；页面其余位置（在线列表、状态行）从不渲染组名文本，状态行只显示 key 前 8 位。

**延迟只有在真测过时存在。** `record_round_trip` 在一次 hello 完成时把实测毫秒记到
**双方**（往返是这对客户端之间的属性），并累计总和与次数。`lastRoundTripMs` /
`avgRoundTripMs` / `roundTrips` 在没有任何完成的 hello 时为 0，页面渲染为「尚未测量」，
不显示 0 ms——把缺测量渲染成 0 会让状态页谎报连接速度。亚毫秒往返截断为 0，页面显示
`<1 ms` 而不是四舍五入成一个没人等过的毫秒数。

**`client/status` 只投影计数与延迟。** `clientsOnline` / `groupsOnline` 是全 Host 计数
（回答「这台 Host 上有多少客户端与分组」这个进程级事实），每组一行 `GroupStatus`：
key、在线数、最近与平均延迟、往返次数、共享文件数与字节。**不投影任何个人行**：不含
linkId、名称或文件明细。`label` 恒为空字符串——组名是加入凭据（见上），统计行只认 key。

**文件共享是同组临时内容。** 四个同源路由共用 web 会话 cookie：

| 路由 | 方法 | 授权 | 用途 |
|---|---|---|---|
| `/web/files` | GET | cookie | 列出本组文件（只含元数据） |
| `/web/files` | POST | cookie + Origin + CSRF | 共享一个文件（原始字节） |
| `/web/files/download` | GET | cookie | 读取字节，可按 Range 取一段 |
| `/web/files/remove` | POST | cookie + Origin + CSRF | 撤回自己提供的文件 |
| `/web/files/uploads` | POST | cookie + Origin + CSRF | 声明并开始一个分块上传（或重新加入） |
| `/web/files/uploads/{id}/chunks/{index}` | PUT | cookie + Origin + CSRF | 发送第 index 块 |
| `/web/files/uploads/{id}` | HEAD | cookie | 该上传已收到多少块 |
| `/web/files/uploads/{id}/commit` | POST | cookie + Origin + CSRF | 完成上传并加入本组 |
| `/web/files/uploads/{id}/cancel` | POST | cookie + Origin + CSRF | 放弃上传 |

读方法与 `/web/session`、`/content` 同规则（GET 是安全方法）；写方法是状态变更，
必须校验精确匹配的 `web_origin` 与 CSRF nonce，与 `POST /web/logout` 一致。

**分块上传按内容身份续传。** 客户端在发第一个字节之前先声明每个分块的 CID（浏览器用
`crypto.subtle` 算 sha2-256，见 `web/src/lib/cid.ts`），主机据此用
`conex_proto::cid::content_cid_for_parts` 派生 root CID，再开 staging。因此：

- staging 绑定的是**内容**（owner + size + leaf CID 列表），不是一次传输尝试。同一份内容
  从同一 owner 再次声明会**重新找到同一个 staging** 并返回已收块数——「网络中断」和
  「标签页刷新」因此是同一个用例、同一段代码，客户端不需要保存任何上传状态。
- 分块**只按顺序收**（`index` 必须等于已收计数）。单写方场景下顺序足够，而它把「从哪继续」
  塌缩成一个整数；并行只会换来一个没人愿意 reconcile 的缺失集合。
- 每个分块在落盘前对照**声明里那一个 index 的 CID** 校验，坏块立即拒绝、不写 receipt。
  校验用的 CID 来自 receipt，不是请求头，否则客户端可在线谎报一个宽松的 CID 绕过它。
- 上传前按**声明大小**拒超，而不是传完再拒。
- uploadId 不是访问令牌：resume/chunk/commit/cancel 都由 `Owner{principal,tenant}` 把关，
  另一个 principal 四项操作全部 `Forbidden`。
- 已 commit / 已 cancel / 过期的 uploadId 与「从未存在」答同一个 404——可区分的答案就是探针。
- 分块大小固定 `CHUNK_SIZE`（256 KiB），不协商。

**元数据在内存，字节在 content 存储。** `ShareStore` 只保留一行元数据（id、name、
mime、size、`rootCid`、owner、groupKey、过期时刻），字节下沉到 `conex-content` 块存储，
按 CID 寻址——与 P1 blob 通道同一套寻址、同一个 `content_cid` 函数。因此列表面响
应永远不含字节，而一个 700 KiB 的预览只需读 256 KiB。`share_root` 独立于
`content_root`（未配置时取 `content_root` 的兄弟目录，且禁止两者相同）：blob 通道的
`collect_garbage` 会回收「不被任何 committed root 可达」的块，而在途上传正好长这样，
共用一个 root 会让两个场景互相删掉对方的分块。share root **不做 GC**，只按 hold 与
租约回收。

**Item 的 ETag 是 root CID；列表的 ETag 是组内 id 的有序哈希。** 一个 item 的
字节在 CID 上是不可变的，所以 `ETag: "bafkrei…"` 就是「这是这份内容」的陈述。一个
列表的 ETag 由 `Sha256(group_key ‖ U+001F 分割的 id 列表 ‖ expiries ‖ sizes)` 派生：
当任何一项被加、删、过期或改了大小，digest 就会变，因此 `If-None-Match` 让常态
轮询降到一次 304。空组没有可校验的内容——返回无 ETag 完整 body，避免把「组为空」
缓存成可观察的差分。

**id 不是访问凭据。** `ShareStore::get` 以调用方 groupKey 过滤，跨组、不存在、已过期
都返回 `None`，下载路由因此对三者答同一个 404——可区分的 403 会确认该 id 存在，过期
与否也不是可观测事实。同理 `rootCid` 与 uploadId 都不是凭据：字节读取由 groupKey
授权，staged upload 由 `Owner{principal,tenant}` 绑定。

**同一份内容只有一个地址。** 两个人共享同一份字节得到同一个 `rootCid`，是**两个
item 指向一个 root**。回收按 hold 计数：`hold_root` 在 commit 时加一，撤回或断线时
`release_root` 减一，只有最后一个松手才真正删块。盲删会让另一个 item 读到空。

**重启回收一切。** 场景是可丢弃的：重启意味着所有连接都已断开，上一轮遗留的
committed root 在启动时全部 `reclaim_all_committed`；staging 不在此列——中断的上
传仍可续传，按自己的租约到期清理。

**配额在存入前检查。** 单文件、单组累计、单客户端文件数均可由 `[share]` 配置
（默认 8 MiB / 32 MiB / 32），组上限低于单文件上限时自动跟随单文件上限——否则
「只想放大单文件」会得到一条不说原因的拒绝。组的当前用量是新文件的计量基准，
因此两个客户端竞争最后一段配额时不会都成功。

**提供者断开即回收。** WSS 循环在 `detach_writer` 之后调用 `ShareStore::remove_owner`，
再按返回的每个 item `release_root`：断开的客户端不再能提供文件，字节与组配额同时归还。
`remove` 先校验归属再删除——先删后校验会让任何组内成员销毁别人的上传。

**文件名可以是相对目录路径。** 目录上传把 `webkitRelativePath`（`dir/sub/file.png`）放
进 `x-conex-file-name`；`ShareStore::sanitize_name` 按 `/` 分段清理——控制字符、`\\`、
引号替换为 `_`，空段与 `.` 丢弃，`..` 展平为 `_`，总长截断 240 字符——名字仍是**显示
字符串**，从不作为文件系统路径（字节按 CID 存在 content 存储）。`Content-Disposition`
用最后一段 basename。页面按路径把每个提供者的文件渲染成目录树，列表视图用原生
`<details>` 折叠，网格视图用目录瓦片下钻 + 面包屑；两种视图共享同一份树数据。

**变更不是轮询出来的。** 写场景的请求触发一份 `conex/share-changed` 推送：
到同组**除自己之外**的所有在线 tab。推送只到 actor 之外的链接，因为 actor 自己知道它写了什么。
收到推送的页面立刻重新拉文件清单——不是等下一次轮询。同组列表（peer/status）
**没有**推送路径：它不是被告知的，是被问的，所以那个轮询保持 3 秒。一个 30 s 的轮询
可以服务被推送替代的「场景」，但不能同时服务「谁在线」。

**预览按 Range 取头部。** 列表面响应带 `previewBytes`（= `min(size, 256 KiB)`，不可预览
时为 0），页面据此构造 `?range=bytes=0-N` 发起预览请求。`/web/files/download` 同时接受
`Range` 头与 `range` query（同一套解析）。

Range 的三种回答必须分开，M1 曾把后两种混为一谈：

| 请求 | 回答 | 依据 |
|---|---|---|
| 可满足的单 Range / 开放 / 后缀 Range | `206` + `Content-Range: bytes s-e/total` + `Content-Length` | RFC 7233 §4.1 |
| 语法合法但无字节可满足（`bytes=999999-`、`bytes=-0`） | `416` + `Content-Range: bytes */total` + `Accept-Ranges` | RFC 7233 §4.4 |
| 多 Range、畸形、`If-Range` 不匹配 | `200` 整个 item | RFC 7233 §4.1／§3.2 |

「畸形答整个 item」不是为了宽容，是为了不泄露：一个上传失败和一个从未存在的 id 必须不可
区分，一个乱写的头部也不该比它得到更多信息。`If-Range` 用 item 的 `ETag` 判定，而 `ETag`
是 **root CID**：它命名内容而不是一次传输，同一份字节的两个 item 共享同一个 `ETag`——
一个恢复中的 `<video>` 靠的是「关于内容的陈述」，不是一个传输编号。

响应体**按块流式**：一次下载的内存成本是「一个块」而不是整个 item，流被丢弃时停止读后续
块。`Content-Length` 必须声明完整长度，否则 `<video>`/`<audio>` 会把 `currentTime` 钳到 0，
即使 Range 是好的。下载（不带 Range）是唯一拿全量的路径。

**内联预览是 Host 的判定。** 只有不带活动内容的类型可内联（`text/plain`、
`text/markdown`、`text/csv`、`application/json`、`application/pdf`、常见位图、常见
音视频）；`text/html`、`image/svg+xml` 等一律 `attachment`，避免在他人会话里执行脚本。
文本响应的 `Content-Type` 必须带 `; charset=utf-8`，否则浏览器按默认编码解码，UTF-8
中文变乱码。文件名与 MIME 进响应头前必须过滤控制字符、引号与 CR/LF；文件名按
`filename="ascii"` + `filename*=UTF-8''pct` 双写。

**页面定位。** 首屏说明 conex 是什么（可嵌入的双向能力路由内核），消息与文件是它做的
两件事而非产品本身。页面是**工作台**：顶部状态栏常驻本标签页的名字、短链接、分组 key、
同组在线数与全站在线数——「我是谁、我占着哪条链接」不能藏在某个标签后面。面板（消息 /
文件 / 状态 / 介绍 / 性能）只切换内容，不切换状态栏。默认面板是消息。

**通知是事件，不是会话。** 问候渲染为右上角的 toast，不是聊天气泡：一条提示到达、被读到、
然后自己消失，页面不会因为它而移动。区别只有一处有意义——**提问**（`ask: true`）带确认
表单与「确认并回复」按钮，且不自动消失，因为 Host 此刻在等一个人而不是等一个 socket。
空字段被省略而不是发成 `""`：发送方拿到更短的回答，而不是一个看起来已回答其实没答的字段。
