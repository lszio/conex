# 多主机内容（M0–M5）实施与验证记录

> 对应执行计划：[2026-10-01-conex-multi-host-content.md](../plans/2026-10-01-conex-multi-host-content.md)。
> 本文只记录**实际执行并通过**的结果；每个条目附验证方式。分支 `refactor/arch`，截至 2026-10-03 未提交（工作树含 M0–M5 全部改动）。

## 交付总览

| 里程碑 | 范围 | 状态 |
|---|---|---|
| M0 | 内容契约冻结：endpointId 定位、revision、blob/get 互斥目标、64 位安全范围、元数据规则 | ✅ 完成 |
| M1 | 公共入口安全：`[web_guest]` 访客配置、配额分离、cookie 重发、连接面板访客隔离；blob 所有权与授权 | ✅ 完成 |
| M2 | 逐端点注册生命周期（staged 链接/逐端点 review/accepted 列表）；Agent 复用 provider-fs 机制；全类型目录 | ✅ 完成 |
| M3 | protobuf 二进制链路（DataChunk）；`blob/get` remote 打通；同源 `/content`（Range/ETag/防注入/流式取消） | ✅ 完成 |
| M5 | 公共页面与格式展示：目录导航、文本/图片/视频、DOCX/ZIP 受限预览、二进制元数据下载 | ✅ 完成（38/38 浏览器检查） |
| M4 | Notez 接入 | ⏳ 未开始（硬依赖：需真实 Notez 仓库与实例） |
| M6 | 验收收口（multi-host-content / public-content-security 套件、容量门槛、跨主机 TLS） | ⏳ 未开始（部分基础已在 M2 生命周期场景落地） |

## M0 内容契约（已验证）

- `schema/conex/{source,blob,chunking,rpc}.proto`：`ResourceSummary`（十进制字符串 sizeBytes、EntryKind、revision）、`SourceReadResponse`（text ⊕ content）、`BlobGetRequest`（`committed | remote` oneof，旧字段 reserved）、`BlobAccess.endpointId`（wire 删除 providerId）。
- 严格解码点：`conex-source::contracts`（text ⊕ content 互斥、CID 格式）、`conex-core::contracts::prepare_blob_get`（混合定位/空端点/路径穿越/非十进制/零长度/64 位溢出全部拒绝）。
- 验证：`crates/conex-core/tests/blob_get_contract.rs`（9 例）、`crates/conex-source/tests/read_contract.rs`（8 例）；conformance 向量 Rust/TS 一致；`cargo xtask generate` 后 workspace 全绿。

## M1 访客会话与 blob 所有权（已验证）

- `[web_guest]` 配置节：principal/tenant/max_sessions/idle_ttl_ms/max_issue_per_minute；load 期拒绝与 policy 租户矛盾、token 主体碰撞（`connected_landing_config.rs`）。
- 匿名配额独立于登录 8 会话；滑动空闲过期；每分钟签发限速（`web_auth.rs::guest_sessions_obey_configured_bound_independently`）。
- 过期 guest cookie 静默换发；认证用户死会话返回 401 不降级（墓碑判定，`stale_guest_cookie_reissues_but_authenticated_does_not_downgrade`）。
- `connection/list` 对访客按 `BrokerCall.link_id` 隔离且隐藏 agentLinks（`endpoint_catalog.rs::connection_list_isolates_guest_visitors_and_hides_agent_links`）。
- blob 所有权：`Owner{principal,tenant}` 持久化进 UploadState/CommitRecord/PinRecord；`blob/get` 要求 chunk ∈ owner 可达集（`blocks_by_root`）；resume/chunk/commit/cancel/pin/unpin 全部校验归属；have 不泄露他人内容；1 GiB 入口 + 固定 256 KiB chunk + 24h 租约 + 累计写入上限 + commit 实长求和校验；无主 staging 不认领、过期清理。
- 验证：`crates/conex-host/tests/blob_ownership.rs`（4 例端到端：A 提交后 B 同租户/跨租户持 CID/uploadId/pinId 全拒、同名 put 不串 staging、重启后规则保持、上限生效）。
- 顺带修复既有缺陷：wire 级单块 raw 提交因 manifest→raw 猜测重试永不触发而必败，现从 upload 记录读取 kind。

## M2 注册生命周期与通用 provider（已验证）

- `agent/register` 改为逐端点声明（`endpoints:[{endpointId,root,methods}]`），Host 逐端点 review，响应含 `acceptedEndpointIds` + `rejectedCapabilities`；全部被拒才整体失败。
- 链接 staged：WSS upgrade 只暂存，注册成功才 swap-in 并关闭旧链；失败 discard，健康旧连接不受影响；代次 fence 保持。
- per-endpoint ready：`RemoteConnections::is_ready_endpoint`（链接 active + 端点在当前代次 accepted）驱动 catalog 投影。
- Agent 删除硬编码 FsRoot 分派，改用 provider-fs `ReadHandler/ListHandler/SearchHandler` + `CallContext`/claim；`RequestContext` 新增 principal/tenant 转发（快照游标按主体绑定）。
- 目录覆盖全部常规文件（未知类型 octet-stream 回退、带 revision）；搜索限 md/org/txt，非 UTF-8 静默跳过。
- 受限分发：每请求独立任务 + Semaphore(8) + `timeout_budget_ms` deadline；回复走有界 mpsc。
- 验证：`agent_link.rs`（5 例含独立 review/accepted 投影）、`runtime_tests.rs`（3 例）、`remote_source.rs`（隔离/根越界/代次门禁回归全过）。

## M3 字节传输与 /content（已验证）

- Agent 链协商 `conex-protobuf-wss`；新增 `DataChunk` 消息体（`rpc.proto` body 5）承载原始字节；host↔agent 控制面与回复全部 prost 二进制；JSON 平面 encode/validate 显式拒绝 DataChunk。
- `Broker::content_range_bytes`：先 `source/read` 授权探针（同一 route/contract/tenant/policy 路径），再按 catalog 的 agent 绑定走二进制链；本地 `source-fs` 端点经 `conex_core::RangeReader`（provider-fs 实现）回落。
- Agent `dispatch_chunk`：每次 ≤ `CHUNK_SIZE`(256 KiB)；读前 `stat` 绑定 mtime revision，请求 revision 不符即 `stale_revision`。
- `/content`（`crates/conex-host/src/content_http.rs`）：GET/HEAD，同源 cookie 会话 + `source/read` 授权探针；单范围 Range（`a-b`/`a-`/`-n`）、200/206/416（`bytes */total`）、多范围按 RFC 降级 200；`ETag: "rev-<mtime>"`、If-Range 不匹配降级全量；`nosniff`、inline 白名单（图片/音视频/纯文本）否则 `attachment` + `filename*` 编码（CR/LF/NUL 拒绝）、`private, no-store`；`Body::from_stream` 逐切片边读边发，浏览器停止即取消上游。
- 顺带修复：`pbjson_to_json` 把整数变 f64 导致 prost `uint32` 严格解码拒绝 `limit:100.0`（`endpoint/list` 必败）；现 `normalize_numbers` 重标整值 float。
- 验证：`crates/conex-host/tests/content_http.rs`（7 例：700 KiB 任意二进制逐字节一致、中段/开放式/后缀 range 精确、越界 416、多范围 200、If-Range 降级、HEAD 无体、未认证不泄露、`.bin` 强制 attachment）；三个 e2e 套件（真实 Host + 双 Agent 进程）全过。

## M5 公共页面与格式展示（已验证）

- 新增 `web/src/{preview,archives,browser}.ts`：
  - `preview.ts`：格式分类（MIME 优先/扩展名回退）、预算常量（预览下载 8 MiB / 归档展开 32 MiB / 条目 2000 / 压缩比 200 / 文本 512 KiB）、八类错误映射（权限/缺失/离线/版本变化/超限/不支持/取消/未知）、`stripDangerousMarkup`（script/iframe/handler/javascript:/外链全部中和）。
  - `archives.ts`：mammoth 1.13（BSD-2）DOCX→HTML + fflate（MIT）ZIP 列目录；**只列不展开**，穿越/绝对路径/控制字符/压缩比 >200/超预算条目拒绝并标注。
  - `browser.ts`：主机分组目录树、面包屑、扁平 scan 合成目录前缀（任意 provider 可钻取）、格式感知预览面板、目录刷新不重置播放（media 元素带 `data-resource`）。
- `web/index.html` 新增 browser-panel；`main.ts` 接线 Browser handlers（list/search/contentUrl/fetchRange 均走同一授权路径）。
- 验证：`web/tests/preview.test.ts` 14 例（分类、清洗、穿越/炸弹/条目上限、DOCX 渲染、错误映射）；**Playwright + chromium 真实浏览器 38/38**（1440px 与 390px 双视口：文本/图片解码/视频 controls/SVG 不内联/DOCX 无外链/恶意 DOCX 无脚本/ZIP 穿越拒/二进制无乱码/键盘可达/无页面脚本错误；截图 `/tmp/m5-desktop.png`、`/tmp/m5-mobile.png`）。
- 顺带修复：`SnapshotCache` 同 (principal, endpoint, root) 第 4 次创建即 QuotaExceeded，目录导航死锁；改为同 key 重建时逐出最旧同 key snapshot（分 principal 预算仍生效），`pagination.rs` 测试更新为新契约。

## 已知缺口 / 未验证

- **M4 未开始**：Notez 接入需要真实 Notez 仓库与运行实例（计划禁止以 mock 或导出目录冒充）。
- **M6 套件未注册**：`multi-host-content`、`public-content-security` 两条 e2e 场景尚未在 xtask 注册（部分场景证据已在 M2/M5 中以单测+e2e 形式存在）；1 GiB 远端读取、20 并发访客、跨主机 TLS 验证未执行。
- **fmt/clippy 债已清零**：分支 `refactor/arch` 原有约 60 个历史文件的 rustfmt（style_edition 2024）与 clippy 偏差，`cargo xtask check` 在 M0 之前即红。本 PR 附带 `fix(ci)` 提交完成全仓 `cargo fmt` 与 clippy（`-D warnings`，含 CI 侧 clippy 1.99 的 `ptr_arg`）清理；另修复 1 GiB 测试在慢速 CI 上因 60s staging 租约不足而失败（改用 1h），以及 5 处测试硬编码作者本机二进制路径（改用 `CARGO_BIN_EXE_conex-host`）。
- 1 GiB blob 的 `p1-stream-1gib` 旧能力回归保留，但不替代 M6 的远端 1 GiB 验证。
- 本机 loopback 证据不作为跨主机 TLS 证据（M6 项）。

## 复现命令

```bash
cargo test --workspace                 # 全部单元/集成测试（含本文引用的各测试文件）
bun test sdk/typescript/tests          # SDK 向量与客户端测试
cd web && bun test tests/preview.test.ts   # M5 预览安全 14 例
bun run typecheck && bun run build:web
cargo xtask e2e --suite connected-landing            # 双 Agent + 生命周期（真停 A 进程）
cargo xtask e2e --suite connected-landing-web        # 浏览器会话全链路
cargo xtask e2e --suite connected-landing-connections
```

浏览器验收脚本（M5）：`/tmp/m5verify.ts`（Playwright，`CHROME_BIN` 指向本机 chromium），对 1440×900 与 390×844 两个视口各执行 19 项检查。
