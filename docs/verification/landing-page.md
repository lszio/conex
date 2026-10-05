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
pong 同样是 0 ms），不是网络 RTT，也不是每次发送的固定开销。线上真实浏览器
实测为 19–20 ms（§5），该现象不出现。页面在亚毫秒时显示 `<1 ms` 而不是
`0 ms`，避免把有效数字读成零。

## 5. 线上验收（https://conex.lszio.space）

两个独立浏览器 context（两套 cookie = 两个访客），无任何凭据：

```console
both connected (no credential)
A sees: [ 'bob', 'alice' ]
B sees: [ 'bob', 'alice' ]
A→B: → bob：20 ms，回复「pong」
B got: ← 收到 hello：hello alice
B→A: → alice：19 ms，回复「pong」
A sees after B hides: [ 'alice' ]
page errors: none
```

即：互相可见、互相 greet、隐藏后从对方列表消失、零页面错误。
线上实测往返 **19–20 ms**；本地 Bun 下出现的 0–1 ms 确认为其一次性惰性初始化，
真实浏览器不出现（见 §4）。

## 6. 门禁

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

## 6. 性能（实测，非估计）

`cargo xtask bench --suite hello` 启动真实 Host、用真实 SDK 客户端跑，结果写入
`web/src/perf-data.json` 供性能页渲染。数据为本机 loopback，不代表公网延迟。

| 场景 | p50 | p95 | p99 | max |
|---|---|---|---|---|
| hello 单客户端给自己 | <1 ms | 0.4 ms | 0.5 ms | 0.5 ms |
| hello（8 客户端在线） | 0.75 ms | **41.4 ms** | 42.2 ms | 42.5 ms |
| hello（64 客户端在线） | 0.7 ms | **41.4 ms** | 42.1 ms | 42.5 ms |
| client/list（8 客户端） | 0.45 ms | 0.61 ms | 0.74 ms | 0.83 ms |
| client/list（64 客户端） | 2.18 ms | 2.81 ms | 3.17 ms | 3.5 ms |

**两个诚实结论：**

1. **注册表规模不影响 hello 延迟。** 8 个与 64 个客户端在线时分布几乎相同，说明
   链路成本与在线人数无关；`client/list` 从 0.45 ms 涨到 2.18 ms（64 vs 8），
   这才是随人数增长的部分，也是页面把轮询间隔设为 3s 的原因。

2. **p95 的 41 ms 是真的，本轮未定位。** 逐次打印采样位置后发现：慢样本**永远落在
   每轮的第二个发送者**（`position=1`），第一个永远正常；换目标客户端的第一次
   也正常，warmup 也消不掉。规模不变、位置固定、值几乎恒为 41 ms，与 Linux
   delayed-ACK 量级吻合，但未做抓包确认，因此性能页照实展示该 p95，不做粉饰。

## 6b. 线上验收（shadcn 版本）

```console
both connected, no credential
tabs: 客户端 | 介绍 | 性能
A sees: [ 'alice', 'alice-2' ]      # 两人同名 → Host 加后缀
B sees: [ 'alice', 'alice-2' ]
A name after reload: alice          # localStorage 缓存生效
perf rows: 5 | first p50: <1 ms     # 性能页渲染 bench 数据
intro headings: 这是什么 | 浏览器只连一处 | 名字只是显示 | hello 是真往返
after hide, A sees: [ 'alice' ]    # 只剩自己
errors: none
```

## 7. 未验证项

- 多访客高并发（>50）下的广播与回收行为。
- 真实移动端浏览器（仅桌面视口模拟）。
- 跨主机反连、真实证书 pin、真实 OIDC：沿用既有缺口，本轮未涉及。

## 附：上一版内容浏览页面的部署故障根因（2026-10-04 上午）

保留以便对照，三者都与 hello 页面无关但仍在当前代码里：

1. 容器在配置加载期退出：`listen = "0.0.0.0:8787"` 配 `allow_loopback_http`
   被 `config.rs` 拒绝（该开关断言「明文不离开本机」，容器里不成立）。
   → 新增 `allow_plaintext_bind`。
2. Rust 构建阶段失败：Debian 的 `protobuf-compiler` 不含
   `google/protobuf/struct.proto` → 装 `libprotobuf-dev`。
3. 部署永久挂起：基础镜像浮动 tag 的 manifest 拉取卡死 40 分钟且无报错，
   Dokploy 又不允许并发起第二次部署 → 三个基础镜像固定为 digest。

---

# L12：项目定位、分组隔离与文件共享场景（2026-10-05）

契约见 [connected-landing §10](../contracts/connected-landing.md)。本文只记录
**实际执行并通过**的结果。三个真实浏览器上下文（A、B、跨组访客）跑在同一个真实
`conex-host` 上。

## 1. 真实浏览器验收（23 项全过）

脚本 `scripts/verify-landing-scenes.ts`，三个独立 `BrowserContext`（各自独立会话
cookie），`bun web/dev.ts` 起的 guest-only Host：

```console
$ CONEX_VERIFY_ORIGIN=http://127.0.0.1:8080 bun scripts/verify-landing-scenes.ts
ok   页面定位是 conex 本身
ok   介绍页说明 conex 的定位与连接方式
ok   另一个组看不到本组客户端
ok   A 的消息泡泡带实测往返延迟: ["我 → 2:58:07 AM\n\nhello\n\n<1 ms"]
ok   B 收到 A 的 hello 泡泡: ["alice → 2:58:07 AM\n\nhello"]
ok   状态页显示客户端计数
ok   状态页显示分组计数
ok   team-a 显示 2 个客户端
ok   team-a 显示实测延迟
ok   本组完成过 hello 后不再显示「尚未测量」
ok   未发过 hello 的组显示「尚未测量」
ok   A 上传后收到成功提示
ok   B 在文件场景看到 A 提供的文件
ok   文件卡片标出所有者而不是当前用户
ok   文件按提供者分卡片显示所有者
ok   B 看不到撤回按钮（非所有者）
ok   文本文件提供预览入口
ok   预览读到真实字节: 第一行：这是共享的文本。
ok   下载链接指向同源共享路由
ok   同组客户端下载到的字节与上传一致
ok   另一个组的文件场景看不到该文件
ok   提供者断开后文件被回收
ok   页面无控制台错误
全部检查通过
```

关键读数：`team-a` 组「2 客户端 / 最近 <1 ms / 均值 <1 ms」，`team-b` 组
「1 客户端 / 最近 尚未测量 / 均值 尚未测量」——未测量的组显示「尚未测量」而不是
0 ms。跨组访客在 `client/list` 与文件场景都看不到 `team-a` 的任何内容。

## 2. 三个由真浏览器暴露的真实缺陷

### 2.1 文本共享文件预览为乱码

**现象**：UTF-8 中文 `.txt` 共享后，预览显示 `ç¬¬ä¸€è¡Œï¼š...`。

**根因**：下载响应的 `Content-Type` 只有 `text/plain`，没有 charset。浏览器按默认
编码解码 UTF-8 字节流，得到 mojibake。

**修法**：Host 在 `share_http.rs` 对 `text/*` 与 `application/json` 补
`; charset=utf-8`；页面预览改为 `arrayBuffer()` + 显式 `TextDecoder("utf-8")`，
不再依赖 fetch 的二次解码。修后预览实测为「第一行：这是共享的文本。」。

### 2.2 同组客户端看不到对方新共享的文件

**现象**：A 上传后，B 的文件场景一直显示「这个组还没有人共享文件」。

**根因**：轮询只调 `refresh()`（`client/list`），文件列表只在连接建立与本端写入后
刷新。更关键的是 `if (document.hidden) return;` 这一行——A、B 两个标签页中只有一个
可见，**后台标签页的整个轮询被跳过**，所以后台的 B 永远拿不到新文件。

**修法**：`client/list` 仍对隐藏标签页省工，但文件列表无条件轮询。修后 B 的
`GET /web/files` 每 3s 返回含 `report.txt` 的列表。

### 2.3 共享文件撤回先删后校验归属

**现象**：`ShareStore::remove` 先 `files.remove(id)` 再比对 `owner_link_id`，
非所有者的撤回请求虽然返回 `Forbidden`，文件却已经被删掉——任何组内成员都能销毁
别人的上传。

**修法**：先 `get` 比对归属，通过后才 `remove`。`only_the_owner_may_withdraw_a_file`
断言拒绝后文件仍在（实测通过）。

## 3. 自动化测试

```console
$ cargo test -p conex-host --lib                       # 38 passed
$ cargo test -p conex-host --test share                # 8 passed
$ cargo test --workspace                               # 全部 ok
$ bun test sdk/typescript/tests                        # 25 pass / 2 skip / 0 fail
$ bun run typecheck                                    # 通过
$ cargo xtask e2e --suite connected-landing            # passed
$ cargo xtask e2e --suite connected-landing-web        # passed
$ cargo xtask e2e --suite connected-landing-connections # passed
```

`tests/share.rs` 用真实 router + 真实 WSS 握手（guest 会话、ticket、hello/ready）
覆盖 8 项：同组列举/预览/下载字节一致、跨组 404、跨组 hello `Forbidden` 且同组
hello 往返成功并写入延迟、只有所有者可撤回、无 CSRF 拒绝写入、活动内容只下载、
断开即回收、超限上传被传输层拒绝。

`clients.rs` / `share.rs` 单元测试覆盖：组隔离列举、改名即换 key、同标签同 key、
空组是独立 key、状态只在真测量后报延迟、跨组配额、单文件上限、单客户端文件数、
断开释放配额、所有权校验、HTML/SVG 不内联、文件名与 MIME 的头注入清洗、空名回退。

## 4. 未验证项

- 移动端视口（仅桌面 1280 px 验收）。
- 目录选择器（`webkitdirectory`）未在自动化里驱动——脚本用 `DataTransfer` 构造
  单文件，目录整包上传未实测。
- 共享文件不跨重启存活（设计如此，内存实现）。
- 跨主机真实 TLS 与真实 OIDC：沿用既有缺口，本轮未涉及。
## 5. 一个 cookie 挤成两个标签页：链接按 session 铸造的根因修复（2026-10-05）

**现象**：同一浏览器开两个标签页，Host 只认出一个客户端——两页显示同一条链接、同一个
名字，其中一页改名另一页的列表就跳。文件归属同理：两个标签页的文件混在同一个 owner 下。

**根因**：链接在 `WebAuth::login_guest` / `login` 里铸造，也就是**签发 session 时**。cookie
是浏览器级的，所以同一浏览器的所有标签页共享一条链接、一个 `ClientEntry`、一个显示名、
一个文件归属。注册表 `register` 还会按 linkId 覆盖，后来的 socket 直接顶掉前一个的
出站通道。

**修法**：链接改在 **`POST /tickets`** 铸造（`tickets.rs`），ticket 携带 `linkId`，
WSS 升级校验该链接属于本会话（`ws_transport.rs`）后才注册客户端。会话持有自己拥有的链接
集合（`WebSession::links`，上限 32），`claim_link` / `owns_link` 是唯一的判定点；同源
HTTP 路由在 `x-conex-link-id` 上收调用方链接，下载因为是普通 `<a href>` 额外允许
`?linkId=`，同样比对会话集合。

**改这一处的连带影响**（都是同一个根因的下游，不是顺手改的）：

- `WebSession` 去掉 `link_id` 字段，改为链接集合；`revoke` 回收集合里所有链接
- `/web/files*` 四条路由的调用方解析从 `session.link_id` 改为请求头里的链接
- `killing_wss_keeps_link_listed_but_freezes_last_seen` 原先断言「一个 session 一条链接」，
  改为断言「死链仍在列表里且 `lastSeenAtMs` 冻结」+「新链接与之并存」

**实测**（真浏览器，真实 host，`scripts/verify-landing-scenes.ts`）：

```
ok   标签页 A 拿到自己的短链: Ekp_W8aS
ok   同一浏览器的两个标签页是不同的链接: Ekp_W8aS vs jLTKO9k8
ok   同组三个客户端都在 A 的列表里: true/true
ok   另一个组的客户端不出现在 A 的列表里
ok   A 收到角落提示而不是对话气泡: "bob 向你打招呼\n\nhello"
ok   发送方看到实测往返: "已发送 hello 给 alice\n\nalice 已确认，<1 ms"
ok   提问在接收方变成待确认提示: "bob 提问\n\n请回答这个问题\n\n你的状态\n确认并回复"
ok   待确认提示里有确认按钮
ok   回答回到发送方: "alice 的回答\n\n你的状态：在线"
ok   同一浏览器的另一个标签页也能看到该文件
ok   另一个标签页不能撤回他人文件（所有者按链接区分）
全部检查通过（28 项）
```

**本轮顺带做的界面改动**（同一批真实验收覆盖）：

- 页面从「五个标签」改为**工作台**：顶部状态栏常驻本标签页名字、短链接、分组 key、
  同组在线数、全站在线数；面板只切内容
- 消息泡泡组件删除，问候改为右上角 toast（`ToastStack`），不再推动页面布局
- `client/hello` 新增 `payload` / `answer`：带参数即提问，目标的 SDK 不再自动确认，
  回答只能由页面的「确认并回复」按钮给出。Host 侧 payload ≤ 1 KiB、answer ≤ 64 节点 / 4 层
- Rust 单测：`a_question_pushes_ask_and_waits_for_the_targets_own_answer`、
  `a_bare_greeting_carries_no_ask_and_no_payload`、
  `an_oversized_payload_is_refused_to_its_own_sender`、
  `an_answer_deeper_or_wider_than_the_cap_is_dropped_not_walked`

**门禁**：`cargo fmt --all`、`cargo clippy --workspace --all-targets`（零警告）、
`cargo test --workspace`、`bun test sdk/typescript/tests`（25 pass）、`bun run typecheck`、
`bun run build:web`、`cargo xtask generate --check` 全部通过。
>>>>>>> 7c573df (feat(web): a workbench where each tab is one client)
