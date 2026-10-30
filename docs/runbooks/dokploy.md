# Dokploy 部署运行手册

> 当前部署（2026-10-19）：`https://conex.lszio.space` 由 Dokploy 跟踪 `dev`，运行 `fa75832`；入口通过 Authentik `conex-users` 组限制访问，目前仅 `lszio`。旧公开免登录演示已改为受限工作台；应用内部仍使用 guest 会话，不等于 S4 用户/Space 授权完成。实测见 [验证记录 §9](../verification/landing-page.md#9-authentik-受限部署2026-10-19)。
>
> 上游：[connected-landing 契约](../contracts/connected-landing.md)、[connected-landing 运行手册](connected-landing.md)、[设计 v6](../design/2026-09-14-conex-design.md)。

## 1. 两种部署形态

| 形态 | 用途 | 配置 | 组成 |
|---|---|---|---|
| **受限工作台**（当前线上） | Authentik 登录后使用已有客户端/hello/共享文件工作台 | `docker/host.toml` + Dokploy 域名认证中间件 + Authentik 组策略 | 单个 Host 容器 |
| **多主机服务** | 真实反连 Agent、按主体静态批准 | `deployment/host.toml.example` + Secrets/File Mounts | Host + Agent |

镜像的 guest 模式不授予 Authentik 用户 Conex 业务身份；此次只把既有工作台限制到批准访问者，不能据此公开多用户业务或宣称 S4 已交付。生产内部授权仍须按 S4 落地。

## 2. 受限工作台形态（当前线上）

Dokploy Application 使用仓库 `lszio/conex` 的 `dev` 分支、根 `Dockerfile`，监听容器端口 8787；`autoDeploy=true`，后续 push 到 `dev` 会触发生产构建。部署前应明确这一副作用。

本地 `deployment/docker-compose.yml` 仍可用于单机演示，但它**不包含 Authentik 入口保护**，不能直接按公开生产配置使用：

```bash
docker compose -f deployment/docker-compose.yml up --build
```

| 镜像路径 | 当前内容 |
|---|---|
| `/etc/conex/host.toml` | guest 会话、P1 数据根、`web_origin=https://conex.lszio.space` |
| `/opt/conex/web` | 已有工作台静态资源与 llms.txt |

当前镜像没有配置 `[[endpoints]]`、S1 静态服务令牌或计数器节点；这次没有新增 S1 工作台 UI，也没有将 SDK 测试计数器部署成生产数据源。线上操作仍是既有真实客户端/hello/文件场景。

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
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/                # 未登录：302
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/web/session      # 未登录：302
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/tickets          # 未登录：302
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/rpc              # 未登录：302
curl -sS -o /dev/null -w '%{http_code}\n' $ORIGIN/wss              # 未登录：302
```

上述重定向应指向 `auth.lszio.space`。获准账号登录后再检查 Host 的安全响应头和 Secure/HttpOnly 会话 cookie；不能把代理的 302 当成 Host 的 200。

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
| 部署一直 `running`、日志不增长、站点 502 | 基础镜像浮动 tag 的 manifest 拉取卡死；Dokploy 不允许并发起第二次部署 | 已把三个基础镜像固定为 digest；卡死后先 `application.stop` 再 redeploy |
| Agent `Connection refused` 持续 | host 未就绪或网络不通 | 启动握手前那两行属正常；持续出现查 `depends_on` 与网络 |


## 8. 多域名：一份镜像服务生产与 preview

一个镜像会被部署到**多个域名**：生产域名，加上每个 PR 一个 preview 域名。而
`web_origin` 驱动四处校验（`/web/login` 与 `/tickets` 的 Origin 比对、CSRF、ticket
绑定、WSS 升级的 Origin 比对）。单值 `web_origin` 会让 preview 部署「页面能打开、
所有调用 401」——正是 §4 记录的换域名陷阱，只是这次由每个 PR 自动触发。

`web_origins` 追加可接受 origin：

```toml
web_origin = "https://conex.lszio.space"     # 主 origin
web_origins = [
  "https://preview-conex.lszio.space",      # preview 通配展开后的实际域名
]
```

规则：

- `WebAuth` 持有 origin **集合**；会话记录**自己被创建时那个** origin，签发的 ticket
  绑定同一个值。不这样绑定，preview 会话会被钉在生产 origin 上，WSS 升级直接被拒。
- 未配置的 origin 一律 401；格式非法的 `web_origins` 条目在**加载期**就失败，不等到
  第一个请求——不能带着一个永远匹配不上的 origin 启动。
- 通配符不在配置里展开。`docker/host.toml` 保持可审计的明文列表，用到哪种 preview
  域名形态就加一行。

实测（本地真实 host，`/tickets` 带上有效 CSRF）：

```console
production origin                  bound origin: https://conex.lszio.space          201
preview origin (configured)         bound origin: https://preview-conex.lszio.space 201
unconfigured origin                bound origin: None                             401
```

## 9. PR preview 环境

Dokploy 原生 preview deployment：每个 PR 一个独立容器 + 独立域名，互不影响，`limit`
之外自动回收。已为 Conex 应用（`fEKm9C8gzoKBXN94LAFq4`）开启：

| 字段 | 值 | 含义 |
|---|---|---|
| `isPreviewDeploymentsActive` | `true` | 开启 |
| `previewWildcard` | `preview-conex` | 域名形如 `preview-conex.<pr>.lszio.space` |
| `previewHttps` / `previewCertificateType` | `true` / `letsencrypt` | 与生产一致 |
| `previewPort` | `8787` | 与生产一致 |
| `previewLimit` | `3` | 同时存活的 preview 上限 |
| `previewRequireCollaboratorPermissions` | `true` | 仅协作者可触发 |

**前置条件：通配 DNS。** 2026-10-05 实测本机无法判定——本地 DNS 被代理劫持成
`198.18.x` fake-IP，直连权威 NS（`dns11.hichina.com`）拿到的也是同样地址。判定方法是
穿过代理实际请求：

```console
$ curl -o /dev/null -w '%{http_code}' https://conex.lszio.space/          # 200
$ curl -o /dev/null -w '%{http_code}' https://random-nonexistent.lszio.space/  # 000
```

`000` 说明随机子域没有记录，**通配未配置**。加一条 `*.lszio.space` 指向当前 A 记录
后，preview 域名才能签发证书并路由。

未加通配记录前：preview 容器可以构建启动，但域名解析/证书会失败，页面访问不到。

**当前状态（2026-10-05）：配置已就绪，preview 容器尚未生成。** 设置已写入
（`isPreviewDeploymentsActive: true` / `previewWildcard: preview-conex`），但
`previewDeployments` 始终为空：本实例安装的 GitHub App
（`dokploy-2026-09-09-7mx4i4`，installationId `160348011`）只在 push 到跟踪分支时
投递事件，PR 的 `synchronize`/`opened` 事件没有到达 Dokploy，因此没有触发 preview
部署。普通 push 会正常触发**生产**部署（`deployment.queueList` 里可见
`applicationId=fEKm9C8gzoKBXN94LAFq4` 的条目）。

也就是说：通配 DNS 补齐之前这一步就卡住；补齐之后还需要 GitHub App 侧确实投递
PR 事件。验证判据是 `application.one` 的 `previewDeployments` 出现条目并带
`url`/`domain`。这两件事都做完之前，不要把「preview 可用」写进任何交付说明。

## 10. 生产分支切换

Dokploy 应用 `fEKm9C8gzoKBXN94LAFq4` 部署 `branch`。切换即改该字段并重新部署：

```bash
dokploy application update --applicationId fEKm9C8gzoKBXN94LAFq4 --branch dev
dokploy application deploy  --applicationId fEKm9C8gzoKBXN94LAFq4
```

**`application.redeploy` 与 `application.deploy` 都不会新建部署记录**（返回
`json: null`，`deployments` 列表不变），只有 `dokploy application deploy` 真的入队。
判据是 `application.one` 里最新一条 `deployments[0].status` 变成 `running`。

切换前先确认线上现状，便于回退：

```console
$ curl -sS -o /dev/null -w '%{http_code}\n' https://conex.lszio.space/
302
```

## 11. Authentik 受限入口（2026-10-19）

- 应用：`Conex`，slug=`conex`，Proxy Provider `conex-forward-auth`（id=8），模式 `forward_single`，external host=`https://conex.lszio.space`。
- 访问组：`conex-users`；应用仅绑定此组，policy mode=`all`，失败关闭。当前成员只有 `lszio`。扩展访问应由管理员明确加组，不要删掉绑定或改成所有已登录用户可访问。
- 复用默认 implicit-consent 授权 flow 和 invalidation flow，并加入 embedded outpost；不修改其他应用或共享中间件。
- Dokploy 域名 `GMpeTZsJuF2w3ghfNz7az` 的 `middlewares` 为 `authentik-auth@file`。应用没有发布 Host 端口，入口流量必须经过 Traefik。
- 使用 Authentik API 创建 Proxy Provider 时会自动调用 `set_oauth_defaults()`；经 ORM 管理时也必须调用它并保存，生成严格 callback URI 与 grant types。否则页面会报 Redirect URI Error。不要手工删掉 callback 的 `?X-authentik-auth-callback=true`。

本次保护全部外部路径，包括 `/rpc`、`/wss`、`/tickets`。未登录的机器 SDK 同样会被重定向，不能把此域名当作已配置好的 S1 bearer 服务端点。后续若开放机器入口，必须单独设计并验证服务凭据与静态业务批准，不能为省事取消全站保护。

实际浏览器验证使用两个临时非管理员账号：有组账号完成密码登录、callback、Conex 会话、同 cookie 两标签页独立链接和真实 hello；无组账号登录后被拒绝。完成后删除两账号，关闭浏览器，复查组成员仅 `lszio`，没有保留测试密码或 API token。

当前是部署入口访问控制；Conex 内部仍为 guest 主体。OIDC 用户映射、节点/Space 资源授权、撤权传播属于 S4，未在此次部署中实现。
