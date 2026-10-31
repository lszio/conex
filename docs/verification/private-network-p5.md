# P5 私有访问与网络 — 实施记录

日期:2026-10-31。范围:[P5 计划](../plans/2026-10-29-conex-mvp-p5.md)的 P5.1–P5.5 全部五个子包,运行平台 macOS(darwin/arm64)与本轮 e2e 为 loopback 明文;真实 TLS/公网入口与两机验收仍属 P9(计划 §5/MVP-P5 运行条款本身即如此规定,不以本机网络替代公网)。

## 1. 交付内容(源码观察 + 运行验证混合;逐项标注)

| 交付物 | 位置 | 证据类型 |
|---|---|---|
| 授权结构唯一源 | `schema/conex/runtime_access.proto`(`AccessGrantScope/Request/Response/Context`、撤权分页、`AccessRevoke*`、`AccessTicket*`) | 源码观察;生成链 `cargo xtask generate --check` 通过 |
| 共享 grant 引擎 | `crates/conex-access/`(自 `conex-host` 抽出,host/agent 共用,避免安全代码双份) | 运行验证:8 个单元测试(签发/验签往返、RFC 8032 §7.1 TEST 2 固定向量、独立签名器交叉、篡改/重复键/未知 claim/时间越界/超用途上限拒绝) |
| 中心私钥签发 | `conex-host/src/grants.rs`(re-export shim)、`private_access.rs` | 运行验证(经 e2e 全链路) |
| 中心私有存储 | `conex-host/src/private_store.rs`:专用 DB 线程 + 有界队列 64、WAL/FULL/外键、user_version、bootstrap 码只存哈希、grant ledger 只存 jti/主体/范围、撤权单调 revision | 运行验证:金丝雀扫描中心库/日志无引导码明文、grant token、客户端凭据 |
| 私有入口 | `POST /private/session|grant|revoke`(cookie+Origin+CSRF 与专用 bearer 双路径);`conex-host private-bootstrap` 一次性 256bit 引导码 | 运行验证:e2e N01/N02/N04/N05/N16 |
| 节点验证与授权 | `conex-agent/src/access.rs::authorize_grant`(tenant/audience/epoch/generation/purpose/methods/资源维度精确交集)+ 每 5 秒 revision 撤权同步(`runtime.rs::sync_hub_revocations`,缺口回卷全量,应用幂等) | 运行验证:e2e N07/N08/N11/N12/N15 |
| 节点直连入口 | `conex-agent/src/direct_api.rs`:`POST /private/tickets`(30 秒一次性、容量 4096 满则拒签)、WSS 子协议 `conex-ticket.<opaque>` + 实际 profile、native bearer 直升、每操作重查 | 运行验证:e2e 票据单次性/回显禁止/错误 Origin/N08 |
| 撤权持久化 | `conex-agent-domain` v3 迁移:`access_revocations(source hub/local, revision, kind, target)` + `access_sync_watermark`;节点重启保留主体/kid 撤销与水位 | 运行验证:domain 27 测试全绿 + e2e N11/N12 跨重启 |
| 控制方法 | `access/node-identity`、`access/revocations/sync`(常量在 `conex_proto::capability`,不进 S1_METHODS;broker `access/*` 分派限非 UI 角色) | 运行验证:e2e 身份注册、epoch 绑定 |
| e2e 套件 | `xtask/src/private_network_e2e.rs`,门禁 `cargo xtask check` 已纳入 `private-network` | 运行验证:见 §2 |

## 2. 实际运行结果

`cargo xtask e2e --suite private-network`(**2026-10-31 通过**,真实进程:1 hub + 1 node + HTTP/WSS 客户端):

- 私有登录:引导码单次消费、错误码 401、缺 Origin 401;会话 cookie+CSRF 建立。
- grant:cookie 路径与 native bearer 路径均签发;匿名 401;非管理员 revoke 403。
- 直连票据:批准 Origin + direct grant 签发 30 秒票据;错误 Origin 403;relay grant 403(N08);匿名 401;同票据二次升级失败(N04);服务端只回实际 profile,票据值零回显(断言)。
- 直连调用:direct grant `counter/read` 成功;read-only scope 下 `counter/add` -32002(N07,精确匹配不放行)。
- 撤权:jti 撤销先持久(revision 非零)后回执;节点 ≤5 秒同步后新调用/升级被拒(N11);principal 撤销跨节点重启仍生效(N12 本地持久)。
- 中心重启:节点以同一 epoch 重连重注册,新签 grant 绑定同 epoch,直连恢复(N14);浏览器会话按契约失效,重新引导码登录。
- 节点重启:epoch 更新,旧 epoch grant 全部拒绝(N15);新身份重新签发可用。
- 金丝雀:引导码明文、grant token、客户端凭据在工作目录全部文件(除签名密钥文件本身)扫描无命中。

相关单元/回归:`cargo test -p conex-access`(8)、`conex-host --lib`(63)、`conex-agent`(39)、`conex-agent-domain`(27)、`conex-sdk`(22)、`cargo xtask e2e --suite s2-plugins`(8 场景 ok)。

## 3. 实施中修正的真实缺陷

- **pkcs8 版本字节**:手工构造 Ed25519 PKCS#8 信封时 version 必须为 1(RFC 5958 v2),`00` 被 ring 拒绝;测试信封按 ring `generate_pkcs8` 布局(`[1] IMPLICIT` tag 0x81)对齐。
- **平台可移植性**:`process_identity/known_exited` 原依赖 Linux `/proc`;macOS 上 P2 的受控 worker 无法启动。改为 `sysinfo`(安全 API,节点 crate 保持 `#![forbid(unsafe_code)]`)读取进程启动时间 + 系统启动时间,语义不变(同一 (boot, start) 对)。
- **direct-only 节点的发布链**:P2 的发布循环在无导出/远程绑定时不开链;私有节点必须始终建链(身份注册与撤权同步的唯一通道),`always_link` 在 `[private]` 存在时置位。
- **发布链角色**:节点发布 token 若用 `agent` 角色会被"agent 链仅限 register/heartbeat"网关拦截;私有发布链使用 `service` 角色(与 P2 探针一致)。
- **身份注册竞态**:hub 重启后首次 `access/node-identity` 可能因中心仍在装配被拒;钩子在同一活链上以 500ms 重试至多 15 秒,不依赖下一次断连。

## 4. 明确未交付(不因本页关闭)

- 真实 TLS/HTTPS 与证书策略、公网入口→两私网节点、真实浏览器私网策略:按计划归 P9。
- P5.4 中依赖 Agent 后端的项(取消定位原执行、unknown 语义与执行的交互)随 P3/P4 落地。
- TS SDK 的 direct/relay 选择与浏览器票据子协议客户端(计划 P5 文件表内):服务端能力已就绪,e2e 以 native 消费者验证;TS 绑定在 P7 Web 工作台接入时补齐并另记证据。
- `cargo xtask e2e --suite private-runtime` 的 TS 探针在 macOS 因其自身 `/proc` 读取失败(先于本轮存在,P2 证据产生于 Linux);本轮 Rust 侧进程身份已平台化,TS 探针平台化未做。
