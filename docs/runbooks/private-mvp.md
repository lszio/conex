# 私有 MVP 部署运行手册（公网 hub + 私网节点）

> 用户于 2026-11-03 明确授权私有产品公网部署（已执行；原 Authentik 受限访客演示已替换）。
> 2026-11-04 实际状态：hub 已上线（`CONEX_HUB_SIGNING_KEY` 已配置、`conex-data` 卷挂载 `/var/lib/conex`、Authentik 中间件已从域名移除）；本机节点已通过 wss 连接并注册。验收边界：A02 的"两台真实机器"与 Linux 采样核对仍归 P9。
> 域名上曾有的 Authentik 中间件必须摘除的原因：它会拦截节点的 WSS 发布链握手（无 SSO 会话的连接被 302），产品自身的引导码认证是其唯一入口。若需恢复双重认证，在 Dokploy 域名设置里把 `authentik-auth@file` 加回 middlewares（但这会同时使节点无法连接）。

## 0. 拓扑

```
浏览器 ──HTTPS──▶ 公网 hub（Dokploy, conex.lszio.space）
                     │  ▲
        WSS 出站(节点主动)  │ grant/票据后直连
                     ▼  │   （同机 loopback / 跨机经家中反代子域）
                  私网节点（Mac，conex-agent：OMP/metrics/relations）
```

- hub 不落对话正文（全部私有模块 direct-only），只持有目录/授权/撤权。
- 节点主动 WSS 出站，无需入站端口；浏览器经 hub 登录、取 grant，然后**直连节点**。直连地址按 nodeId 记忆在工作台"节点地址"输入框：
  - **同机浏览器**（浏览器与节点同一台 Mac）：`http://127.0.0.1:7451`。
  - **跨机/公网浏览器**（2026-11-12 起支持）：节点 `direct_listen` 已放开到内网网卡 `192.168.8.228:7451` + `allow_remote_direct = true`（`node_config.rs` 校验非 loopback 必须带此开关；TLS 由家中公网反代在子域上终结——与 hub 的明文监听+Dokploy 反代终结 TLS 同一模式）。家中反代加一条 `conex-node.lszio.space → http://192.168.8.228:7451`（须支持 WebSocket 升级，现有入口已验证支持），工作台"节点地址"填 `https://conex-node.lszio.space` 即可。路由器建议为 Mac 固定 DHCP 保留 `192.168.8.228`，地址一变监听会 fail-fast。

## 1. hub（Dokploy）

镜像由仓库根 `Dockerfile` 构建，`docker/host.toml` 已是私有形态（无 `[web_guest]`）。`dev` 分支 push 即自动构建。

首次部署/换钥需要操作者完成的步骤：

1. **签名私钥（已完成）**：`CONEX_HUB_SIGNING_KEY`（base64 pkcs8 DER）已写入 Dokploy 环境变量，本机留档 `~/.conex/home-node/hub-key.txt`（第 1 行公钥 hex，第 2 行 base64 私钥）。没有该变量 hub 拒绝启动（fail-fast）。
   轮换：生成新钥 → 节点先装新 kid 公钥 → hub 改 `kid` 并换 env → 旧钥保留至最后授权 exp + 60 秒（详见 [P5 手册 §2](private-network.md)）。注意 host 配置校验支持 `env:`/`env64:`/`file:` 三种凭据来源（4e508ab 修复）。

2. **持久卷（已完成）**：`conex-data` 卷已挂载 `/var/lib/conex`（含 `private.sqlite`、content/session/operation 根）。容器内明文监听由 Dokploy 反代终结 TLS（`allow_plaintext_bind`）。

3. **登录**（账号为主，引导码为首次建号与恢复凭证）：

   ```sh
   # Dokploy 容器控制台执行（也可 ssh labray.lan docker exec 代取）：
   conex-host private-bootstrap --config=/etc/conex/host.toml
   # stdout 打印 43 位引导码，5 分钟有效、单次消费
   ```

   - **首次部署**：打开 https://conex.lszio.space 输入引导码 → 页面转为"设置管理员账号"（用户名 + 密码，Argon2id 存储，账号表只存 PHC 串）→ 此后日常用账号密码登录，**hub 重启不再需要引导码**。
   - **日常登录**：账号密码。失败统一 401（不区分用户名/密码错误），按用户名指数退避（0.5s 起倍增，上限 15s）+ 全局 60 次/分钟限速。
   - **忘记密码**：容器终端取一条新引导码 → 登录页"使用引导码登录" → 重置密码 → 进入。
   - 会话仍在 hub 内存（重启即失效），重新登录即可，无需再引导。引导码不进 URL/日志，库中只存哈希。
   - 会话内改密：`POST /private/account/password`（登录页引导码路径同款表单）。

   路线 B（OIDC，预留未启用）：`accounts.kind='oidc'` 行带 `(issuer, subject)` 唯一索引，`oidc_jwt.rs` 已有 RS256/JWKS 验签器；接入时新增 IdP 登录回调把身份映射到账号行即可，本地账号与 OIDC 身份互不混用。

部署后验证：

```sh
curl -s -o /dev/null -w '%{http_code}\n' https://conex.lszio.space/            # 200（登录页）
curl -s -o /dev/null -w '%{http_code}\n' https://conex.lszio.space/private/directory  # 401（未认证拒绝）
```

## 2. 节点（本机 Mac）

配置在 `~/.conex/home-node/`（不入库）：`node.toml`（含 hub 公钥、批准 Origin、OMP/指标/关系模块）、`application.toml`、`node-token.txt`（0600）、`start.sh`。

```sh
~/.conex/home-node/start.sh        # 首次会自动 cargo build --release
```

- 发布凭据经 `CONEX_NODE_TOKEN` 环境变量注入（hub 端只存其 sha256）。
- Agent 工作区是 `~/conex-workspace`；模型为 `deepseek/deepseek-flash`（本机 `omp` 环境授权，凭据不离开节点）。
- 直连入口 `192.168.8.228:7451`（内网网卡明文，`allow_remote_direct = true`；公网 TLS 由家中反代子域终结，同机浏览器仍可用 `127.0.0.1:7451`）。
- 节点重启 = 新 epoch：所有旧 grant 失效（预期），浏览器刷新页面即恢复。

节点健康：

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://192.168.8.228:7451/wss   # 400（入口在，需升级头；同机另可探 127.0.0.1:7451）
echo node/snapshot | 私有 socket ~/.conex/home-node/data/operator.sock  # 实例/模块/代际
```

## 3. 首次使用清单（2026-11-04 状态：第 1–2、4 步已完成）

1. ~~push dev → Dokploy 构建完成。~~（已上线）
2. ~~Dokploy 设置 `CONEX_HUB_SIGNING_KEY`，重启应用。~~（已配置）
3. **待用户执行**：Dokploy 面板 → conex 应用 → 容器终端执行
   `conex-host private-bootstrap --config=/etc/conex/host.toml`
   → 43 位引导码（5 分钟、单次消费）→ https://conex.lszio.space 登录。
4. ~~本机 `~/.conex/home-node/start.sh` 启动节点~~（已在运行并注册；hub 重启后会自动重连重注册）。
5. 登录后工作台出现节点身份与"直连"徽标 → 选端点/Workspace → 新建会话发消息。

## 4. LLM 提供者与中继（P10，2026-11-13 地基已交付）

节点把 OpenAI 兼容上游（首实例 omniroute）发布为 `conex.llm` 能力（relay-allowed），hub 经既有转发路径暴露 `/v1/models` 与 `/v1/chat/completions`；外部任意 OpenAI 兼容客户端配 `base_url = https://conex.lszio.space/v1` + API key 即可调用家里模型。流式为整段单帧 + 终帧 + `[DONE]`（中继上结果已完整到达，不伪装增量）。

hub `host.toml` 增补（全部为管理员批准项，缺一不可）：

```toml
wss_link_timeout_ms = 300000        # 链路协商超时上限；LLM 调用 > 8s 必须抬高

[[capabilities.contracts]]
contract_id = "conex.llm"
[[capabilities.contracts.methods]]
name = "llm/providers/list"
input_schema = "conex.LlmProvidersListInput"
output_schema = "conex.LlmProvidersListResult"
action = "llm.providers.list"
execution = "read_only"
[[capabilities.contracts.methods]]
name = "llm/chat"
input_schema = "conex.LlmChatInput"
output_schema = "conex.LlmChatResult"
action = "llm.chat"
execution = "non_replayable"

# node_id 填节点发布链路的 principal（hub [[tokens]] 里的 principal_id），不是节点 ULID。
[[capabilities.publish]]
node_id = "<node-link-principal>"
space_id = "private"
integration_id = "local-apps"
provider_endpoint_id = "<endpoint-prefix>-<app-id>-llm"
contract_id = "conex.llm"

[[capabilities.consume]]
principal_id = "<gateway-consumer-principal>"   # 缺省取 [private].principal
space_id = "private"
provider_endpoint_id = "<endpoint-prefix>-<app-id>-llm"
contract_id = "conex.llm"
methods = ["llm/providers/list", "llm/chat"]

[llm_gateway]
space_id = "private"
provider_endpoint_id = "<endpoint-prefix>-<app-id>-llm"
# consumer_principal/consumer_tenant 缺省取 [private]；显式覆盖亦可
timeout_ms = 180000                # ≤ wss_link_timeout_ms
max_concurrent_per_key = 4
keys = [
  # printf %s "明文" | sha256sum；明文只显示一次，永不落盘
  { name = "primary", token_hash = "<64-hex>" },
  # 可选：受限 key（models 为空 = 全部已发布模型）
  { name = "restricted", token_hash = "<64-hex>", models = ["qwen3:8b"] },
]
```

节点 `node.toml` 增补：

```toml
[publication]
# ... 既有字段 ...
link_timeout_ms = 300000           # 与 hub 侧共同决定协商值（取小）
exposure_principals = ["<gateway-consumer-principal>"]   # 中继调用主体的节点侧准入
exposure_tenants = ["<hub tenant>"]

[llm]
[[llm.providers]]
id = "omniroute"
base_url = "http://<omniroute>/v1"  # OpenAI 兼容根；凭据经 api_key_backend = "env:..." / "file:..."
models_allowlist = ["qwen3:8b"]     # 空 = 上游全量列表（omniroute 模型面是动态的）
# timeout_ms = 180000               # 单次补全上游预算，缺省 120s
```

应用包（`application.toml`）追加模块：

```toml
[[modules]]
id = "llm"
implementation = "node-llm"
permissions = ["llm.providers.list", "llm.chat"]
exports = [{ contract_id = "conex.llm", strategy = "relay-allowed" }]
```

签发 key：本地生成强随机串 → `printf %s "<key>" | sha256sum` → 哈希填入 `keys` → 明文交给调用方一次。撤销 = 从配置移除该 key 并重启 hub。网关负路径行为：无/错 key → 401；key 未授权该模型 → 403；模型未发布 → 400；节点离线 → 503；生成中断 → 502（结果未知，不自动重试）。

验收：`cargo xtask e2e --suite private-llm`（真实 hub + 真实节点 + fixture 上游，L01–L04 可自动化部分）。真实 omniroute 接入与两机公网 live（L05–L07）待执行；反代需确认 SSE 不被缓冲（`stream:true` 响应为 text/event-stream）。

## 5. 边界与回退

- 本部署替换了原访客演示（`docker/host.toml` 的 `[web_guest]` 已移除）。回退需要恢复旧配置并重建镜像，属显式动作。
- 撤权/轮换/故障排查见 [P5 手册](private-network.md)；撤销 owner 主体会同时注销全部浏览器会话。
- 已知边界：Chrome 的 Private Network Access 预检未来强制时，跨机 https→loopback 直连需节点补 PNA 头（P9）；事件流为 800ms 有界轮询（直连入口无推送通道）。
