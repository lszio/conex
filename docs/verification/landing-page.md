# hello 页面与客户端契约验证记录

> 对应分支 `landing`（提交 `d03cd06`）。本文只记录**实际执行并通过**的结果。
> 前一版内容浏览页面的记录见 git 历史；那一版的部署故障根因另见 §附。

## 1. 页面

三块，全部是真实调用，没有模拟数据：

| 区块 | 行为 |
|---|---|
| 我的设置 | 名称（≤40）、组（≤24）、是否可见；保存即 `client/profile` |
| 在线客户端 | 所有 `visible = true` 的访客，按组→名排序，逐行一个「发送 hello」 |
| 消息 | 本会话发出的 hello（含往返毫秒）与收到的 hello |

构建产物 `web/dist/app.js` 从 709 KB 降到 51 KB——原先的内容浏览模块
（`browser.ts` / `preview.ts` / `archives.ts` / `walkthrough.ts` / `view.ts`）
与其 14 项预览测试一并删除。

## 2. 客户端契约

契约类型在 `schema/conex/dashboard.proto`（L11），由 `cargo xtask generate`
派生 Rust / TypeScript / JSON Schema，无手写副本。

- `ClientProfile { displayName, group, visible }`：自述、未经认证。
  入库前去除控制字符并截断（`clients.rs::clean`）。
- `client/list` 只返回**已挂出站通道**的链接：握手未完成的、崩溃时没走 clean
  close 的死链都不出现，否则会列出一个永远无法应答 hello 的客户端。
- `visible = false` 是真正的隐私开关：既不进任何列表，`client/hello` 对它也
  返回 `unavailable`，不是静默丢弃。
- 名字为空时列表显示 `client-<linkId 前 6 位>`，不伪造身份。

## 3. 两个真实缺陷（由真浏览器发现，非推测）

### 3.1 推送帧绕过出站字节预算 → 目标 socket 被关

**现象**：A 给 B 发 hello 能成功（40 ms，有回复），但 B 的连接随即被关闭；
此后 B 再不能调用任何方法。B→A 方向则直接报 `websocket connection closed`。

**定位**：B 收到的第一帧 `conex/client-hello` 后立即死亡。逐步收窄：
- pong 解析正确（`ws_transport::client_pong` 单测覆盖真实报文）
- 连续三次 A→B 都能成功，但第二次起 `roundTripMs` 恒为 0

**根因**：`ws_transport` 的 writer 任务对每个发出的帧执行
`queued_bytes.fetch_sub(bytes)`。我最初直接 `tx.send(...)` 推送，
**没有先 `fetch_add`**。于是 writer 的 `fetch_sub` 把 `AtomicUsize` 下溢：
调试构建回绕，release 构建回绕成接近 `usize::MAX`，之后每一次
`enqueue_message` 的预算检查都失败 → `break` → socket 关闭。

**修复**：`clients::Outbound` 持有 writer 的同一个计数器，改为
`fetch_add` 后 `try_send`，失败时回滚预留。回归断言由
`clients::tests::hello_round_trip_reports_the_reply` 覆盖（测试里也按真实
writer 的方式 drain 计数器）。

### 3.2 死链残留在客户端列表里

**现象**：列表里出现 `client-xxxxxx` 这样的行，且其 `lastSeen` 是「刚刚」。

**定位**：不是残留，而是**活着**的另一条链接。逐项核对后确认：`client/list`
只返回可寻址链接这一行为本身是对的；真正的缺口是**没有回收**——一个标签页
崩溃后，其条目永久驻留，注册表无界增长。

**修复**：`client/list` 只返回有 writer 的链接（`can_receive`），
`reap_orphans()` 在 60s 宽限后回收无 writer 的条目；`set_profile` 对无 writer
的链接返回 `None`，避免死链伪装成已配置。
回归测试 `clients::tests::writerless_links_are_not_listed_and_are_reaped`。

## 4. 真浏览器验证（两个独立会话）

两个 Playwright 独立 context（两套 cookie = 两个访客）：

```console
A rows: [ 'bob', 'alice' ] | A self: alice
B rows: [ 'bob', 'alice' ] | B self: bob
A→B: → bob：<1 ms，回复「pong」
B got: ← 收到 hello：hello alice
after B hides → A rows: [ 'alice' ]
bob rows in A: 0
page errors: none
```

即：互相可见、互相 greet、隐藏后从对方列表消失、零页面错误。

**关于 `<1 ms`**：Host 侧测得的首个 pong 约 40 ms，其后为 0–1 ms。经交叉验证，
这是 **Bun `WebSocket` 进程内一次性惰性初始化**（换一个目标客户端，它的第一个
pong 同样是 0 ms），不是网络 RTT，也不是每次发送的固定开销。真实浏览器下该
现象不出现。页面在亚毫秒时显示 `<1 ms` 而不是 `0 ms`，避免把有效数字读成零。

## 5. 门禁

```console
$ cargo test --workspace --offline      # 67 个套件全 ok，0 失败（跑两遍一致）
$ cargo clippy --workspace --all-targets -- -D warnings   # 0 findings
$ cargo fmt --all --check               # clean
$ bun test sdk/typescript/tests         # 25 pass / 0 fail（新增 2 例 client/hello）
$ bun run typecheck                     # SDK + web 无错
$ cargo test -p conex-host --lib clients::   # 7/7
$ cargo test -p conex-host --lib client_pong # 2/2
```

`cargo xtask check` 未在本轮运行；其子项（fmt / generate / clippy / 全量测试）
已分别验证通过。

## 6. 未验证项

- **线上 `https://conex.lszio.space` 的 hello 验收**：镜像重建期间本记录收尾，
  §4 的全部实测在本地同一配置（`docker/host.toml`，无端点、无凭据）下完成。
  线上仍需复跑：两页互见、互发 hello、隐藏生效。
- 多访客高并发（>50）下的广播与回收行为。
- 真实移动端浏览器（仅 1280px 视口模拟）。

## 附：上一版内容浏览页面的部署故障根因（2026-10-04 上午）

保留以便对照，三者都与 hello 页面无关但仍在当前代码里：

1. 容器在配置加载期退出：`listen = "0.0.0.0:8787"` 配 `allow_loopback_http`
   被 `config.rs` 拒绝（该开关断言「明文不离开本机」，容器里不成立）。
   → 新增 `allow_plaintext_bind`。
2. Rust 构建阶段失败：Debian 的 `protobuf-compiler` 不含
   `google/protobuf/struct.proto` → 装 `libprotobuf-dev`。
3. 部署永久挂起：基础镜像浮动 tag 的 manifest 拉取卡死 40 分钟且无报错，
   Dokploy 又不允许并发起第二次部署 → 三个基础镜像固定为 digest。
