# 多主机数据服务与公共内容页面优化计划

> 执行方式：实施时使用 `executing-plans` 或 `subagent-driven-development`，按依赖逐项执行；勾选只代表已有运行证据，不代表仅完成代码。本文是优化方案与工作包计划，不是已实现能力声明。

**Goal:** 多台主机通过独立 Agent 注册预授权数据服务到同一 Host；公共页面按主机展示明确公开的 Notez 文档和附件，支持文本、DOCX、ZIP、MP4、图片及任意二进制下载。

**Architecture:** 保留中心 Host + 主动反连 Agent + endpoint 路由。控制消息负责目录、元数据和授权，内容通过有界字节传输交付；Host 不默认复制所有远端数据。公开访问是资源策略，不是绕过租户或匿名开放整个存储。

**Tech Stack:** 现有 Rust/Tokio/Axum、protobuf 类型源、WSS、TypeScript SDK 与原生浏览器能力；不新增数据库、消息队列或独立媒体服务作为默认前提。

---

## 1. 范围与设计取舍

### 1.1 选定方向

| 方案 | 好处 | 代价 | 决定 |
|---|---|---|---|
| 按需经 Host 代理远端内容 | 复用反连，私有主机不需公网入口，不复制全部数据 | Agent 离线时不可读；Host 承担流量 | 采用 |
| 全量同步附件到 Host/对象存储 | Agent 离线也可读，方便 CDN | 增加同步、删除、权限撤回、空间与 GC 责任 | 不作为本轮前提 |
| 浏览器直连各主机 | 少一跳 | NAT、TLS、CORS、上游凭据和访问边界复杂 | 不采用 |

- 服务所有者仍需凭据；页面读者可匿名。匿名访问不能用于 Agent 注册和写入。
- 每台主机独立 `agent_id`；端点 ID 在 Host 内唯一；同 ID 重连是替换，不是多机负载均衡。
- 本轮保持管理员预授权注册：Host 配置 Agent、端点、方法、公开范围，Agent 注册激活它们。新增端点允许通过配置更新与 Host 重启完成，不承诺无重启自助开户。
- 公共页面默认聚合同一展示租户内明确公开的端点；不同私有租户仍隔离。跨租户公共聚合需要独立共享授权模型，不用全局 guest 绕过现有隔离。
- DOCX/ZIP 首先必须能够完整下载；本计划同时包含 DOCX 安全预览和 ZIP 安全目录查看，不将二进制传输冒充格式预览。
- 不增加 API 版本前缀或平行旧接口。契约变更统一迁移 Host、Agent、SDK、页面、向量和文档；保留内容寻址格式自身必要的格式标识。
- 不引入写回、协作编辑、P2P、跨主机全文索引、视频转码和离线副本。这些不属于本次数据浏览目标。

### 1.2 已核实基线

- `cargo xtask e2e --suite connected-landing-web` 已通过：真实 Host + 两个 Agent，登录、目录和远程文本读取。
- Agent 实际仅支持 UTF-8 `.md/.org`，单文件 256 KiB 上限；普通 `.txt`、非法 UTF-8 二进制和超限文件的拒绝已实跑。
- guest 固定为 `guest/demo`；production 目录为空已复现；第 9 个新匿名会话返回 429 已复现。
- 源码确认远端分页缺失、连接目录缺租户过滤、blob 资源授权未贯通。后两项实施时需补跨身份行为回归，不能只凭注释判定安全。
- 本机 E2E 不等于跨主机 TLS 或浏览器媒体验收；历史 1 GiB blob 测试不等于远端附件可用。

## 2. 工作包与依赖

```text
M0 契约与安全不变量
  ├─ M1 公共入口与隔离
  └─ M2 注册生命周期与通用 provider
       └─ M3 远端内容与浏览器交付（同时依赖 M1）
            ├─ M4 Notez 适配
            └─ M5 页面展示与格式预览
                 └─ M6 真实跨主机验收（同时依赖 M4）
```

M1 与 M2 在 M0 完成后可并行；M4 与 M5 在 M3 契约和运行路径稳定后可并行。协议源、Host 分派及生成文件由同一集成人负责，避免各层自行定义第二套类型。

## M0：冻结内容契约与安全不变量

**目标：** 先定义统一资源语义，避免图片、视频、文档分别发明一套传输接口。

**涉及文件：** `schema/conex/{source,blob,chunking,agent,endpoint,stream}.proto`、`crates/conex-source/src/contracts.rs`、`crates/conex-core/src/contracts.rs`、`docs/contracts/connected-landing.md`。

- [x] 明确资源定位由 `endpointId + resourceId` 构成；租户来自认证上下文，不能信任请求自报。`provider_id` 不得代替唯一端点定位。（`BlobAccess.endpointId`，wire 已删除 `providerId`）
- [x] 扩展 `source/read` 内容结果：小型受支持文本可内联；附件和大文本返回内容描述及可解析的内容引用。复用现有 `BlobRef/BlobAccess`，补齐端点定位与 revision，不另建一套附件身份系统。（`text ⊕ content` 互斥由校验器强制；`conex-source/tests/read_contract.rs`）
- [x] 区分内容身份与读取定位：CID 仅用于已计算的不可变字节；远端可变文件允许使用资源 + revision 定位，未计算 CID 时明确缺省，禁止用路径或 revision 假充 CID。不得为开始播放而先整文件读取并计算 hash。（`BlobRef.cid` 改 optional，校验器拒绝非法 CID）
- [x] 明确 `blob/get` 的授权定位与读取模式：Host 已提交内容按授权根及块读取；远端内容按端点、资源、revision 和范围读取。两者输入使用互斥结构，避免同一个 offset 同时被解释成块内偏移与文件偏移。（oneof `committed/remote`；`conex-core/tests/blob_get_contract.rs`）
- [x] 冻结范围语义：offset/length 为字节，64 位安全编码；拒绝溢出、非法范围和超配额；读取结束返回实际长度及 EOF；revision 不匹配返回 `stale_revision`，不能拼接两个版本。（十进制字符串 + `checked_add` + `length>=1` 校验器；`stale_revision` 处理路径在 M3 接通）
- [x] 元数据统一包含文件名、MIME、大小、revision、目录项类型；未知类型使用 `application/octet-stream`。MIME 不等于允许执行内容。（`ResourceSummary` + `EntryKind`；producer 已迁移）
- [x] 授权统一覆盖目录、读取、范围读取、have、上传恢复、提交、pin/unpin、cancel。CID、upload ID 和 pin ID 都不是授权凭据。（契约语句冻结于 `docs/contracts/connected-landing.md` §8；所有权强制执行在 M1.2 落地）
- [x] 从 `.proto` 生成 Rust/TS/schema，删除被替代的手写 wire 结构，并迁移全部调用方；不保留双轨契约。（`cargo xtask generate`；旧 `chunkCid/rangeOffset/rangeLength` 字段号 reserved，全部 Rust/TS/测试调用方已迁移）

**验收：** 一套类型可描述 md/txt/docx/zip/mp4/png/bin；JS 大整数不丢精度；错租户、错资源、混合定位、越界范围和过期 revision 均有确定错误。复用 `cargo xtask generate` 与现有契约向量门禁。

## M1：修复公共入口与内容权限

### M1.1 访客会话与公开目录

**修改：** `crates/conex-host/src/{config,web_auth,tickets,catalog,ui_links,broker,serve}.rs`、`deployment/host.toml.example`、`xtask/src/landing_demo.rs`。

- [x] 从配置明确解析公共展示租户及匿名策略主体；拒绝 guest 配置与目标租户矛盾，移除 `demo/host.local` 硬编码。（`[web_guest]` 配置节 + load 期矛盾/碰撞校验；`connected_landing_config.rs`）
- [x] 保留匿名读取的共享授权策略，但将匿名配额与登录主体的 8 会话限制分开；采用可配置的有界全局匿名会话数、空闲过期和签发速率限制，不改成无限会话。（`max_sessions`/`idle_ttl_ms`/`max_issue_per_minute`；`web_auth.rs::guest_sessions_obey_configured_bound_independently`）
- [x] 无效或过期 guest cookie 可重新取得匿名会话；登录用户会话失效不得静默降级并掩盖认证错误。保留 Origin、CSRF、Secure/HttpOnly 和一次性 ticket 约束。（墓碑判定 + `stale_guest_cookie_reissues_but_authenticated_does_not_downgrade`）
- [x] 所有目录投影按租户和资源可见性过滤；公共访客不获取其他访客的会话活动，`connection/list` 不向普通访客暴露管理用 Agent 资源声明。（`BrokerCall.link_id` + guest 投影过滤；`endpoint_catalog.rs::connection_list_isolates_guest_visitors_and_hides_agent_links`）
- [x] 修复 demo 的 guest 策略与部署示例，公开策略只授权示例共享目录，私有资源保留拒绝行为。（`landing_demo.rs` guest 只读策略 + `deployment/host.toml.example` `[web_guest]`）

**回归位置：** `crates/conex-host/tests/{web_auth,ui_links,endpoint_catalog,connected_landing_config}.rs`。

**验收：** production 配置下无凭据访问可见公开端点；20 个独立匿名会话在配置额度内均成功；超过配置额度返回明确 429；会话过期可回收；同名主体不同租户、不同匿名访客不串列；私有文件始终被拒绝。

### M1.2 Blob 所有权与资源授权

**修改：** `crates/conex-host/src/broker.rs`、`crates/conex-content/src/{content,store}.rs`、M0 对应合约。

- [x] 在共享内容存储之上记录 tenant、资源引用、上传 owner、pin owner；物理内容可去重，逻辑授权不得因相同 CID 合并。（`Owner` 写入 UploadState/CommitRecord/PinRecord，serde default 向后兼容）
- [x] 所有 blob 入口走统一资源策略；读取块需要证明该块属于获授权根，不能仅提供一个合法 CID。（`blocks_by_root` 可达集 + `chunk_owned_by`；`blob_ownership.rs`）
- [x] staging 恢复、chunk、commit、cancel 检查上传归属；pin/unpin 检查引用归属；have 不泄露未授权内容存在性。（resume/cancel/commit/pin/unpin owner 校验；have 按 owner 内容求值）
- [x] 在入口和累计写入处强制字节/分块/租约上限，commit 校验声明长度和真实长度；禁止只返回上限却不执行。（1 GiB 入口、CHUNK_SIZE 固定、24h 租约、`received_bytes` 累计上限、commit 实长求和校验）
- [x] 未绑定所有权的历史 staging 不自动认领；清理过期 staging。历史 committed 字节保留，但在管理员显式绑定授权资源之前不可公开，不能静默删除已有内容。（owner=None 拒绝认领；`purge_expired_uploads` + recover 期过期清理；无主 commit 不可经 blob/get 读取）

**回归位置：** `crates/conex-host/tests/host_p1_integration.rs`、`crates/conex-content` 现有测试；必要时新增专门的 blob 授权行为测试文件。

**验收：** 用户 A 上传后，用户 B 即使知道 CID/uploadId/pinId 也无法读取、取消、续传或解 pin；同租户未授权资源同样拒绝；Host 重启后规则保持；撤回公开策略后新读取被拒绝。

## M2：注册生命周期与通用 Agent 数据源

**修改：** `crates/conex-host/src/{agent,remote,ws_transport,catalog}.rs`、`crates/conex-agent/src/{config,runtime}.rs`、`crates/conex-assembly/src/lib.rs`、`crates/conex-provider-fs/src/{read,list,search}.rs`。

- [x] 新连接认证及逐端点注册成功后才替换旧连接；失败注册不踢掉健康 Agent。旧连接 cleanup 继续受代次保护。（`RemoteConnections::stage/activate/discard`：链接在 WSS upgrade 只暂存，注册成功才 swap-in 并关闭旧链；失败走 discard，旧链不受影响；代次 fence 保持；`agent_link.rs` + connected-landing e2e 生命周期）
- [x] 按 `AgentEndpoint` 验证端点、方法、资源根，不将所有端点权限扁平合并。ready 表示该端点已在当前连接代次被接受，而非 Agent 在线即全端点在线。（`AgentAuthorization.endpoints` 逐端点 review，accepted/rejected 分离响应；`AgentRegistry.accepts_endpoint` + `RemoteConnections::is_ready_endpoint` 驱动 catalog；删除 provider_id∪endpoint.id 混合）
- [x] Agent 复用既有 provider 注册/调用机制，删除硬编码的平行 FsRoot 分派；本地 fs 与远端 fs 使用同一目录、读取和分页语义。（agent runtime 改用 provider-fs `ReadHandler/ListHandler/SearchHandler` + `CallContext`/claim/ExecutionIo；`LocalProvider` 硬编码 match 删除；`RequestContext` 增加转发 principal/tenant）
- [x] 目录覆盖全部常规文件，不再只列 md/org；文本搜索仅作用于明确支持的文本，不能把二进制强行转 UTF-8，也不能假称已支持附件全文检索。（scan 列全部常规文件 + octet-stream 回退 + revision；搜索限 md/org/txt，非 UTF-8 静默跳过；read 小文本内联、其余返回 BlobRef 内容引用）
- [x] 复用已有快照游标机制；游标绑定租户/主体/端点/查询，不能在另一个端点使用。页面和 SDK 消费 next cursor。（SnapshotKey 六元组绑定既有实现经 handler 复用生效；agent 侧 next_cursor 不再恒为 None；`pagination.rs` 151 项无重复验收测试）
- [x] 文件 I/O 与扫描使用受限工作任务；单 Agent 的慢请求不能阻塞心跳与其他端点。传递剩余 deadline 和取消，保持有界队列与并发。（dispatch 独立任务 + Semaphore(8) + `timeout_budget_ms` deadline；回复经有界 mpsc 通道回写）

**回归位置：** `crates/conex-host/tests/{agent_link,remote_source,ws_duplex,endpoint_catalog}.rs` 及 Agent/provider 现有测试。

**验收：** 两个 Agent 使用相同资源名返回不同内容；错误注册不影响旧连接；只注册一个端点时另一个不显示 ready；151 个资源完整分页且无重复；100 个以上端点可枚举；关闭 A 后 B 仍可读取。最后一项必须真的停止 A 进程，不能只靠输出文字声明。

## M3：贯通远端字节传输与浏览器内容入口

### M3.1 Agent → Host 有界内容读取

**修改：** `crates/conex-agent/src/runtime.rs`、`crates/conex-host/src/{remote,broker,ws_transport}.rs`、`crates/conex-core/src/stream/`、`sdk/typescript/src/{content,ws-client,client}.ts`。

- [x] 实现 M0 的远端内容读取；Host 授权后路由到固定 Agent/端点，Agent 再核对本地根和方法范围。（`Broker::content_range_bytes` 先走 `source/read` 授权探针（同一 route/contract/tenant/policy 路径），再按 catalog 的 agent 绑定路由；Agent `dispatch_chunk` 校验端点归属与 resources 范围）
- [x] 使用现有 protobuf 二进制 Profile 与流控机制承载有界数据帧，接通 Agent Binary 接收路径；控制面仍可保留 JSON，禁止整文件 base64 或把任意二进制转字符串。（agent 链协商 `conex-protobuf-wss`，新增 `DataChunk` 消息体（rpc.proto body 5）承载原始字节；host `RemoteLink::request/request_chunk` 与 agent 控制面均走 prost 二进制；JSON 平面显式拒绝 DataChunk）
- [x] 复用现有 256 KiB 分块作为默认传输粒度；窗口、并发和缓冲均受额度约束。只有完整内容按既有 CID 格式校验时才声明内容寻址结果，不给任意 range 冒充完整块 CID。（每请求上限一个 `CHUNK_SIZE`；agent 并发 Semaphore(8)+mpsc(8)；host 链队列 `max_queued` 预算；range 响应只带 revision/eof，无 cid）
- [x] 打开/读取期间绑定文件 revision；可变源提供稳定快照或前后版本验证，检测变化返回 stale_revision。无法保证一致快照的 provider 必须拒绝该保证，不伪造可续传。（读前 `stat` 得 mtime revision，与请求 revision 不符即 `stale_revision`；流式过程中 revision 变化截断；ETag/If-Range 绑定同一 revision）
- [x] 浏览器取消或 Agent 断线时释放读取句柄、任务和队列。续传仅允许同资源同 revision 的只读范围，不自动重放写操作。（`Body::from_stream` drop 即取消上游 pending；过期/断线返回 unavailable/timeout；写操作不重放）

**验收：** 随机二进制跨 Host/Agent 下载后 hash 完全一致；读取 MP4 中间范围只传输所请求范围及有界预取；1 GiB 文件内存不随文件大小线性增长；慢消费者有背压；中途改文件、取消和掉线返回确定结果。

### M3.2 浏览器原生 HTTP 内容交付

**修改：** `crates/conex-host/src/{serve,http}.rs`；新增 `crates/conex-host/src/content_http.rs` 作为同源 HTTP 映射，不建第二套授权服务。

- [x] 新增无版本前缀的同源 `GET/HEAD /content`，参数使用 endpointId/resourceId/revision，正确 URL 编码；禁止携带长期 token 或绝对文件路径。（`crates/conex-host/src/content_http.rs`，Query 反序列化，无 token/路径参数）
- [x] 入口复用 Web 会话与公开策略，内部调用同一资源授权和远端读取，不从 URL 直接拼接磁盘路径。（cookie → `WebAuth::session_from_headers`；`source/read` 授权探针；字节取自 `content_range_bytes`，不触碰磁盘路径）
- [x] 支持单范围 Range，包括开放尾部和 suffix；实现 200/206/416、Content-Length、Content-Range、Accept-Ranges、ETag/If-Range；多范围请求按 HTTP 规则忽略 Range 返回 200，不伪造 multipart 支持。（`parse_single_range`；`Accept-Ranges: bytes`；`ETag: "rev-<revision>"`；If-Range 不匹配降级 200；`bytes */total` 416）
- [x] MIME 和 Content-Disposition 防头注入；未知二进制强制 attachment；HTML/SVG 等活动内容不在 Host 同源内联执行；设置 nosniff。公开和私有响应不得共享错误的缓存策略，撤权后不得由公共缓存继续提供私有内容。（CR/LF/NUL 拒绝；白名单 inline，其余 attachment；`filename*` 百分号编码；`nosniff`；`private, no-store`）
- [x] HTTP 响应边读边发，停止消费立即取消上游；读超时、Agent 离线和版本变化不返回截断内容却标记完整成功。（`Body::from_stream` 逐切片拉取；drop 即取消；revision 变化→流错误，不静默截断）

**新增验证：** `crates/conex-host/tests/content_http.rs`，覆盖字节结果、范围语义、会话/资源授权及响应头；浏览器实际使用 img/video/下载链接验证，不只验证 RPC。

## M4：接入真实 Notez 服务

**目标：** 接入 Notez 文档、资源身份和附件，不直接暴露其数据库文件或整个工作目录。

**修改边界：** `crates/conex-agent/src/config.rs`、`crates/conex-assembly/src/lib.rs`、`Cargo.toml`；新增 `crates/conex-provider-notez/{Cargo.toml,src/lib.rs}`，仅在现有 HTTP catalog 不能无损覆盖 Notez 语义时创建专用 provider。

- [ ] 执行前定位实际 Notez 仓库与运行实例，读取其正式接口及授权规则，记录文档枚举、正文、附件元数据、原始字节、revision 和范围能力映射；不能猜测 Notez URL 或用 mock 代替此步骤。
- [ ] 优先复用已有 HTTP provider 的 TLS、凭据解析和请求限制；Notez 服务凭据只留在 Agent，浏览器和中心目录均不暴露上游地址/凭据。
- [ ] 映射 Notez 稳定文档/附件 ID 到资源 ID，支持中文、空格、同名附件和不存在资源；不要用显示名作为唯一键。
- [ ] 公开范围取 Host 授权、Agent 配置和 Notez 上游权限的交集；不能借 Agent 的广权限服务 token 绕过 Notez 分享边界。
- [ ] 如 Notez 缺少必需的原始字节或稳定 revision 接口，先在 Notez 正式服务层补齐再完成适配；该项为功能交付依赖，不以导出目录冒充完成。

**验收：** 两台真实 Notez 主机各有一篇文档及 docx/zip/mp4/图片/bin 附件；同名附件按主机正确区分；正文、附件字节与上游一致；未分享附件不可见且不可通过直接 URL 读取。

## M5：公共页面与格式展示

**修改：** `web/src/main.ts`、`web/src/view.ts`、`web/src/style.css`、`web/index.html`、`sdk/typescript/src/{index,client,ws-client,content}.ts`。格式预览确需新模块时新增 `web/src/preview.ts`。

- [x] 页面按主机/端点展示目录与在线状态，支持分页、打开资源、返回目录和错误提示；刷新目录不能重置正在输入的搜索条件或当前媒体播放。（`web/src/browser.ts` Browser 类：目录树/面包屑/加载更多/分类错误；`visibilitychange` 刷新不动 media 元素，实测通过）
- [x] 文本安全呈现；Markdown 不默认执行原始 HTML。大文本分页/增量读取，不一次性载入任意大文件。（文本以 `textContent` 渲染、脚本计数为 0 实测；`TEXT_CHAR_LIMIT` 512KiB 截断；超限资源仅下载）
- [x] 图片使用原生 img；MP4 使用原生 video + 同源 content URL，验证开始播放、拖动、暂停和取消。（`img.preview-image` naturalWidth 解码验证；`video.controls` + Range 支持拖动；`data-resource` 在刷新后保持）
- [x] DOCX 提供预览与原文件下载；实现前检查 Notez 是否有可复用预览接口，否则选定一个维护中的解析器，不手写 OOXML 引擎。预览位于隔离环境，阻止外链自动加载和文档内容获取 Host 凭据。（Notez 未接入（M4 依赖），选定 mammoth 1.13（BSD-2，维护中）；`stripDangerousMarkup` 清洗 + 外链全部中和，恶意 DOCX 样本实测无 script/外链）
- [x] ZIP 提供条目目录、压缩前后大小及原文件下载；限制归档总大小、条目数、展开字节及压缩比，不自动递归解压，不接受目录穿越。DOCX 同样受 ZIP 解包额度约束。（fflate 列目录不展开载荷；`reviewArchiveEntries` 拒绝 `..`/绝对路径/控制字符/压缩比 >200/超 32MiB；bundle.zip 穿越条目实测被拒）
- [x] 任意二进制显示名称、MIME、大小、revision 并可下载；不显示乱码，不假装支持预览。（`.binary-meta` 显示四元组；SVG 不内联仅下载（活动内容）；实测 8B bin 无乱码）
- [x] 权限拒绝、文件缺失、Agent 离线、版本变化、超限、格式不支持分别显示；键盘可操作，图片替代文本和基本焦点管理齐备。（`failureKind` 八类映射；Tab 焦点实测 BUTTON；img alt + aria-live/role=alert；1440/390 双视口截图）

**验收：** 1440px/390px 真实浏览器分别完成文本阅读、图片查看、视频拖动、DOCX 预览、ZIP 条目查看、二进制下载；下载与源 hash 一致；恶意 HTML/SVG/DOCX 外链和 ZIP bomb 样例不会突破隔离或资源限额。

## M6：运行验收、发布与证据

**修改：** `xtask/src/{e2e,landing_demo,check}.rs`、`docs/contracts/connected-landing.md`、`docs/runbooks/{connected-landing,dokploy}.md`、`docs/verification/connected-landing.md`、`README.md`。

- [ ] 扩展现有三进程 fixture：两个 Agent 的同名文件内容不同，含 UTF-8 文本、docx、zip、图片、mp4、随机二进制和一个 1 GiB 文件；仅使用可公开的合成样例。
- [ ] 在 xtask 注册 `multi-host-content`、`public-content-security` 两条场景，复用现有进程管理，不创建另一套测试宿主。
- [ ] 安全场景验证跨租户/跨资源/CID 已知的拒绝、guest 配额、链接隔离、撤权、路径穿越、上传归属与缓存行为。
- [ ] 生命周期场景真的停止 Agent A，确认 A 离线、B 可读；重连后新代次恢复；非法重注册不会中断原健康连接。
- [ ] 基础容量门槛：20 个并发访客，在受控网络下读取两个 Agent 的资源；两个 1 GiB 下载并行时额外用户态内容缓冲预算不超过 64 MiB，记录 RSS、吞吐、首字节和取消耗时，剔除 OS 文件缓存口径。排队达到上限明确拒绝，不无限堆积。
- [ ] 用至少两台真实主机和有效 TLS 验证反连、浏览器同源会话、图片/视频/下载；记录网络环境和文件 hash。本机 loopback 结果单独列出，不作为跨主机证据。
- [ ] 同步更新能力表和已知限制，清除与实际不符的“已支持”描述；运行证据只记录实际输出。未通过任一核心数据类型不得标记全功能交付。

### 执行命令

实施过程中每个工作包只运行覆盖变化的聚焦测试与真实 smoke；全部集成后统一执行以下门禁。新增 suite 必须先实现注册，不能把当前不存在的命令写成已通过。

```bash
cargo xtask generate
cargo test --workspace
bun test sdk/typescript/tests/
bun run typecheck
bun run build:web
cargo xtask e2e --suite connected-landing-web
cargo xtask e2e --suite multi-host-content
cargo xtask e2e --suite public-content-security
cargo xtask check
```

现有 `p1-stream-1gib` 作为旧能力回归保留，但不替代新场景中的远端 1 GiB 验证。

## 3. 开工与停止条件

1. 本计划批准后，从 M0 开始，不直接从页面预览开始。
2. 编辑符号前按仓库要求执行 GitNexus impact，导出符号变更同时查 LSP references；当前 GitNexus 不可用的问题须先恢复，不能跳过影响分析。HIGH/CRITICAL 风险先报告。
3. M1 安全回归未通过，不将新的 blob/内容入口开放公网；M3 字节完整性未通过，不以页面能打开证明数据正确。
4. Notez 真实接口和实例属于 M4 的执行依赖；可先完成通用 fs 链路，但不能将 fs 验收替代 Notez 验收。
5. 每个阶段交付可运行路径、聚焦回归与对应契约/操作文档；不新增平行版本、不做无关重构、不用无限队列掩盖容量问题。

## 4. 需求覆盖与交付标准

| 用户需求 | 工作包 | 必须看到的证据 |
|---|---|---|
| 多客户端注册到一个服务器 | M2/M6 | 两 Agent 独立身份、逐端点注册、代次切换与断线隔离 |
| 公共页面分别展示各主机 | M1/M5 | 无凭据可见公开目录，按主机区分，私有数据不可直读 |
| Notez 不同主机数据 | M4/M6 | 两个真实 Notez 实例正文/附件一致性与权限验证 |
| 文本 | M0/M2/M5 | md/org/txt、UTF-8、大小边界与安全渲染 |
| DOCX、ZIP | M3/M5 | 原文件完整下载、受限文档预览/归档目录 |
| MP4 | M3/M5 | HTTP Range 字节正确、真实浏览器播放与拖动 |
| 图片 | M3/M5 | 正确 MIME、原生展示、活动内容隔离 |
| 任意二进制 | M3/M5 | 不经过 UTF-8 转换，下载 hash 一致 |
| 大文件与多访客可用 | M1/M3/M6 | 配额、背压、取消、1 GiB 有界内存及并发场景 |

**总体验收：** 公共读者不输入管理凭据即可按主机浏览明确公开的 Notez 数据；所有要求的数据类型都有真实交付路径；一个主机掉线不影响其他主机；未授权内容无法通过目录、直接地址、已知 CID 或上传标识绕过权限；证据来自真实程序和浏览器，而非仅类型生成或 mock。
