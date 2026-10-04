# 落地页与公开部署验证记录

> 对应分支 `landing`（提交 `a9d3034`、`af47b3d`）。本文只记录**实际执行并通过**的结果，每项附复现命令；未执行项显式列在末尾。

## 1. 部署故障的真实根因

部署到 Dokploy 连续 5 次 `error`，有两个互相独立的阻塞原因，都在干净环境复现：

**(1) 容器在配置加载期就退出。** `docker/host.toml` 写的是
`listen = "0.0.0.0:8787"` + `allow_loopback_http = true`，而
`crates/conex-host/src/config.rs` 要求 `allow_loopback_http` 必须配 loopback 地址。
实测：

```console
$ ./target/debug/conex-host /tmp/t.toml      # listen=0.0.0.0:8787, allow_loopback_http=true
config error: allow_loopback_http requires a loopback listen address
```

修复：新增 `allow_plaintext_bind`，语义是「明文监听非 loopback（反代后的容器属正常）」，
与 `allow_loopback_http`（明文绝不离开本机）分开。回归测试
`tls_server.rs::plaintext_bind_behind_proxy_needs_its_own_flag` 断言
`allow_loopback_http` 单独**不能**打开非 loopback 明文监听。

**(2) Rust 构建阶段缺 well-known protos。** `schema/conex/common.proto` 导入
`google/protobuf/struct.proto`，而 Debian 的 `protobuf-compiler` 不带这些文件。
实测 `protoc failed: google/protobuf/struct.proto: File not found`，
连带 `conex/control.proto` 报 `Plane/Limits is not defined`。修复：Dockerfile 增装
`libprotobuf-dev`。同时把基础镜像从 `rust:1.85` 改为 `rust:1`，与
`rust-toolchain.toml` 的 `channel = "stable"` 对齐。

## 2. 静态资源与 llms.txt（已验证）

```console
$ curl -sSI http://127.0.0.1:18787/
content-type: text/html; charset=utf-8
x-content-type-options: nosniff
x-frame-options: DENY
content-security-policy: default-src 'self'; script-src 'self'; ... object-src 'none'; frame-ancestors 'none'
cache-control: no-store
referrer-policy: no-referrer

$ curl -sS -o /dev/null -w '%{http_code} %{content_type}\n' .../llms.txt   # 200 text/plain; charset=utf-8
$ curl -sS -o /dev/null -w '%{http_code}\n' .../llm.txt                    # 200（别名）
$ curl -sS -o /dev/null -w '%{http_code}\n' .../not-found                  # 404（不回退 HTML）
```

`web.rs` 回归测试 4/4 通过（`llms_txt_is_served_under_both_spellings_when_present`、
`missing_llms_txt_does_not_break_the_page`）；缺 `llms.txt` 的构建仍然服务页面，
该路径 404 而非返回 HTML。`bun run build:web` 在缺 `llms.txt` 时直接失败。

## 3. 免登录访客会话（已验证）

```console
$ curl -sSi http://127.0.0.1:18787/web/session
HTTP/1.1 200 OK
set-cookie: conex_web_session=…; Path=/; HttpOnly; SameSite=Strict; Secure
{"csrf":"…","principalId":"guest","role":"ui","tenantId":"demo","expiresAtMs":"…"}
```

真实浏览器（headless Chromium，1440px）确认登录面板不出现、控制台直接可用：
`host status: 已就绪`、`login panel visible: false`、`app panel visible: true`。

## 4. 真实 SDK 全链路（已验证）

用仓库自带 SDK（`web/` 工作区，与页面同一条代码路径）驱动运行中的 Host：

```console
$ bun web/tmp-guest-check.ts        # 一次性脚本，已删除
principal -> guest
endpoint/list -> archives:CONNECTION_STATE_NOT_APPLICABLE, docs:…, media:…
source/list   -> architecture.md, readme.md
source/search -> （命中）
source/read   -> text length 535
connection/list -> links: 1 agentLinks: 0
traversal -> refused: invalid resourceId
handshake: authenticating | ticket:requesting ticket | connecting | negotiating | ready:sent
```

握手顺序与契约一致：`conex/hello` → `conex/ready` → 业务帧。访客只看到自己的
一条 UI link，`agentLinks` 为空。

## 5. 授权拒绝路径（已验证）

| 请求 | 结果 |
|---|---|
| `resourceId=../../docker/host.toml` | 400 `invalid resourceId` |
| `endpointId=nope` | 400 |
| 无 cookie 读 `/content` | 401 |
| 二进制 `random.bin` | `content-disposition: attachment` + `nosniff` |

## 6. 内容与 Range（已验证）

```console
# 字节一致性：256 KiB 随机二进制
$ curl ... /content?endpointId=archives&resourceId=random.bin
39d6a7d09282ff7ef20aa21df0d63f46c5ece78b82c75ca584cdbc88824d474f  got.bin
39d6a7d09282ff7ef20aa21df0d63f46c5ece78b82c75ca584cdbc88824d474f  docker/demo/archives/random.bin
BYTE-IDENTICAL

# 中段 Range
HTTP 206 size=1000 → 与源文件 bytes 1000-1999 逐字节一致
```

## 7. 发现并修复的第二个产品缺陷：媒体无法拖动

**现象**：`clip.webm` 能播放（`readyState 4`、时长 6s、640px），但
`currentTime = 4` 之后被钳回 0。

**定位**：同一文件经 `file://` 加载时 seek 正常到 4.0s，经 `/content` 则为 0，
因此不是素材问题。响应头显示 `accept-ranges: bytes` 但
`transfer-encoding: chunked`、**没有 `content-length`**。

**根因**：`content_http.rs` 算出了 `length` 后用 `let _ = length;` 显式丢弃，
声称「framing is implicit in the streamed body」。媒体元素需要已知总长度才能
把响应视为可 seek。

**修复**：输出 `Content-Length`（全量为总长，206 为分片长），流式与上游取消行为不变。
新增断言于 `content_http.rs` 既有测试：全量 `content-length == payload.len()`、
206 `content-length == 100`（不能是全量，否则浏览器会以为文件只有 100 字节）。

**修复后实测**：

```console
$ curl -sSI .../content?endpointId=media&resourceId=clip.webm
accept-ranges: bytes
content-length: 196343

# 真实浏览器 seek
seek: {"ok":true,"t":4}
```

`content_http` 测试 7/7 通过。

## 8. 浏览器检查（1440px 与 390px）

驱动真实 headless Chromium（仓库锁定的 playwright 1.55）：

| 检查 | 结果 |
|---|---|
| 标题 / h1 / CTA / 价值卡 | 有（曾丢失 `<title>`，已补回并复验） |
| 请求链路演示 7 步 | 渲染并按真实方法名推进 |
| 演示结束后状态 | `active: 0, done: 7`（修复前 7 个全 `active`） |
| 重新播放 | 点击后回到第 1 步 |
| `prefers-reduced-motion: reduce` | 直接呈现完成态、无动画（修复前仍播放） |
| 访客连接 | 无凭据即 `已就绪` |
| 端点目录 / 拓扑节点 | 3 / 3 |
| Markdown 预览 | 渲染正文 |
| 图片预览 | `naturalWidth > 0` |
| 视频播放 | `readyState 4`，6s，640×360 |
| 视频拖动 | seek 到 4.0s（见 §7） |
| ZIP 预览 | 列出 4 个条目，不展开 |
| DOCX 预览 | 有文本，`<script>`/`<iframe>` 计数 0 |
| 二进制元数据 + 下载链接 | 存在且带 revision |
| 390px 横向溢出 | 无（`scrollWidth == innerWidth == 390`） |
| 页面 JS 错误 | 0 |

截图：`/tmp/landing-1440.png`、`/tmp/landing-mobile.png`。

**headless Chromium 无专有编解码器**：`canPlayType('video/mp4; codecs="avc1.42E01E"')`
返回空串，故 `clip.mp4` 在该环境下报 `DEMUXER_ERROR_NO_SUPPORTED_STREAMS`。
这是测试环境限制而非页面缺陷（Safari / 桌面 Chrome 正常）。因此另提供
`clip.webm`（VP9，开源编解码器）作为可验证播放与拖动的素材。

## 9. 门禁

```console
$ cargo test --workspace --offline     # 67 个测试套件全 ok，0 失败
$ bun test sdk/typescript/tests         # 23 pass / 0 fail
$ bun test web/tests/preview.test.ts    # 14 pass / 0 fail
$ bun run typecheck                     # SDK + web 均无错
$ cargo xtask e2e --suite connected-landing             # A/B 隔离 + 真停 A 进程生命周期
$ cargo xtask e2e --suite connected-landing-web         # login/session/list+read
$ cargo xtask e2e --suite connected-landing-connections # connection/list stale + 计数
$ cargo xtask check                                      # all steps passed（fmt/generate/conformance/additivity/e2e）
$ docker build -t conex:final-pin .                      # 成功（生产镜像）
```

`cargo fmt --all --check` 与 `cargo clippy --workspace --all-targets -- -D warnings`
亦单独跑过：fmt 干净、clippy 0 findings。

## 10. Dokploy 部署（已上线）

应用 `fEKm9C8gzoKBXN94LAFq4`（App/production）原本跟踪已删除的分支
`refactor/arch`，每次自动部署必然失败。已改指 `landing` 分支；域名
`conex.lszio.space`（Let's Encrypt → 反代 → 容器 8787）已创建。

**第三次阻塞：基础镜像 tag 拉取卡死。** 首次部署在
`#2 [internal] load metadata for docker.io/library/rust:1-bookworm` 停了约 40 分钟，
日志字节数冻结在 19577 不再增长，且没有任何报错；Dokploy 在旧部署仍为
`running` 时拒绝启动新部署，因此站点一直 502。根因是浮动 tag 必须在构建期
经 registry 解析 manifest，拉取卡住就整体挂起。

修复：三个基础镜像全部按 digest 固定。重新部署后该行耗时
`#2 DONE 0.3s`（原为无限等待），构建正常完成。

## 11. 线上验收（https://conex.lszio.space）

```console
$ curl -sS -o /dev/null -w '%{http_code}\n' $O/          # 200 text/html; charset=utf-8
$ curl -sS -o /dev/null -w '%{http_code}\n' $O/llms.txt  # 200 text/plain; charset=utf-8
$ curl -sS -o /dev/null -w '%{http_code}\n' $O/llm.txt   # 200（别名）
$ curl -sS -o /dev/null -w '%{http_code}\n' $O/not-found # 404（不回退 HTML）

$ curl -sSi $O/ | grep -iE 'content-security|nosniff|x-frame|cache-control|referrer'
content-security-policy: default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self' ws: wss:; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'
x-content-type-options: nosniff
x-frame-options: DENY
cache-control: no-store
referrer-policy: no-referrer

$ curl -sSi $O/web/session | grep -iE '^HTTP|set-cookie'
HTTP/2 200
set-cookie: conex_web_session=…; Path=/; HttpOnly; SameSite=Strict; Secure
{"csrf":"…","principalId":"guest","role":"ui","tenantId":"demo","expiresAtMs":"…"}

$ curl -sS -o /dev/null -w '%{http_code}\n' "$O/content?endpointId=docs&resourceId=readme.md"   # 401（无会话不泄露）
```

真实 HTTPS 下 cookie 带 `Secure`，与 `allow_plaintext_bind`（不影响 cookie 策略）一致。

**生产镜像作为容器运行**（`docker run conex:final-pin`，健康检查 `healthy`）：
四个路由全部正确、访客会话自动签发；把 `web_origin` 覆写为本地地址后，
容器内页面通过全部预览检查（Markdown、图片、ZIP 4 条目、DOCX 0 脚本、二进制元数据、
0 页面错误），WebM 拖动到 4.0s 正常。

注意：直接用 `http://127.0.0.1:<port>` 访问容器**会失败**，因为镜像内
`web_origin = "https://conex.lszio.space"`，Origin 校验按设计拒绝。这不是缺陷，
本地验证需要覆写 `web_origin`。

**真实浏览器对线上站**（headless Chromium，1440px）：

```console
markdownText: "# conex 落地页演示内容…"
imageOk: true      zipEntries: 4      docxHasText: true      docxScripts: 0
binaryMeta: true   page errors: none
video src: https://conex.lszio.space/content?endpointId=media&resourceId=clip.mp4&revision=…
seek: {"ok":true,"t":4}
walkthrough: 结束态 active 0 / done 7；点击「重新播放」回到第 1 步
prefers-reduced-motion: done 7 / active 0（无动画）
```

## 未验证项

- 跨主机真实反连、真实证书 pin、真实 OIDC：沿用既有缺口，本轮未涉及。
- 移动端真机（仅 390px 视口模拟，无触屏设备实测）。
- `clip.mp4` 未在有专有编解码器的浏览器上实测（headless 环境无 H.264）；
  同一容器的 `clip.webm` 播放与拖动已实测通过。
- 多访客并发、大文件（1 GiB）线上读取：未执行。
