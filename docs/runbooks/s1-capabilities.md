# S1 双向能力运行手册

## 支持范围与安全边界

- TS/JS SDK 与独立 Rust `conex-sdk` 在同一条 JSON WSS 连接上提供和消费预安装模块契约；SDK 不依赖 hello 工作台或 Agent 守护进程。
- 中心完成认证、静态发布/消费批准、目录和路由；注册不能安装 schema，也不能授予权限。提供端还必须执行本地授权上限。
- HTTP binding 仅支持 `module/list`、`module/invoke`、`module/cancel`，不支持模块发布与状态订阅。状态必须使用 WSS。
- S1 不是 S4 身份治理。**S4 完成前禁止公开多用户使用**；开发配置仅绑定 loopback，禁止把示例令牌用于公网服务。远程连接使用 TLS，bearer 只进入 Authorization，不放 URL。
- 本切片不提供 P2P、插件运行时、OIDC 用户管理或 Astro 工作台；已有文件/工作台 API 不因此删除。

## 三进程样例

按 [完整启动步骤](../../examples/s1/README.md#运行) 分别运行 Host、TS 和 Rust 节点。三个进程的命令、开发令牌摘要和退出行为以该样例为准。

| 变量 | TS 节点 | Rust 节点 |
|---|---|---|
| `CONEX_URL` | `ws://127.0.0.1:8788/wss` | 同左 |
| `CONEX_TOKEN` | TS 专属批准令牌 | Rust 专属批准令牌 |
| `CONEX_SPACE` | `dev` | `dev` |
| `CONEX_ENDPOINT` | `counter-ts` | `counter-rust` |
| `CONEX_PEER_ENDPOINT` | `counter-rust` | `counter-ts` |

中心读取 `examples/s1/host.toml`，只保存 SHA-256 令牌摘要。更换令牌时同步修改摘要与节点环境；不要将任意环境变量传给远端执行器。节点不输出明文凭据。

观察两个节点各自的 `publish`、`discover`、`snapshot`、`read`、`add`、`update-seen` 和 `done`。两方向都应读到自己的独立计数器 `0 → 3 → 3`；`add(0)` 仍推进 revision。`done` 只结束消费演示，提供端继续服务。双方完成后向节点发 SIGTERM/Ctrl-C，再停止 Host。

## 取消、失联和状态过期

`Invocation.cancel()` 返回 `{requested:true}` 只表示停止请求被受理；必须继续观察原 `result`。已确定提交的成功不会因稍后取消而回滚。

非幂等调用可能已发送但结果无法确认时，错误为 `-32012`、`execution=unknown`、`retry=never`。不能因超时、断线、格式错误或重连自动重发。只读错误中的 `retry=safe` 也是调用方提示，不是 SDK 自动重试开关。

Rust 使用 `Error::new(...)` 构造 SDK 错误；完整生成负载存于 `Error.0: Box<conex_proto::Error>`，`code()`、`execution()`、`retry()` 读取语义字段。装箱仅发生在错误路径，避免放大所有成功 `Result` 的布局；不改变线上 Error 类型。

订阅先交付 snapshot，再交付 seq 从 `"1"` 连续递增的完整 sample。revision/seq/sampledAtMs 是 uint64 十进制字符串，不要转 Number。失序结束为 `session_lost`；缓冲超限结束为 `slow_consumer`。保留最后值的应用必须将其标记过期。重连后重新发现目标、发布必要模块、新建订阅；不恢复旧 subscriptionId 或补播旧更新。

## 自动验收与排障

```sh
cargo xtask e2e --suite s1-capabilities
cargo xtask conformance --vectors conformance/vectors/s1
cargo xtask check
```

S1 suite 生成临时端口、主体、令牌摘要和数据目录，启动真实 Host/TS/Rust 子进程，使用仅存在于测试进程的提交/快照屏障。输出 A01–A12 的预期和实际值；行为回归证据标记为 SUPPORT。任一失败非零退出，成功与失败都 kill/wait 子进程并回收临时配置。不要用固定 sleep 或缩减场景掩盖失败。

- `forbidden`：核对主体、space、endpoint、contract、发布/消费批准及提供端本地策略；不要绕过批准。
- `stale_revision`：旧注册引用已失效，重新发现；旧连接 cleanup 不能删掉新注册。
- 协商失败：双方必须支持 S1 方法及六项正整数限额；不要退回文件或 hello 方法。
- 缓冲超限：检查消费速度与受限队列；不得静默丢帧并假装序列连续。

实际执行结果见 [S1 验证记录](../verification/s1-capabilities.md)。
