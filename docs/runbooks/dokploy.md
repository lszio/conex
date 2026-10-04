# Dokploy 部署运行手册

> 状态：**公开落地页已部署并验证**（2026-10-04）。`https://conex.lszio.space` 跑单容器 guest-only 演示栈：访客免凭据浏览、真实只读调用、`/llms.txt` 可用。逐项实测输出见 [落地页验证记录](../verification/landing-page.md)。
>
> 上游：[connected-landing 契约](../contracts/connected-landing.md)、[connected-landing 运行手册](connected-landing.md)、[设计 v6](../design/2026-09-14-conex-design.md)。

## 1. 两种部署形态

| 形态 | 用途 | 配置 | 组成 |
|---|---|---|---|
| **公开演示**（当前线上） | 落地页、访客免登录浏览、llms.txt | `docker/host.toml`（镜像内烘焙） | 单个 `host` 容器 |
| **多主机生产** | 真实反连 Agent、按主体授权 | `deployment/host.toml.example` + Secrets/File Mounts | `host` + `agent-a` + `agent-b` |

公开演示形态没有任何 secret：镜像自带合成内容树与 guest-only 配置，克隆即可部署。
生产形态的步骤见 §6。

## 2. 公开演示形态（当前线上）

`deployment/docker-compose.yml` 只有一个 service，无环境变量、无 File Mount、无 Secrets：

```bash
docker compose -f deployment/docker-compose.yml up --build
# 或 Dokploy：Compose service → compose path `deployment/docker-compose.yml`
#            Dockerfile `Dockerfile` → Deploy
```

镜像内烘焙的内容：

| 路径 | 内容 |
|---|---|
| `/etc/conex/host.toml` | guest-only 配置，3 个 `source-fs` 端点，`web_root=/opt/conex/web` |
| `/opt/conex/web` | 落地页静态资源（`index.html` / `app.js` / `style.css` / `llms.txt`） |
| `/opt/conex/demo` | 合成演示内容树（docs / media / archives） |

三个只读端点（全部 `source-fs`，授权 `root = ""` + `subtree = true`）：

| endpointId | 目录 | 覆盖的预览路径 |
|---|---|---|
| `docs` | `readme.md`、`architecture.md` | Markdown / 纯文本 |
| `media` | `clip.mp4`、`clip.webm`、`diagram.png` | 视频（含 Range 拖动）、图片 |
| `archives` | `report.docx`、`bundle.zip`、`random.bin` | DOCX 受限预览、ZIP 只列不展开、二进制元数据下载 |

## 3. 明文监听：两个开关不要混用

`host.toml` 里的两个明文开关语义不同，混用会导致容器起不来或语义错误：

| 开关 | 含义 | 约束 |
|---|---|---|
| `allow_loopback_http` | 明文**绝不离开本机** | 必须配 loopback `listen`；同时决定会话 cookie 是否带 `Secure` |
| `allow_plaintext_bind` | 明文监听非 loopback 地址（反代后容器内属正常） | 与 TLS 互斥检查独立 |

反代后的容器必须用 `allow_plaintext_bind = true`。写成
`listen = "0.0.0.0:8787"` + `allow_loopback_http = true` 会被 `config.rs` 在加载期拒绝，
容器直接退出——这正是本轮修复前的真实故障。

`docker/host.toml` 里 `allow_plaintext_bind = true` + `allow_loopback_http` 缺省，
因此会话 cookie 仍带 `Secure`（公网 HTTPS 域下正确）。

## 4. web_origin 必须与公网 origin 逐字一致

`web_origin` 驱动三处校验：`/web/login` 与 `/tickets` 的 Origin 比对、
ticket 绑定、WSS 升级的 Origin 比对。写错的表现是访客能打开页面但一切调用 401。

改域名后必须同步改 `docker/host.toml` 的 `web_origin` 并重新构建镜像。

## 5. 验证（部署后）

```bash
ORIGIN=https://conex.lszio.space
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/                # 200
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/llms.txt        # 200
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/llm.txt         # 200（别名）
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/not-found       # 404（不回退 HTML）
curl -sSi $ORIGIN/web/session | head -3                           # 200 + Set-Cookie
```

响应头（`/` 与 `/llms.txt` 均带）：`content-security-policy`、`x-content-type-options: nosniff`、
`x-frame-options: DENY`、`cache-control: no-store`。

## 6. 多主机生产形态

需要真实反连 Agent 与按主体授权时，改用 `deployment/host.toml.example`：

1. **project → Compose**：仓库 `lszio/conex`，compose path `deployment/docker-compose.yml`（旧三服务版含 `agent-a`/`agent-b`，需要额外环境变量与 File Mounts），或按 [Dokploy 部署运行手册 §3](connected-landing.md) 手工挂载。
2. **环境变量**：`CONEX_HOST_CONFIG` / `CONEX_CA_PEM` / `CONEX_AGENT_{A,B}_{CONFIG,DATA,TOKEN_FILE}`。注意 `${VAR:?msg}` 语法会让 Dokploy 在注入前解析失败，用 `${VAR:-}` 并在应用侧 fail-closed。
3. **Secrets**：`agent_a_token` / `agent_b_token` 文件内容为明文 token；`host.toml` 只存 `sha256(<plaintext>)`，用 `scripts/gen-token.sh` 生成。
4. **Agent 出站**：agent.toml 的 `host_url` 必须是 `wss://`，`ca_pem` 与 `expected_server_name` 指向反代证书。内网无需开入站端口。
5. **访客开关**：删掉 `[web_guest]` 即回到「必须凭据」模式。

## 7. 故障排查

| 现象 | 原因 | 处理 |
|---|---|---|
| 容器退出，日志 `config error: allow_loopback_http requires a loopback listen address` | 用 `allow_loopback_http` 配了 `0.0.0.0` | 改用 `allow_plaintext_bind = true`（§3） |
| 容器退出，日志 `protoc failed: google/protobuf/struct.proto: File not found` | 构建镜像缺 well-known protos | Dockerfile 已装 `libprotobuf-dev`；勿回退 |
| 落地页 502 | 容器未 healthy | 看 Dokploy 日志的 listen/config 报错 |
| 页面能开但全部调用 401 | `web_origin` 与真实 origin 不一致 | 同步后重新构建（§4） |
| `/llms.txt` 404 | `web_root` 缺该文件 | `bun run build:web` 现在缺文件即失败 |
| 视频能播但拖不动 | 响应缺 `Content-Length` | 已修（`content_http.rs`）；旧镜像需重建 |
| Agent `Connection refused` 持续 | host 未就绪或网络不通 | 启动握手前那两行属正常；持续出现查 `depends_on` 与网络 |
