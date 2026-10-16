# 落地页文件传输、预览与剪贴板共享优化计划

> 执行方式：实施时使用 `executing-plans` 或 `subagent-driven-development`，按依赖逐项执行；勾选只代表已有运行证据，不代表仅完成代码。本文是优化方案与工作包计划，不是已实现能力声明。

**Goal:** 落地页文件场景从「内存整块 + 整份拷贝 + 3 秒全量轮询」改造为「磁盘分块 + 断点续传 + Range 预览」，支持大文件；剪贴板内容以显式确认的方式在同组客户端间共享，复用同一条交付链路。

**Architecture:** `ShareStore` 只保留元数据行（含 `rootCid`），字节下沉到独立的 `conex-content` 块存储；上传变为「声明 → 分块 → 提交」三段式，读取统一走 `read_range(id, offset, len)`；下载与预览都从同一个 Range 入口出，主机决定 inline。隔离仍是 `group_key` 一次 lookup，这是全仓库唯一不允许被拆散的保证。

**Tech Stack:** 现有 Rust/Tokio/Axum、`conex-content` 块存储、`conex_proto::cid`、WSS notification、原生浏览器能力（`crypto.subtle`、`ClipboardItem`、`Range`）；不新增数据库、对象存储、消息队列或独立媒体服务。

---

## 1. 范围与设计取舍

### 1.1 选定方向

| 方案 | 好处 | 代价 | 决定 |
|---|---|---|---|
| 复用 `conex-content` 作为字节后端 | 分块、receipt、崩溃恢复、CID 校验、原子写全部已有；不写第三套存储引擎 | 需要 root 隔离与归属适配 | 采用（独立 root 实例） |
| 自己写一套 staging 落盘 | 不依赖 P1 是否启用 | 重复实现 receipt/恢复/GC，出现第二套事实源 | 不采用 |
| 沿用内存 + 提高上限 | 改动最小 | 宿主内存被匿名上传撑爆，8 MiB→1 GiB 直接变成 DoS 面 | 不采用 |
| 浏览器直连 P2P | 少一跳 | NAT、TLS、凭据与组隔离全要重做；组隔离是 host 的职责 | 不采用 |
| 剪贴板走独立 store/路由 | UI 语义干净 | 下载、预览、Range、隔离要再实现一遍 | 不采用，作为 `kind: "clip"` 复用文件链路 |
| 剪贴板自动广播 | 体验最顺 | 用户从别处复制的内容被隐式外发，是数据泄露面 | 不采用，必须显式确认 |

- **不放松既有安全边界**：`text/html` / `image/svg+xml` 仍是下载-only（`share.rs:62`），clip 同样适用。
- **不引入平行接口**：新能力挂在既有 `/web/files*` 路由族下，不做 `/api/v2`。
- **不为 clip 破例隔离**：clip 与 file 走同一个 `group_key` lookup。
- 本计划不做：浏览器端 ZIP/DOCX 内嵌预览引擎复活（M5 已删除的那套）、视频转码、缩略图服务端生成、写回与协作编辑。前两项属于独立的展示增强轮次，不在传输链路内。

### 1.2 已核实基线

- 字节仅存内存：`share.rs:53` `bytes: Arc<Vec<u8>>`；上限 `MAX_FILE_BYTES 8 MiB` / `MAX_GROUP_BYTES 32 MiB` / `MAX_FILES_PER_CLIENT 32`（`share.rs:29-33`）。
- 下载整份拷贝：`share_http.rs:212` `Body::from(bytes.to_vec())`；无 `Accept-Ranges`、无 `ETag`、`Cache-Control: private, no-store`（`share_http.rs:230`）。
- 上传无分块：`share_http.rs:322` `read_bounded` 累积进单个 `Vec<u8>`；路由级 `UPLOAD_BODY_LIMIT`（`share_http.rs:43`、`http.rs:84`）。
- 3 秒全量轮询：`use-hello-page.ts:16` `POLL_MS = 3000`，每次重拉 `listFiles` + `client/list` + `client/status`。
- 预览全量拉取：`file-scene.tsx:544` `fetch(url).arrayBuffer()`，而 `PREVIEW_LIMIT` 只要 200 KiB（`file-scene.tsx:37`）。
- 已有但未接入落地页的成熟实现：`content_http.rs:288` 单 Range 解析、`:245` `Accept-Ranges`、`:258` `ETag`、`:135-144` `If-Range` 降级、`:175-232` 流式 folding 与 drop 取消上游、`:280-282` 声明 `Content-Length`（无它 `<video>` 的 `currentTime` 被钳到 0）。
- 已有但未接入落地页的分块上传实现：`broker.rs:746-752` 先验 CID 再落盘；`content.rs:227-241` `put_chunk` 内容寻址写入；`store.rs:334-364` receipt 持久化；`store.rs:131/160` 重开时 `recover()` 重建 upload 状态。
- **GC 会删在途分块**：`store.rs:619-697` `collect_garbage` 删除「不在任何 committed root 可达集且 refcount 为 0」的块；未提交的分块正落在此列。这是 P1 已存在的隐患，share 链路不能继承。
- 两种演示拓扑都配置了三个 P1 root（`xtask/src/landing_demo.rs:179-183`、`web/dev.ts:27-29`），但 share 场景不能以 P1 启用为前提。
- `web/package.json` 的 `fflate` / `mammoth` 已无引用（M5 删除后的死依赖）。

## 2. 影响面（改前必读）

GitNexus CLI 在当前环境无法完成依赖安装，本次以精确调用方枚举替代图分析；后续实施时必须用 `node .gitnexus/run.cjs impact <symbol> --direction upstream --repo .` 复核。

| 拟改符号 | 直接调用方 | 说明 |
|---|---|---|
| `ShareStore` | `serve.rs:354`（构造）、`share_http.rs:158/208/278`、`ws_transport.rs:1012`（断开回收）、`tests/http.rs:176` | 构造点唯一，方法调用点共 4 处，改造可控 |
| `SharedFile` / `inline()` | `share_http.rs:303`（序列化）、`file-scene.tsx:27`（类型） | 新增字段对旧页面是增量，不破坏 |
| `share_http` 四个 handler | `http.rs:80-87`、`lib.rs:39`、`tests/share.rs` | 路由表集中在一处，便于拆子模块 |
| `refresh_shared_counters` | `share_http.rs:170/284` | 计数来源，改动后仍需刷新 |
| `listFiles` / `uploadFile` / `downloadFileUrl` | `use-hello-page.ts:13`、`file-scene.tsx:27` | 前端仅两处消费者 |
| `FileScene` | `app.tsx:14/124` | 入口单一 |

结论： blast radius 小且集中，适合一次性分层改造；主要成本在 `share_http` 单文件继续膨胀（现 481 行），应拆为 `share_http/{mod,uploads,downloads,clip}.rs`。

## 3. 架构与数据流

```text
浏览器                            Host                          磁盘
──────                            ────                          ────
选择文件
  ├─ crypto.subtle 分块算 CID
  ├─ POST /web/files/uploads   → 元数据校验 + 配额预扣 → begin_upload  staging/<id>/
  ├─ PUT .../chunks/{i}        → 验 CID → put_block            blocks/<aa>/<bb>/<rest>
  ├─ HEAD .../uploads/{id}     → 已收计数 N                    （内容寻址，重传即去重）
  ├─ POST .../commit           → 复算 root → 落 refs/ → 插元数据行 → SharedFile{rootCid}
  └─ （崩溃/刷新后凭 uploadId + HEAD 续传）

展示
  ├─ GET /web/files             → 只含元数据 + previewBytes + kind，If-None-Match → 304
  └─ WSS push conex/share-changed → 失效通知，替代轮询

预览（只取头部）
  └─ GET /web/files/download?id=  Range: bytes=0-262143 → read_range → 206 + Content-Range

下载（全量，唯一全量路径）
  └─ GET /web/files/download?id=  无 Range → 流式 folding（复用 content_http 模式）

剪贴板
  ├─ 用户点「共享剪贴板」 → POST /web/files/clip → kind:"clip" 元数据行（复用同链路）
  └─ 同组 push + toast「复制」→ navigator.clipboard.writeText
```

隔离不变量保持 `share.rs:174` 的形态：`get(id, caller_group)` 对未知 id 与跨组 id 同样返回 `None`；新增的 upload/commit/clip 路径一律先解析 `entry.group_key`，file id 与 uploadId 都不是访问令牌。

## 4. 工作包与依赖

```text
M0 契约冻结与不变量
  ├─ M1 存储改造（元数据 + 块后端）
  ├─ M2 分块上传与断点续传（依赖 M1）
  ├─ M3 Range 下载与预览（依赖 M1）
  ├─ M4 展示性能（依赖 M1，可与 M2/M3 并行）
  └─ M5 剪贴板共享（依赖 M1 + M3）
       └─ M6 真实浏览器验收
```

## M0：冻结契约与安全不变量 ✅

**目标：** 先定单一类型源与可测不变量，避免 Host、SDK、页面各定义一套。

**涉及文件：** 新增 `schema/conex/share.proto`、`schema/generated/*`、`docs/contracts/connected-landing.md`、`crates/conex-host/src/share.rs`。

- [x] 在 proto 中定义 share 上传/提交/clip 的请求响应类型与 `conex/share-changed`、`conex/clip-added` notification（沿用 `rpc.proto:72` `Notification{method,params}` 与 `conex/client-pong` 的既有形态）。`cargo xtask generate --check` 通过，生成文件未手改。（`schema/conex/share.proto`，16 个消息 + 2 个 notification；`schema/generated/{rust/jsonschema}` 与 `sdk/typescript/src/generated/conex/share.ts` 由 `cargo xtask generate` 产出）
- [x] 冻结安全不变量，每条都要有对应用例（详见 §5）——M1 落地的部分已有对应用例，见 §9。
- [x] `share` 场景的 HTTP JSON 平面与 proto 的关系写清楚：proto 是类型单一源，HTTP 路由是浏览器传输层。HTTP 路由的字段名与 proto 消息逐字对应，`crates/conex-host/src/share_http.rs` 只做 transport（header/query/cookie），不重复定义字段表。
- [x] `chunkSize` 固定为 `conex_proto::cid::CHUNK_SIZE`（256 KiB），不协商（`share_blocks::SHARE_CHUNK_SIZE`）。

## M1：存储改造——元数据行 + 独立块后端 ✅

**目标：** 字节出内存，`ShareStore` 只剩小元数据；不动隔离形态。

**涉及文件：** `crates/conex-host/src/share.rs`、`crates/conex-host/src/share_blocks.rs`（新）、`crates/conex-host/src/config.rs`、`crates/conex-host/src/serve.rs`、`http.rs`、`host.toml` 示例。

- [x] `SharedFile` 去掉 `bytes`，改为 `root_cid` + `kind` + `preview_bytes` + `expires_at_ms`；`list()`/`get()` 不再携带任何字节。
- [x] 新增 `[share] share_root` 配置，**独立于 `content_root` 单独开一个 `conex-content` 实例**。理由：`collect_garbage` 会删掉不在任何 committed root 可达集里的块，与 blob 通道共 root 会互相误删。`share_root` 缺省取 `content_root` 的兄弟目录，且启动时禁止两者相同（`config.rs` 校验 + `connected_landing_config.rs` 用例）。
- [x] 归属适配：`ClientEntry::owner()`（`clients.rs`）构造 `conex_content::Owner`。uploadId 因此不是凭据，陌生人五项操作全部 `Forbidden`。
- [x] **share root 永不 GC**：回收只走 hold 与租约。为此给 `conex-content` 增加了 `hold_root`/`release_root`/`reclaim_all_committed` ——块 refcount 回答「有多少个 committed root 可达」，hold 回答「还有多少个活着的 item 在用它」，只有后者能决定何时可以删。这是本轮唯一一处新增的存储原语。
- [x] 未配置存储时的行为：场景路由全部挂在 `attach_p1` 内（`http.rs:73-91`），因此**没有 P1 root 的 host 根本不会挂载这些路由**，`[share]` 只在场景可达处才有意义。`ShareStore::new()` 的 unbacked 形态保留为防御分支，读返回 `unavailable`。（原计划的内存回落已取消：一个场景一套引擎，不做第二套事实源）
- [x] `in_flight` 上传计入磁盘配额：M2 落地（`begin_upload` 即绑定 `declared_size_bytes`，存储侧已有累计上限 `store.rs:349`）。

**M2 新增用例（12 + 浏览器 6）：**
1. **断点续传不需要客户端存任何状态。** 原计划让客户端把 `{uploadId}` 存进 `sessionStorage`、断开时由 `remove_owner` 取消未提交 upload。实际做法相反：staging 按**内容身份**绑定（owner + size + leaf CID 列表），客户端重新声明同一个文件就会**重新找到同一个 staging** 并拿到已收块数。于是「网络中断」和「标签页刷新」是同一个用例、同一段代码，`sessionStorage` 整个不需要。代价是断开不取消 staging，靠租约（可配，默认 24h，上限 24h）与启动回收兜底。
2. **root CID 由主机算，不由浏览器算。** 原计划让浏览器用 SDK 的 `contentCid` 算根 CID（要引 `multiformats`，bundle +40KB）。改为浏览器只对每个分块算 sha2-256（原生 `crypto.subtle`，`web/src/lib/cid.ts` 约 90 行，零依赖），把 leaf CID 列表交给主机，由 `conex_proto::cid::content_cid_for_parts` 派生根 CID。结果：一份 manifest 实现、零新依赖、且 chunkCid 与冻结的 conformance 向量对齐（`web/src/lib/cid.test.ts`）。
3. **`UploadState` 增加 `declared_chunk_cids`（`#[serde(default)]`）**：init 时声明、重启后仍在，因此「主机重启后还能逐块校验」。为此 `ContentStore::begin_upload` 多一个参数，P1 调用方传 `&[]`——P1 通道是边传边声明，本来就没有前置列表。
4. **`Put` 分块的响应返回的是「主机已持有的总数」而不是本次新增数**：第一版返回了插入前的计数（`Upload::state()` 是快照，不是活视图），客户端会回退一块重传。修法是 `expected_index + 1`，用例直接抓到。
1. **M3 的 Range 支持提前到 M1**。页面预览改为按 `previewBytes` 取一段，而 M1 结束时下载路径若不支持 Range，预览会全量下载——所以 `range_scope`（`share_http.rs`）随 M1 一起落地，M3 只余多 Range/`If-Range`/ETag。
2. **组配额不再因「小于单文件上限」而拒绝**，改为自动跟随（`config.rs::share_limits`）。原计划是一条校验，实修时发现这是易错陷阱：运维只想放大单文件却得到不说原因的拒绝。
3. **重启回收改为「全部 reclaim」而非按 owner 重建**：场景语义是断线即弃，重启等于所有连接断开，遗留 committed root 一律回收；staging 保留（仍可续传）。

## M2：分块上传与断点续传 ✅

**目标：** 支持大文件与两种中断（网络中断、标签页刷新）的续传。

**涉及文件：** `crates/conex-host/src/share_http/uploads.rs`（新）、`crates/conex-host/src/http.rs`、`crates/conex-host/src/share.rs`。

- [x] 路由：`POST /web/files/uploads`、`PUT /web/files/uploads/{id}/chunks/{index}`、`HEAD /web/files/uploads/{id}`、`POST /web/files/uploads/{id}/commit`、`POST /web/files/uploads/{id}/cancel`。写操作带 Origin + CSRF（同 `check_mutation`），读只需 cookie。（`share_http/uploads.rs`，路由注册在 `http.rs`）
- [x] **严格顺序分块**：`index` 必须等于已收计数。单写方场景下顺序足够，而它让「断点」塌缩成一个整数——`HEAD` 返回 `x-conex-received-count`，客户端从 `file.slice(N * 256KiB)` 继续。`)
- [x] **断点不需要客户端记住任何东西**：见下面「实施时变更的决定」第 1 条。
- [x] 分块先验 CID 再落盘，复用 `broker.rs:746-752` 的判定顺序；坏块返回 `BadBlob` 且不写入 receipt。（验的 CID 来自 upload receipt 里客户端** init 时声明**的那份，不是请求头——否则客户端可以在线谎报一个宽松的 CID 把校验绕开）
- [ ] 单块 body 上限随分块大小放宽（per-route `DefaultBodyLimit`，不动全局 1 MiB：`http.rs:25`），并保留「声明的 `Content-Length` 撒谎也要在流式中复查」的现有做法（`share_http.rs:322`）。


- [ ] 配额：`[share] max_file_bytes`（默认 256 MiB，硬上限沿用 blob 的 1 GiB）、`max_group_bytes`、`max_host_bytes`、`disk_watermark`。匿名公开端点必须在上传前按声明大小拒超，而不是传完再拒。

## M3：Range 下载与预览 ✅

**目标：** 预览只取头部，下载才全量；下载流式、可断点、可被浏览器当作可 seek。

**涉及文件：** `crates/conex-host/src/share_http/downloads.rs`（新）、`crates/conex-host/src/share.rs`、`web/src/lib/files.ts`、`web/src/components/file-scene.tsx`。

**M3 实施时抓到三个真 bug（都不是预审发现的）：**
1. `range_plan` 的 `end` 没夹到 `total - 1`，`Content-Range` 会声明一个不存在的字节——浏览器会信这个头，等于指向一个 seek 不到的位置。
2. `take` 用排他边界算 inclusive 范围，每个 range 少一个字节。
3. `last` 写成 `(end - 1) / chunk`：`end` 是 inclusive 下标，「预览到第 0 字节」的请求会让它下溢。

第三个是一次**遍历所有起点 × 所有边界长度**的 sweep 用例抓到的
（`share_blocks.rs::every_range_of_a_small_item_reads_back_the_bytes_it_names`），另两个是边界断言抓到的。
三个都在同一段不到 20 行的算术里，而这段代码的失败方式是「给浏览器一个它没要的字节」——不报错。
所以逐边界断言加一次穷举 sweep 是必需的，不是装饰。

**M3 实施时变更的决定：**
1. **`If-Range` 与「畸形 range」分开**：RFC（以及既有 `content_http.rs`）里「无法解析」和「解析得出但无字节可答」是两回事，M1 把两者都退化成整个 item。M3 拆开：后者答 `416 + bytes */total`。这不违反「可区分的答案是探针」——那条约束说的是 id 是否存在、是否过期，而这里调用方已经知道 id 了。
2. **`ETag` 用 root CID**：它命名内容而不是一次传输，同一份字节的两个 item 共享同一个 ETag，正是 `If-Range` 需要的「关于内容的陈述」。

**M3 新增用例（6 Rust + 4 浏览器）：**
- `share_blocks`：`range_plan` 指向的块、skip/take 与边界、越界夹紧、整体越界无块、一次遍历 sweep 校验「声明服务的字节数 == 实际读到的字节」
- `tests/share.rs`：越界答 `416` 且带 `bytes */total`、后缀 0 字节答 `416`、`If-Range` 匹配按 Range 答 / 不匹配答整个 item、ETag 跨请求稳定、多块流式下载且 `Content-Length` 完整、跨块边界的 middle range、单字节 range
- 浏览器：下载响应带 ETag、`Content-Range` 精确、`If-Range` 两条路径


- [x] `GET /web/files/download` 支持 `Range`/`If-Range`，语义与 `content_http.rs` 对齐：单 Range、`206` + `Content-Range`、多 Range 与畸形值降级为 `200`（RFC 7233）、越界 `416` + `bytes */total` + `Accept-Ranges`、`ETag`、`If-Range` 不匹配时忽略 Range。（`share_http/downloads.rs::scope_of` 三答：Partial / Whole / Unsatisfiable）
- [ ] 响应改为 `Body::from_stream` + `try_unfold` 分片读取，删除 `share_http.rs:212` 的 `to_vec()` 全量拷贝；流被丢弃时取消上游（`content_http.rs:193-203` 的注释已说明此机制）。
- [x] `linkId` 仍从 query 进，校验方式不变。（M1 已做）
- [x] inline 白名单补 `audio/*`、`video/*`；html/svg 维持下载-only。（M1 已做）
- [ ] 列表新增 `previewBytes`（`min(size, 256 KiB)`，UTF-8 边界对齐）与 `previewMime`；响应带 `x-conex-truncated`，客户端不必先拉全量再发现被截断。
- [ ] 前端：文本预览改 `Range: bytes=0-<previewBytes>`；`<img>` 保持 `loading="lazy"`；PDF 用 `<object>` 依赖 Range 翻页；`file-scene.tsx:544` 不再 `arrayBuffer()` 全量。
- [ ] 已提交文件 id 永不复用（id 单调递增：`share.rs:136`），因此可按 id 做不可变缓存；`private, no-store` 只留给列表，下载用 `ETag` + `must-revalidate`。

## M4：展示性能 ✅

**目标：** 去掉 3 秒全量轮询，让文件变化变成推送。

**涉及文件：** `crates/conex-host/src/share_http.rs`、`crates/conex-host/src/clients.rs`、`crates/conex-host/src/ws_transport.rs`、`web/src/hooks/use-hello-page.ts`。

- [x] `GET /web/files` 增加 `ETag`（组内文件 id 有序拼接取哈希）+ `If-None-Match` → `304`，把轮询的常态成本降到一次 304。
- [x] 增删/提交成功后向同组在线链接推 `conex/share-changed`（复用 `clients.rs:76` 有字节预算的 `try_send`；推送不计入发送方字节预算的既有规则见 `clients.rs:31` 注释）。
- [x] 收到推送即刷新列表，轮询退化为 30 s 兜底；`visibilitychange`/`focus` 刷新保留（`use-hello-page.ts:243-250`）。
- [x] `list()` 在锁内直接构建 JSON（做了更小的改进：clone 仍然 cheap，原计划里那条 `Arc<SharedFile>` 的微优化反而会扩散到全表，因此放弃），避免每文件 `clone()` 后二次遍历。
- [x] `refresh_shared_counters`（M1 已用 `owner_totals()`）在大文件下不要全组 `list()` 再过滤，改为按 owner 索引直取（M1 已按 owner 建索引）。
- [ ] 验证首屏与轮询流量下降：用 `xtask bench` 或一次抓包给出前后对比，不写「应该更快」。

**M4 实施时变更的决定：**
1. **两份轮询而不是一份。** 原本想把所有 refresh 退到 30 s 一次（被推送替代），但**对端列表（`client/list`）没有推送路径**——谁是同组的是靠问的，不是被告知的。慢 30 s 意味着同组页面要等半分钟才看见对方。把两个刷新拆成两路：peer 仍是 3 s（没有推送成本会随用户数线性增长），文件 30 s（推送的兜底）。一个间隔同时服务两件事，要么是同伴慢半分钟，要么是场景空转。
2. **推送抵达后立即 `refreshFiles()`，不等下一次轮询。** 这是设计目标：通知存在就是为了让页面秒级更新，等下一周期就退化成轮询了。
3. **每个分块字节上限提升到 `CHUNK + 1 MiB` slack**。`read_bounded` 保留「声明的 `Content-Length` 撒谎也照拒」的语义，slack 只覆盖客户端在多块边界处的合理读数差异，挡住的依然是「明显过载的请求」。
4. **`notify_group` 跳过 actor 自己。** actor 的页自己就知道它写了什么；告诉它等于让它刷新两次。推送是「告诉其他同组的人」，不是「告诉所有人」。
5. **空组的 listing 不发 ETag。** 没有可校验的东西，发一个固定 token 会把「空」变成可缓存的 oracle，泄露「之前这里没东西」的差分。空组发完整 body 是最小成本，不值得用 304 换。
6. **存 limits 而不是临时 fetch。** `listingCeiling()` 现在读 `lastLimits`，upload 之前是 304 也不会拿到 0 字节上限。M3 的 ETag 撞到了这里：M1 的 `listFiles()` 返回的 listing 还在被读作「当前上限」，304 后它变成 `null` 之后被解释成 0，每一笔上传都失败。这是一处「304 的语义从 cache 读出后坍缩」的 bug。
7. **executor 调度回归 + 浏览器验收自己挂掉的修复。** M4 的浏览器验证最先定位的问题是客户端列表空了——`POLL_MS = 30_000` 把「3 秒看见同组」打到了「30 秒」。修复：拆双 clock。同一根因下还有一处：push 检查被排在 `tabA.close()` 之后，那一刻文件已被回收，peer 看不到任何东西；把它移到 close 之前。一个 plan item 的位置 + 一个 poll 间隔，是两个不同的设计 bug。

**M4 新增用例（2 Rust + 1 浏览器）：**
- `share`：unchanged listing 的 digest 稳定、同一项过期后 digest 变化
- 集成（`tests/share.rs`）：上传推到对端 socket 且 actor 自身不收到；撤销也推送；跨组 peer 收不到任何推送
- 浏览器：上传后 4 秒内对端文件面板有该项——30 s 轮询跑不完这条


## M5：剪贴板共享 ⬜

**目标：** 同组客户端之间传文本与图片剪贴板内容，显式确认，可一键复制。

**涉及文件：** `crates/conex-host/src/share_http/clip.rs`（新）、`crates/conex-host/src/share.rs`、`crates/conex-host/src/clients.rs`、`web/src/lib/files.ts`、`web/src/components/clip-scene.tsx`（新）、`web/src/hooks/use-hello-page.ts`、`schema/conex/share.proto`。

- [ ] `SharedFile.kind` 区分 `"file"` / `"clip"`，下载、预览、Range、隔离全部复用 M1/M3 链路。
- [ ] `POST /web/files/clip`：文本 body ≤ 64 KiB，`text/html` 一律拒绝；图片走 `ClipboardItem`，转分块上传路径（M2），不新增第二条字节通道。
- [ ] clip 配额独立：默认 20 条 / 1 MiB / TTL 10 分钟，不得挤占文件场景的 32 条名额；到期由 M1 的清扫路径回收。
- [ ] **只显式触发**：页面放「共享剪贴板」按钮（读 `navigator.clipboard.readText()`，需权限）与拖入/粘贴到指定区域；不做全局 `paste` 监听自动广播。
- [ ] 接收方：`conex/clip-added` 推送 → toast 展示摘要 + 「复制」按钮（`writeText`）+ 「下载」（若为二进制）。clip 不经 `localStorage` 持久化。
- [ ] 审计只记元数据（字节数、mime、owner link、时间），绝不记内容。
- [ ] UI 单独一张「剪贴板」卡片，与文件卡分开，避免混排后看不出哪些内容是刚传来的。

## M6：真实浏览器验收 ⬜

**涉及文件：** `scripts/verify-landing-scenes.ts`、`docs/verification/`。

- [ ] 大文件续传：中断网络后 `HEAD` 续传，最终 root 与一次性上传一致。
- [ ] 标签页刷新后续传完成，`sessionStorage` 中无残留 upload。
- [ ] Range 边界用例（`a-b` / `a-` / `-n` / 越界 / `If-Range` 不匹配）与「preview 不全量」的抓包证据。
- [ ] 跨组拿到同一个 id 仍是 404，且与未知 id 不可区分。
- [ ] clip：同组可见 + 跨组不可见 + TTL 到期 + 拒绝 html + 断开即回收。
- [ ] 配额：超 `max_file_bytes` 在上传前被拒，磁盘峰值有界。

## 5. 安全不变量（每条一例）

1. 文件 id / uploadId / rootCid 都不是访问令牌；隔离只由 `group_key` 一次 lookup 决定，跨组与同未知 id 不可区分。
2. 任何字节落盘前必须通过 CID 校验；坏块不写入 receipt、不占配额。
3. inline 是主机决定，不是页面决定；`text/html` / `image/svg+xml` / clip 的 html 一律下载-only。
4. 写操作（上传/提交/取消/撤回/clip）必须 Origin + CSRF；读只需 cookie。
5. 声明的 `Content-Length` 撒谎时，流式中仍按实际上限拒（沿用 `share_http.rs:322`）。
6. 头部注入不可行：文件名与 MIME 沿用 `sanitize_name` / `sanitize_mime` 与 `ascii_name` / `percent_encode`。
7. clip 永不自动广播，永不落盘，只记元数据审计。
8. 磁盘配额在上传前按声明大小预扣，失败回滚；share root 不做 GC，只按租约与断线回收。

## 6. 配置项

```toml
[share]
share_root = "/var/lib/conex/share"   # 独立于 content_root
max_file_bytes = 268435456             # 默认 256 MiB，硬上限 1 GiB
max_group_bytes = 2147483648
max_host_bytes = 10737418240
max_files_per_client = 32
max_clips_per_client = 20
clip_ttl_ms = 600000
max_clip_bytes = 1048576
disk_watermark = 0.9
```

未配置 `[share]` 时保持现状（内存 + 8 MiB），旧部署不停机。

## 7. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| GC 误删在途分块 | 断点续传在 commit 时才发现缺块 | share root 单独实例 + 永不 GC（M1） |
| 与 blob 通道共 root 互相误删 | 数据静默丢失 | 强制 `share_root != content_root`，启动时校验并拒绝 |
| 匿名端点被大文件拖满磁盘 | 宿主不可用 | 上传前按声明大小预扣 + host 总配额 + 水位线 |
| clip 挤占文件配额 | 文件场景不可用 | clip 独立计数与字节预算 |
| `share_http` 单文件继续膨胀 | 后续改动风险上升 | 按 M2 拆子模块，单文件不超过 ~200 行 |
| proto 与手写 JSON 平面双源漂移 | 类型不一致在运行期才暴露 | M0 先冻结，`xtask generate --check` 进门禁 |
| 循环依赖：conex-host 依赖 conex-content | 已有依赖（`serve.rs` 已 open ContentStore） | 无新增，仅多开一个实例 |

## 8. 明确不做

- 服务端缩略图 / 视频转码 / 音频波形：需要引入编解码依赖，属展示增强轮次。
- 复活 M5 的 ZIP/DOCX 内嵌预览引擎（`fflate` / `mammoth` 死依赖）：可在 M6 之后单独评估，先用 `<object>` 与下载兜底。
- 自动剪贴板广播、跨组剪贴板、剪贴板历史无限留存。
- P2P 直传、写回与协作编辑、跨主机文件同步。

## 9. M0+M1 已落地证据

**新增/修改的 Rust 侧：** `schema/conex/share.proto`（新）、`crates/conex-content/src/{store,error}.rs`
（`leaves_of` / `hold_root` / `release_root` / `reclaim_all_committed` / `retract_root`、
commit 时写 `refs/<root>.leaves`）、`crates/conex-host/src/share_blocks.rs`（新）、
`crates/conex-host/src/share.rs`（重写为元数据 + 配额 + 过期）、`share_http.rs`（Range + 限额 +
kind/previewBytes 投影）、`config.rs`（`[share]`）、`serve.rs`（接线 + 启动回收 + 过期清扫）、
`ws_transport.rs`（断线按 root 释放）、`clients.rs`（`owner()`）。

**前端：** `web/src/lib/files.ts`（`kind`/`previewBytes`/`expiresAtMs`/limits、`previewFileUrl`）、
`file-scene.tsx`（预览改 Range、剪贴板样式、去掉页面侧 200 KiB 常量）、`use-hello-page.ts`、
`app.tsx`。

**门禁实跑结果：** `cargo fmt --all --check` 通过；`cargo clippy --workspace --all-targets
-- -D warnings` 通过；`cargo test --workspace` 68 个 suite 全过；`xtask generate --check`
通过；`xtask conformance` p0/p1 通过；`bun run typecheck`、`bun test` 通过；
`scripts/verify-landing-scenes.ts`（真实浏览器，28 项检查含文本预览读到真实字节）全部通过。

**新增用例（17）：** 块存储 4（顺序 leaf、range 重组、retract 释放块与 refs、共享块随最后
一个 root 释放）、share_blocks 4（range 读精确字节、乱序分块拒绝、uploadId 不是凭据、
同内容两个 offer 共用一个 root）、share 元数据 14（跨组不可读、未知 id 与跨组 id 不可
区分、组/单文件/clip 三类配额、clip 过期后与未知 id 同形、断线回收并归还配额、仅所有者
可撤回、active content 不内联且 clip 不放宽白名单、previewBytes 有界、名字注头、owner
计数只描述自己）+ 集成 4（预览只取 head 且带 `Content-Range`/`Content-Length`、四种畸形
Range 答整个 item、share 字节落自己的 root 而非 content root、组上限跟随单文件上限）
+ 配置 3（`[share]` 缺省值、显式 root、禁止与 content_root 相同）。
