# connected-landing 运行手册

> 状态：**本轮落地页已可在 loopback 多进程环境验证**（2026-09-21）。`cargo xtask landing-demo` 与 `cargo xtask e2e --suite connected-landing(-web)` 是当前可重复的真实证据。跨主机 / 真实证书 / 真实浏览器可访问部署仍待环境具备后补做，详见 [verification](verification/connected-landing.md) §未验证项。
>
> 上游：[中心服务与交互落地页计划](../../plans/2026-09-20-conex-connected-landing.md)、[Connected landing 契约](../../contracts/connected-landing.md)、[P1 运行手册](p1.md)。

## 1. 范围（当前实际可达）

- 中心 Host（`conex-host`）同端口提供 `index.html` + `/rpc` + `/web/login` + `/web/session` + `/web/logout` + `/tickets` + `/wss`，由同一 token 哈希 / agent 配置驱动。
- 两个真实 `conex-agent` 子进程，主动出站连 `ws://host/wss`，经 hello / ready / register 后绑定到 `notes-a` / `notes-b` 远端端点。
- TypeScript SDK：`@conex/sdk` 的 `ConexClient`（HTTP /rpc + bearer）与 `ConexWsClient`（cookie → ticket → WSS）。
- 落地页静态文件：`web/dist/{index.html,app.js,style.css}`，由 `bun run build:web` 编译 `web/src/*`。

**未实现 / 不在本轮：**

- 跨主机真实部署证据（不同主机、不同网络、不同证书）。当前两 Agent 与 Host 都在 127.0.0.1。
- 真实浏览器自动化截图（计划 §L09 1440px / 390px 视口手动验收），headless 烟雾仅用 SDK 路径。
- WSS protobuf profile 与 1 GiB 弱网续传 — 与 P1-04 同源，沿用 [P1 运行手册](p1.md) §未实现项。

## 2. 启动本地三进程演示

依赖：`./scripts/bootstrap-toolchain.sh` + `source .toolchain/env.sh` + `bun install`。下列命令均使用 `cargo xtask`，无需手写配置文件。

```bash
# 一次性
bun run build:web                 # 生成 web/dist/{index.html, app.js, style.css}
cargo build -p conex-host -p conex-agent --locked

# 启动（前台，按 Ctrl-C 退出）
cargo xtask landing-demo
```

启动后：

1. 控制台会打印一行 `UI login credential (temporary; not stored in logs): <64-hex-chars>`。**该 token 不会写入任何日志或服务访问审计**；复制下来用于同源的 `/web/login`。
2. 同源页面 URL：`http://127.0.0.1:<port>/`（port 取自 `--port` 或自动选择）。
3. 关闭 Host：按一次 `Ctrl-C`，`tokio::signal::ctrl_c` 触发，Host 与两个 Agent 全部优雅退出。

`cargo xtask landing-demo` 生成的临时目录包含：

| 路径 | 内容 | 权限 |
|---|---|---|
| `<tmp>/host.toml` | Host 配置（hash 化 token、Agent 凭据映射、`web_origin` / `web_root`、端点列表） | 0600 |
| `<tmp>/agent-{a,b}.toml` | 两个 Agent 配置（host_url、`token_backend = file:`、本地 `[[endpoints]]`） | 0600 |
| `<tmp>/agent-{a,b}.token` | Agent 出站凭据明文 | 0600 |
| `<tmp>/agent-{a,b}-files/` | 各 Agent 受限 fs root，含 `team/shared.md` 与 `only-this-agent.md` | dir |

进程退出时整个临时目录被 `Drop for DemoProcesses` 清理；不会残留 secrets。

## 3. host_origin / audience / token 三元组

| 角色 | 名称 | `principal_id` | `audience` | `role` | 登录入口 |
|---|---|---|---|---|---|
| UI（页面） | demo-ui | demo-ui | host.local | ui | `/web/login` → cookie → `/tickets` → `/wss` |
| Service（脚本） | demo-probe | demo-probe | host.local | service | `/rpc` + `Authorization: Bearer …` |
| Agent A | agent-a | agent-a | host.local | agent | `/wss` 升级（agent 出站） |
| Agent B | agent-b | agent-b | host.local | agent | `/wss` 升级（agent 出站） |

- **audience** 必须与 Host 配置的 `audience` 一致；不一致会得到 `audience mismatch`。
- **host_origin** 在演示里统一是 `conex://host.local`，Agent 的 `host_origin` 与 Host `host_origin` 必须完全相同，否则 `agent/register` 在握手期间被拒。
- **token rotation** 是按 secret 后端重写文件（如 `<tmp>/agent-a.token`）→ 进程内自动重连加载；UI token 需在 `host.toml` 中替换 `token_hash` 并重启 Host。所有 token / secret 文件必须保持 `0600` 属主私有，**部署示例不写明文**。

## 4. 关键配置示例

完整示例见 [`deployment/host.toml.example`](../../deployment/host.toml.example)、[`deployment/agent-a.toml.example`](../../deployment/agent-a.toml.example) 与 [`deployment/agent-b.toml.example`](../../deployment/agent-b.toml.example)。摘要：

```toml
# host.toml（节选；UI / agent-a / agent-b 三组 token 必须各自独立 0600）
listen = "127.0.0.1:8787"
allow_loopback_http = true
audience = "host.local"
host_origin = "conex://host.local"
web_origin = "http://127.0.0.1:8787"        # 部署到公网时改为 https://landing.example.com
web_root = "/opt/conex/web"
content_root = "/var/lib/conex/content"
session_root = "/var/lib/conex/session"
operation_root = "/var/lib/conex/operation"

[[tokens]]
token_hash = "<sha256(ui-bearer)>"
principal_id = "ui-admin"
tenant_id = "production"
audience = "host.local"
role = "ui"

[[endpoints]]
id = "notes-a"
tenant_id = "production"
provider_id = "notes-a-provider"
kind = "source-remote"          # 关键：远端端点绑定到 Agent
provides = ["source/list", "source/read", "source/search"]
root = "team"
agent_id = "agent-a"
```

Agent 配置必须显式信任 `host_origin`、`ca_pem` 与 `expected_server_name`；明文 ws 仅允许 loopback 开发模式（`allow_loopback_ws = true`）。

## 5. docker-compose 栈

`deployment/docker-compose.yml` 的最小启动步骤：

```bash
# 1. 生成两份 0600 agent token
umask 077
printf %s "agent-a-secret" > /run/conex/agent_a_token
printf %s "agent-b-secret" > /run/conex/agent_b_token

# 2. 计算 UI token 的 sha256
ui_token="$(openssl rand -hex 32)"
printf '%s' "$ui_token" | sha256sum | awk '{print $1}' > /tmp/ui_token_hash

# 3. 编辑 deployment/host.toml.example / agent-{a,b}.toml.example
#    把 token_hash、agent credential_name / credential_backend 等占位替换
#    （host.toml：把 <sha256-of-ui-token>、<sha256-of-agent-a-token> 等替换；agent.toml：
#    把 /run/secrets/agent_a_token 等路径保留即可）

# 4. 启动
CONEX_AGENT_A_TOKEN_FILE=/run/conex/agent_a_token \
CONEX_AGENT_B_TOKEN_FILE=/run/conex/agent_b_token \
CONEX_AGENT_A_CONFIG=./agent-a.toml \
CONEX_AGENT_B_CONFIG=./agent-b.toml \
CONEX_AGENT_A_DATA=./agent-a-data \
CONEX_AGENT_B_DATA=./agent-b-data \
CONEX_HOST_CONFIG=./host.toml \
docker compose -f deployment/docker-compose.yml up --build
```

`docker-compose.yml` 已强制 `read_only: true`、tmpfs `/tmp`、secrets 通过 file 注入；`host` 健康检查使用 `curl http://127.0.0.1:8787/`（健康但 200/404 由 host 自定）。

## 6. 日志期望

成功路径必须看到：

- Host 启动：`conex/1 broker listening on <addr>`（仅 stdout，不含任何 token / cookie / ticket 明文）。
- Agent 重连：`agent link closed: connect websocket: IO error: Connection refused` 仅出现于启动序列握手前；握手后只能看到 `hello → ready → registered` 之类的 trace（trace 来自 agent runtime）。
- `cargo xtask landing-demo` 的 `web/dist` 缺失时立即失败（错误指向缺失文件）；不会回退到占位 HTML。

失败语义（必须明确，不静默重放）：

- 认证失败 → `ConexError` code `-32001`、HTTP 401。
- 端点离线 → broker 返回 `unavailable`（code `-32005`）。
- Origin 不匹配 → `web/login` / `/tickets` / `/wss` 三处都会 401。

## 7. 冒烟门禁

```bash
cargo xtask e2e --suite connected-landing            # 现有三进程 source 三方法 + A/B 隔离
cargo xtask e2e --suite connected-landing-web        # 本轮新增：登录 + 会话 + WSS 列表 + 远端读取
cargo xtask e2e --suite connected-landing-connections # L10 新增：UI Link 面板 + 计数器 + stale 检测
```

三条套件**共享同一 fixture 与同源 token**，因此任何一个回归都会在另一条上显形。`cargo xtask check` 在集成阶段把三条套件一起作为门禁。

## 8. 查看 / 回收 UI Link（L10）

`cargo xtask landing-demo` 启动后，落地页右下角的「浏览器 UI 链接」面板每 1 秒刷新一次，列出当前主体名下每条 UI 链接：

| 字段 | 来源 |
|---|---|
| 短哈希 | `linkId`（8 字节随机 base64url）前 6 + 后 2 |
| principal | `principalId` |
| tenant | `tenantId` |
| connected | `connectedAtMs`（会话创建时间） |
| seen | `lastSeenAtMs`（最近一次 ready / 业务帧） |
| calls / in-flight / tickets | `callsTotal` / `callsInFlight` / `ticketsIssued` |

WS 关闭后行不消失，但 `seen` 停止前进；下一次重连会刷新 `seen`。新进入的行带 1.4s 的绿色 flash 提示。

强制回收当前主体的所有 UI Link + 取消其未消费 ticket：

```bash
# 浏览器侧：调用 /web/logout（页面右上角按钮或手动 fetch）
curl -X POST http://127.0.0.1:<port>/web/logout \
     -H "Origin: http://127.0.0.1:<port>" \
     -H "Cookie: conex_web_session=…" \
     -H "x-csrf-token: <csrf>"
```

注销后 host 同步：
- 撤销该 web session；
- 删除该 session 签发但未消费的 ticket；
- 从 `UiLinkRegistry` 中移除对应 `linkId`（下一次 `connection/list` 不再返回该行）。

## 9. 验证清单

| 项 | 落地证据 |
|---|---|
| `cargo xtask e2e --suite connected-landing` 通过 | [verification §1](../../verification/connected-landing.md#1-cargo-xtask-e2e---suite-connected-landing) |
| `cargo xtask e2e --suite connected-landing-web` 通过 | [verification §2](../../verification/connected-landing.md#2-cargo-xtask-e2e---suite-connected-landing-web) |
| `cargo xtask e2e --suite connected-landing-connections` 通过 | [verification §2b](../../verification/connected-landing.md#2b-cargo-xtask-e2e---suite-connected-landing-connections) |
| `cargo build -p xtask --locked` 无警告 | [verification §3](../../verification/connected-landing.md#3-cargo-build--p-xtask---locked) |
| `cargo test -p conex-host --offline --test connected_landing_config` 通过 | [verification §4](../../verification/connected-landing.md#4-cargo-test--p-conex-host---offline---test-connected_landing_config) |
| `cargo test -p conex-host --offline --test web_auth` 通过 | [verification §5](../../verification/connected-landing.md#5-cargo-test--p-conex-host---offline---test-web_auth) |
| `cargo test -p conex-host --offline --test ui_links` 通过 | [verification §5b](../../verification/connected-landing.md#5b-cargo-test--p-conex-host---offline---test-ui_links) |
| `bun test sdk/typescript/tests/` 通过 | [verification §6](../../verification/connected-landing.md#6-bun-test-sdktypescripttests) |
| `bun run typecheck` 通过 | [verification §7](../../verification/connected-landing.md#7-bun-run-typecheck) |
| 跨主机真实网络验收 | **未验证**（环境未具备，见 [verification §未验证项](../../verification/connected-landing.md#未验证项)） |

## 10. 已知缺口

- 浏览器真实视口截图（1440px / 390px）尚未在本轮采集；落地页逻辑已通过 SDK 路径冒烟，但视觉回归属后续。
- `cargo xtask e2e --suite connected-landing-web` 仍依赖 loopback 明文；真实 TLS + 跨主机路径未在 CI 验证。
- UI 凭据不能用作 `/rpc` 入站（host 显式拒绝 UI 角色直接进 `/rpc`）；落地页必须经 `/web/login` → cookie → `/tickets` → `/wss`。这是 L03 的设计约束，不视为缺陷。
- 跨主机 / 跨网络 / 真实 OIDC（issuer pin + RS256 id_token 签名）不在本轮落地页范围内；如需上线必须先补 [P1 运行手册](p1.md) §1 中标注为「未实现」的能力。
