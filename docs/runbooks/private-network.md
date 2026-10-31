# 私有网络运行手册(P5)

私有认证、grant、直连与撤权的操作要点。验收证据见 [P5 记录](../verification/private-network-p5.md);公网/两机部署仍属 P9。

## 1. 中心配置

`host.toml` 新增(完整字段见 `conex-host/src/config.rs::PrivateConfig`):

```toml
[private]
issuer = "conex://hub.example"      # grant 的 iss,节点侧 hub_issuer 必须一致
kid = "hub-2026q4"                  # 当前签名密钥 ID
signing_key_backend = "file:/etc/conex/hub-signing.pkcs8"   # env:NAME 亦可;0600
tenant = "private"                  # 私有实例租户,签入每个 grant
store_path = "/var/lib/conex/private.sqlite"                # 中心私有库
principal = "owner"                 # 引导码认证的主体
admin_principals = ["owner"]        # 可调用 /private/revoke
node_principals = ["node-a"]        # 允许声明节点身份的发布凭据主体
```

前置条件与互斥:

- `[private]` 存在即启用;与 `[web_guest]` 互斥,同时出现是配置错误(不留访客后门)。
- 需要 P1 后端齐备(`content_root/session_root/operation_root`),否则启动拒绝。
- 发布凭据 principal 若配了 `role = "agent"` 的 token 会被 agent 链网关限制;节点发布链请使用 `role = "service"`。

## 2. 密钥与轮换

- 中心持 pkcs8 Ed25519 私钥;节点只装原始公钥(64 hex)。生成示例(在获准环境执行):

```sh
openssl genpkey -algorithm ed25519 -out /etc/conex/hub-signing.pkcs8
openssl pkey -in /etc/conex/hub-signing.pkcs8 -pubout \
  | openssl pkey -pubin -outform DER | tail -c 32 | xxd -p -c 64   # 节点安装的 32 字节原始公钥
```

- 轮换:先向节点安装新 kid 公钥,中心切新 `kid` 签发,旧钥保留至其最后授权 exp 后 60 秒;紧急时从节点 `approved_public_keys` 删除 kid 立即拒绝。

## 3. 引导码与私有登录

```sh
conex-host private-bootstrap --config /etc/conex/host.toml
# stdout 打印一次 256bit 引导码,5 分钟有效、单次消费;库中只存哈希
```

浏览器 `POST /private/session`(HTTPS,精确 Origin,code 仅在正文)换取私有 cookie+CSRF。中心重启会话失效,需重新引导码登录;不会自动降级为访客。

## 4. grant 与直连

- `POST /private/grant`:cookie+Origin+CSRF 或 `Authorization: Bearer`(非 ui 角色)。正文 `{nodeId, scope{spaceId,providerEndpointId,contractId,moduleGeneration,methods,states,workspaceIds,conversationIds,purpose}}`;正文出现 principal 即拒(未知字段)。
- purpose 上限:relay 5 分钟、direct 1 小时,签发不得超;`moduleGeneration` 必须是目标模块当前代际。
- 节点直连:浏览器 `POST <node>/private/tickets`(Bearer direct grant + 批准 Origin)→ 30 秒一次性票据 → WSS 子协议 `conex-ticket.<票据>` + 实际 profile;native/Emacs 直接 `Authorization: Bearer` 升级。票据不进 URL/日志。
- 每次调用/读取都在节点重查:过期、tenant/audience/epoch/代际、purpose=路径、方法与资源在 grant∩节点规则交集、本地与 hub 撤权。

## 5. 撤权

```sh
curl -X POST https://hub.example/private/revoke \
  -H "Authorization: Bearer <管理员凭据>" \
  -d '{"kind":"principal","target":"owner"}'    # kind: jti | principal | kid
```

- 先持久提交(返回单调 revision)再生效;jti/principal/kid 三类,节点按 revision 分页同步(≤5 秒),同 revision 幂等,缺口回卷全量。
- 中心分区期间有效直连可继续至 grant exp;本地已知撤销立即执行。
- 按主体撤销同时注销中心该主体的全部浏览器会话。

## 6. 故障排查

| 现象 | 处置 |
|---|---|
| grant 签发 403 "node is not registered" | 节点发布链未注册身份:检查节点 `[private]` 配置、发布凭据主体是否在 `node_principals`、发布 token 角色是否 `service` |
| 直连票据 403 | Origin 不在 `approved_web_origins`、grant purpose 非 direct、grant 已过期/撤销 |
| 升级 401 "ticket … already used" | 票据单次消费;重新取票 |
| 节点重启后旧 grant 失效 | 预期行为:epoch 更新,须重新申请 grant |
| 中心重启后会话失效 | 预期行为:重新 private-bootstrap 登录 |

## 7. 验收

```sh
cargo xtask e2e --suite private-network   # loopback 全场景
cargo xtask check                          # 总门禁(含本套件)
```
