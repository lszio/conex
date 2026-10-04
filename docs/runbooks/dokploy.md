# Dokploy 部署运行手册

> 状态：本轮把仓库已有的 `Dockerfile` + `deployment/docker-compose.yml` 串到 Dokploy 的「Compose」service 模型上。`web_guest = true` 默认开启；访客打开落地页就走匿名 guest 会话，受 broker 的只读白名单约束（`endpoint/list` + `connection/list` + `source/*`）。
>
> 上游：[connected-landing 契约](../contracts/connected-landing.md)、[connected-landing 运行手册](connected-landing.md)、[设计 v6](../design/2026-09-14-conex-design.md)。

## 1. 范围

- 一台 Dokploy 节点跑三个容器：`host`、`agent-a`、`agent-b`，通过 Dokploy 内置 bridge 网络互联。
- TLS 在 Dokploy 反代终结；容器内始终是明文 8787（loopback-only 是文档示例），agents 与 host 之间走 Dokploy 内部网络，证书由 `ca.pem` 挂入验证。
- 数据持久化：`conex-content` / `conex-session` / `conex-operation` 三个 Dokploy 命名 volume；agent 文件根用 `agent-{a,b}-data` host bind mount（或 Dokploy volume）。
- Secrets：`agent-a.token` / `agent-b.token` 通过 Compose `secrets:` 由 Dokploy 文件注入。

**不在本轮**：跨主机反连、真实 OIDC（RS256 + JWKS）、WSS protobuf profile、1 GiB 弱网续传。这些能力在 host 已实现，但需要真实证书与跨主机环境做证据采集。

## 2. 前置准备

| 项 | 来源 |
|---|---|
| 域名 + TLS | 你提供（CONEX_PUBLIC_ORIGIN / CONEX_INTERNAL_HOSTNAME）。Dokploy 反代自动 Let's Encrypt 或你上传自备证书。 |
| `ca.pem` | 你的 CA 证书（覆盖 `host.example.com` 与 intermediate），上传到 Dokploy secrets。 |
| 两个 agent token 明文 | 你在密码管理器生成，建议 32+ 字节随机；**永不写入仓库**。 |
| UI admin token 明文 | 同上；用于运维与高权登录（demo 与生产都可选）。 |
| Agent 文件根内容 | 给访客读的目录树（团队 wiki、文档等）。 |

四个 SHA-256 摘要（在 Dokploy 上跑）：

```bash
scripts/gen-token.sh "<ui-token>"
scripts/gen-token.sh "<agent-a-token>"
scripts/gen-token.sh "<agent-b-token>"
```

把三个 hex 摘要分别填入 host.toml 的 `<sha256-of-…-token>` 占位。

## 3. Dokploy 步骤

1. **创建 project**：Dokploy → Projects → New → 命名 `conex`。
2. **导入 Compose**：项目内 → Services → Compose → 选择仓库 `git@github.com:lszio/conex` 分支 `refactor/arch`（或部署分支）。Compose file path 填 `deployment/docker-compose.yml`，Dockerfile 路径 `Dockerfile`。
3. **环境变量**：Compose service 页面 → Environment variables，**直接覆盖 `deployment/.env.example` 中每一个占位**：
   - `CONEX_PUBLIC_ORIGIN=https://landing.example.com`
   - `CONEX_HOST_ORIGIN=conex://host.example.internal`
   - `CONEX_INTERNAL_HOSTNAME=host.example.com`
   - `CONEX_AUDIENCE=host.example.internal`
   - `CONEX_HOST_PORT=8787`
   - `CONEX_TENANT_ID=production`
   - `CONEX_WEB_GUEST=true`（生产私密部署改为 `false`）
   - `CONEX_AGENT_A_ID=agent-a` / `CONEX_AGENT_B_ID=agent-b`
   - `CONEX_CA_PEM=/etc/conex-extra/ca.pem`（用 Dokploy 文件挂载覆盖）
4. **File Mounts**：Dokploy → Service → File Mounts，给 `host` service 挂：
   - `./host.toml` → `/etc/conex/host.toml`（来自上面的 host.toml 模板，`<...>` 全部替换）
   - `./ca.pem` → `/etc/conex/ca.pem:ro`（agent + host 共用同一份）
   - `agent-a.toml` → `agent-a` service 的 `/etc/conex/agent.toml`
   - `agent-b.toml` → `agent-b` service 的 `/etc/conex/agent.toml`
   - `agent-a-data/` → `agent-a` service 的 `/srv/agent-a:ro`
   - `agent-b-data/` → `agent-b` service 的 `/srv/agent-b:ro`
5. **Secrets**：Dokploy → Service → Secrets，给三个 service 挂：
   - `agent_a_token`（host + agent-a 都要）→ 文件内容 = 明文 agent-a token
   - `agent_b_token`（host + agent-b 都要）→ 文件内容 = 明文 agent-b token
   Compose `secrets:` 段已经声明；Dokploy 会把同名 secret 文件注入 `/run/secrets/<name>`，agent.toml 里的 `token_backend = "file:/run/secrets/agent_a_token"` 直接消费。
7. **域名 + TLS**：Dokploy → Service → Domains，给 `host` service 挂 `landing.example.com`（指向 8787）。Dokploy 自动申请 Let's Encrypt 证书并反代；TLS 在反代终结，宿主容器内仍是明文。
8. **持久化 volume**：`conex-content` / `conex-session` / `conex-operation` 由 compose 文件定义，Dokploy 第一次部署会创建命名 volume。
9. **构建**：Dokploy → Service → Deploy。Dokploy 会按 Dockerfile 多阶段构建（`bun build:web` → `cargo build --release`）。

## 4. 验证（部署成功后）

```bash
# 1. 落地页可达，guest 会话自动签发
curl -sI https://landing.example.com/
# 期望: HTTP/1.1 200, Content-Type text/html; 多余 Cache-Control 等

curl -si https://landing.example.com/web/session
# 期望: HTTP/1.1 200, Set-Cookie: conex_web_session=…,
#        body {"principalId":"guest","role":"ui","tenantId":"<CONEX_TENANT_ID>"}

# 2. UI 凭据路径仍可用（命名登录）
UI_TOKEN=…   # 来自 password manager
UI_HASH=$(scripts/gen-token.sh "$UI_TOKEN")
curl -si https://landing.example.com/web/login -H "Authorization: Bearer $UI_TOKEN" \
  -H "Origin: $CONEX_PUBLIC_ORIGIN" | head -5
# 期望: HTTP/1.1 200, Set-Cookie: conex_web_session=…,
#        body {"principalId":"ui-admin","role":"ui",…}

# 3. Agent 反连可达（看 host 日志，Dokploy UI 可看）
# 期望日志序列: agent link hello → ready → registered;
# 没有 "Connection refused"（启动序列握手前那两行除外）。
```

## 5. 升级 / 关闭

- **代码升级**：Dokploy → Service → Redeploy；自动重新走 Dockerfile 多阶段构建。
- **访客模式热切换**：在 Dokploy 上把 `CONEX_WEB_GUEST` 改 `false` → Redeploy；访客打开页面将看到登录框（旧的 guest cookie 仍可继续用直到过期）。
- **关闭访客 + 保留 cookie 会话**：`web_guest = false` 后已签发的 guest 会话继续工作（cookie 不过期即可），新访客需要凭据。

## 6. 已知缺口

- 跨主机反连 / TLS 端到端未在本机 loopback 之外的证据；Dokploy 内部的 TLS 由反代终止，agent → host 的 WSS 段在 Dokploy bridge 内是明文（agent.toml 的 `expected_server_name` 与 `ca_pem` 只对反代外的真实出站生效；同主机部署下跳过这两个校验是合理的，但生产跨主机部署必须把它们填好）。
- 真实 OIDC（issuer pin + RS256 id_token）未启用；当前 agent token 与 UI token 走 SHA-256 文件后端。
- `connection/list` 面板对访客可见自身行（principal=guest）；生产私密部署建议 `web_guest = false` 让访客直接进登录。

## 7. 故障排查

| 现象 | 检查 |
|---|---|
| 落地页 502 | host 容器未 healthy：Dokploy 日志看 listen / config 报错 |
| `/web/session` 401 | `web_guest = false`（访客模式关闭）；或 `web_origin` 配置错导致 Set-Cookie 被拒 |
| Agent `Connection refused` 持续 | host 网络未暴露给 agent；检查 Dokploy `depends_on` 与 compose `network_mode` |
| Agent `certificate verify failed` | agent.toml 的 `ca_pem` 与 host 反代证书链不匹配；或 `expected_server_name` 与 `CONEX_INTERNAL_HOSTNAME` 不一致 |
| 业务方法 Forbidden | broker UI 白名单生效；guest 调非只读方法被拒，符合设计 |