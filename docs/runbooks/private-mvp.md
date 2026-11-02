# 私有 MVP 部署运行手册（公网 hub + 私网节点）

> 用户于 2026-11-03 明确授权私有产品公网部署（替换原 Authentik 受限访客演示）。
> 验收边界：A02 的"两台真实机器"与 Linux 采样核对仍归 P9；本手册交付单 hub + 单节点的可用部署。

## 0. 拓扑

```
浏览器 ──HTTPS──▶ 公网 hub（Dokploy, conex.lszio.space）
                     │  ▲
        WSS 出站(节点主动)  │ grant/票据后直连（loopback，仅同机浏览器）
                     ▼  │
                  私网节点（Mac，conex-agent：OMP/metrics/relations）
```

- hub 不落对话正文（全部私有模块 direct-only），只持有目录/授权/撤权。
- 节点主动 WSS 出站，无需入站端口；浏览器经 hub 登录、取 grant，然后**直连节点**（当前节点与浏览器同机时为 `http://127.0.0.1:7451`，地址按 nodeId 记忆在工作台"节点地址"输入框）。

## 1. hub（Dokploy）

镜像由仓库根 `Dockerfile` 构建，`docker/host.toml` 已是私有形态（无 `[web_guest]`）。`dev` 分支 push 即自动构建。

首次部署/换钥需要操作者完成的步骤：

1. **注入签名私钥**（Dokploy → Application → Environment）：

   ```
   CONEX_HUB_SIGNING_KEY = <base64(pkcs8 DER)>
   ```

   本机生成结果保存在 `~/.conex/home-node/hub-key.txt` 第 2 行（第 1 行是节点要装的公钥 hex）。没有这个环境变量 hub 拒绝启动（fail-fast，属预期）。
   轮换：生成新钥 → 节点先装新 kid 公钥 → hub 改 `kid` 并换 env → 旧钥保留至最后授权 exp + 60 秒（详见 [P5 手册 §2](private-network.md)）。

2. **持久卷**：`/var/lib/conex` 必须挂持久卷（含 `private.sqlite`、content/session/operation 根）。容器内明文监听由 Dokploy 反代终结 TLS（`allow_plaintext_bind`）。

3. **引导码登录**（每次 hub 重启后会话失效，需重新引导）：

   ```sh
   # Dokploy 容器控制台执行：
   conex-host private-bootstrap --config /etc/conex/host.toml
   # stdout 打印 43 位引导码，5 分钟有效、单次消费
   ```

   打开 https://conex.lszio.space 输入引导码即进入工作台。引导码不进 URL/日志，库中只存哈希。

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
- 直连入口 `127.0.0.1:7451`（仅 loopback 明文；跨机直连需 TLS + `allow_remote_direct`，归 P9）。
- 节点重启 = 新 epoch：所有旧 grant 失效（预期），浏览器刷新页面即恢复。

节点健康：

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:7451/wss   # 400（入口在，需升级头）
echo node/snapshot | 私有 socket ~/.conex/home-node/data/operator.sock  # 实例/模块/代际
```

## 3. 首次使用清单

1. push dev → Dokploy 构建完成。
2. Dokploy 设置 `CONEX_HUB_SIGNING_KEY`，重启应用。
3. 容器控制台生成引导码 → 浏览器登录。
4. 本机 `~/.conex/home-node/start.sh` 启动节点（保持运行；建议 `nohup … &` 或后续配 launchd）。
5. 工作台出现节点身份与"直连"徽标 → 选端点/Workspace → 新建会话发消息。

## 4. 边界与回退

- 本部署替换了原访客演示（`docker/host.toml` 的 `[web_guest]` 已移除）。回退需要恢复旧配置并重建镜像，属显式动作。
- 撤权/轮换/故障排查见 [P5 手册](private-network.md)；撤销 owner 主体会同时注销全部浏览器会话。
- 已知边界：Chrome 的 Private Network Access 预检未来强制时，跨机 https→loopback 直连需节点补 PNA 头（P9）；事件流为 800ms 有界轮询（直连入口无推送通道）。
