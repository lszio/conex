# connected-landing 验证记录

> 状态：**本机 loopback 三进程与两套 e2e 已通过**（2026-09-21）。所有命令均使用项目内已有套件；不引入新 runtime / 新依赖 / 新业务面。
>
> 上游：[落地页运行手册](../runbooks/connected-landing.md)、[落地页计划](../../plans/2026-09-20-conex-connected-landing.md)、[Connected landing 契约](../../contracts/connected-landing.md)。
>
> 运行人：李书志 / 工作目录 `/home/lszio/Projects/conex` / HEAD 见 git log。

## 0. 命令清单（运行顺序）

```bash
cargo build -p xtask --locked                                            # §3
cargo xtask e2e --suite connected-landing                                # §1
cargo xtask e2e --suite connected-landing-web                            # §2
cargo test -p conex-host --offline --test connected_landing_config       # §4
cargo test -p conex-host --offline --test web_auth                        # §5
bun test sdk/typescript/tests/                                            # §6
bun run typecheck                                                         # §7
```

下面每节直接贴出实际运行命令与输出（去噪：移除 cargo / rustc 进度条；保留测试名 / `test result` / `PASS` / `error` 等关键结论行）。

## 1. `cargo xtask e2e --suite connected-landing`

```text
$ cargo xtask e2e --suite connected-landing
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/xtask e2e --suite connected-landing`
agent link closed: connect websocket: IO error: Connection refused (os error 111)
agent link closed: connect websocket: IO error: Connection refused (os error 111)

connected-landing: A/B list/read/search passed
connected-landing: real Host + two Agent processes, routing and A/B isolation passed
agent link closed: receive websocket frame: WebSocket protocol error: Connection reset without closing handshake
```

PASS 关键行：

- `connected-landing: A/B list/read/search passed` —— `source/list`、`source/read`、`source/search` 在两个 Agent 各被命中且路由正确。
- `connected-landing: real Host + two Agent processes, routing and A/B isolation passed` —— 关闭 Agent-A 后对 `notes-b` 的 `source/read` 仍返回 `"connected landing agent B\n"`。

`agent link closed: … Connection refused` 是 Agent 在 Host 启动窗口内的预期重连日志（backoff 1/2/4 s），不是失败。

## 2. `cargo xtask e2e --suite connected-landing-web`

```text
$ cargo xtask e2e --suite connected-landing-web
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/xtask e2e --suite connected-landing-web`
agent link closed: connect websocket: IO error: Connection refused (os error 111)
agent link closed: connect websocket: IO error: Connection refused (os error 111)

connected-landing-web: web session lifecycle passed
connected-landing-web: real Host + two Agent processes, login/session/list+read via HTTP and WSS passed
agent link closed: receive websocket frame: WebSocket protocol error: Connection reset without closing handshake
```

PASS 关键行：

- `connected-landing-web: web session lifecycle passed` —— bun 脚本打印的真实 PASS，覆盖：
  1. `POST /web/login`（`Authorization: Bearer <ui-token>` + Origin）→ 200 + `Set-Cookie: conex_web_session=…`。
  2. `GET /web/session`（cookie）→ 200，role=`ui`，csrf 为非空字符串。
  3. `ConexClient`（HTTP /rpc，service token）`source/read("notes-a", "team/shared.md")` → `"connected landing agent A\n"`。
  4. `POST /tickets`（cookie + `x-csrf-token`）→ 201 + ticket 字符串。
  5. `/wss?ticket=…` 升级 + hello + ready（自定义 `WebSocket` 注入 Origin）。
  6. `ConexWsClient.listEndpoints({})` → `endpointId` 排序后等于 `["notes-a","notes-b"]`。
  7. `ConexWsClient.read("notes-b", "team/shared.md")` → `"connected landing agent B\n"`。

`WebSocket protocol error: Connection reset without closing handshake` 来自关闭顺序：脚本先 `wsClient.close()`，再让进程退出，因此是良性的 socket reset，不是断言失败。

## 2b. `cargo xtask e2e --suite connected-landing-connections` (L10)

```text
$ cargo xtask e2e --suite connected-landing-connections
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/xtask e2e --suite connected-landing-connections`
agent link closed: connect websocket: IO error: Connection refused (os error 111)
agent link closed: connect websocket: IO error: Connection refused (os error 111)

connected-landing-connections: WSS connection panel passed
connected-landing-connections: real Host + two Agent processes, connection/list stale + dispatch counters passed
agent link closed: receive websocket frame: WebSocket protocol error: Connection reset without closing handshake
```

PASS 关键行：

- `connected-landing-connections: WSS connection panel passed` —— bun 脚本打印的真实 PASS，覆盖：
  1. `POST /web/login` + `/web/session` + `/tickets`（同 `connected-landing-web`）。
  2. `/wss?ticket=…` 升级 + hello + ready。
  3. `ConexWsClient.listConnections({})` → `browserLinks.length === 1`、`principalId === "demo-ui"`、`tenantId === "demo"`、`ticketsIssued ≥ 1`。
  4. 关闭第一个 WSS，再次 `/tickets` + `/wss`；`listConnections` 仍返回 1 行，且 `lastSeenAtMs` 不小于先前读到的值（stale 检测）。
  5. `ConexWsClient.read("notes-b", "team/shared.md")` → 文本校验通过；再次 `listConnections`：`callsTotal` 增长，`callsInFlight === 0`。

## 3. `cargo build -p xtask --locked`

```text
$ cargo build -p xtask --locked
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s
```

0 warning、0 error。本轮把：

- `xtask/src/e2e.rs:193`：`let mut processes = …` → `let _processes = …`（消除 `unused_mut` + `unused_variables`）。
- `xtask/src/landing_demo.rs:38-43`：删除 `impl DemoProcesses { pub fn stop_agent(...) }` —— 全仓 grep 无任何调用点，整段属于 dead code。

新的 `run_connected_landing_web` 也无 warning（已自查）。

## 4. `cargo test -p conex-host --offline --test connected_landing_config`

```text
running 8 tests
test connected_landing_config_is_additive_and_validates ... ok
test duplicate_agent_id_is_rejected ... ok
test invalid_token_role_is_rejected ... ok
test policy_must_reference_a_known_endpoint ... ok
test preauthorized_agent_requires_credentials_and_matching_agent_token ... ok
test remote_endpoint_requires_supported_method_and_authorized_agent ... ok
test remote_endpoint_requires_agent_and_root ... ok
test web_origin_must_be_a_bare_origin_without_userinfo_path_query_or_slash ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## 6. `bun test sdk/typescript/tests/`

```text
bun test v1.4.0 (34cbb9a40)

 22 pass
 2 skip
 0 fail
 171 expect() calls
Ran 24 tests across 8 files. [319.00ms]
```

被 skip 的 2 个是 `client.test.ts` 与 `e2e.test.ts` 中由 `CONEX_E2E_URL/TOKEN` 环境门控的用例；本地常规 `bun test` 不设这两个 env，因此跳过（这是设计而非缺陷）。

## 7. `bun run typecheck`

```text
$ bun run typecheck
$ cd sdk/typescript && bun x tsc --noEmit && cd ../../web && bun x tsc --noEmit
exit: 0
```

无输出 = 通过；退出码 0。

## 8. 落地的修改文件

| 文件 | 变更 |
|---|---|
| `xtask/src/e2e.rs` | 增加 `run_connected_landing_web`（suite 名 `connected-landing-web`）与原 `run_connected_landing` 的 `_processes` warning fix；新增 suite dispatch |
| `xtask/src/landing_demo.rs` | 删除未被调用的 `DemoProcesses::stop_agent`（dead_code warning） |
| `docs/runbooks/connected-landing.md` | 新增：范围、启动、token 三元组、示例配置、docker-compose、日志、门禁、验证清单 |
| `docs/verification/connected-landing.md` | 本文件 |
| `docs/README.md` | 在「文档地图」表格加入 connected-landing runbook + verification |

`deployment/{host,agent-a,agent-b}.toml.example` 与 `deployment/docker-compose.yml` 未改。

## 未验证项

> 以下条目在本轮交付中**未验证**。它们需要真实跨主机 / 真实证书 / 真实浏览器接入环境，本机 loopback 没有可代替的证据。请勿在提交或上线材料中将这些条目写成「已通过」。

1. **跨主机真实网络验收**（计划 §L09）：在不同主机上分别部署 Host 与至少一个 Agent，记录实际网络位置、TLS、Origin、原生浏览器接入与远端读取结果。本轮仅 127.0.0.1 loopback。
2. **真实证书 + 错误 server name / 不可信 CA 的拒绝路径**：本轮 `allow_loopback_http = true` + `allow_loopback_ws = true`，没有加载 `ca_pem` / `expected_server_name`。需要一张自签证书 + 两台主机。
3. **落地页视觉验收（1440px / 390px + 减少动画）**：本轮仅用 SDK 路径覆盖功能等价物；headless 截图与手动视口走查未做。
4. **WSS protobuf profile、1 GiB 弱网续传、真实 OIDC（RS256 + JWKS）**：与 P1 同源，详见 [P1 验证记录](p1.md) §未实现项。
5. **`cargo xtask check` 全门禁**：项目规定子任务不应运行项目级门禁；本轮只跑聚焦测试 + 类型检查 + 单 test crate。完整 `cargo xtask check` 在主集成阶段执行。

## 9. 复现说明

```bash
git status                                      # 确认 working tree 仅含本节列出的文件
cargo build -p xtask --locked                   # §3
cargo xtask e2e --suite connected-landing       # §1
cargo xtask e2e --suite connected-landing-web   # §2
cargo test -p conex-host --offline --test connected_landing_config
cargo test -p conex-host --offline --test web_auth
bun test sdk/typescript/tests/
bun run typecheck
```

复现时不要设置 `CONEX_E2E_URL` / `CONEX_E2E_TOKEN`（这是 p0-ts 套件的环境，不是本轮）；落地页演示如果手工跑，请使用 `cargo xtask landing-demo` 并在控制台读取临时 UI token，**不要写进任何日志**。
