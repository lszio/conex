# Connected landing contract

状态：**L01 契约冻结 + L10 连接面板契约冻结**（2026-09-21）。L10 在 `schema/conex/dashboard.proto` 新增 `ConnectionListRequest` / `ConnectionListResponse` / `UiLinkSummary` / `AgentLinkSummary`；Rust/TypeScript/JSON Schema 由 `cargo xtask generate` 生成。L02–L09 边界不变。

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
