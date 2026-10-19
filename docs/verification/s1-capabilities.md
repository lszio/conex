# S1 实施与真实进程验证记录

## 范围

本记录对应 `docs/plans/2026-10-17-conex-s1.md`。已运行真实 Host、TS 节点、Rust 节点三个独立进程；这不是 T08 的 A01–A12 完整故障验收，也不表示 T09 总门禁完成。

## 三进程 smoke（2026-10-19）

先构建：

```sh
cargo build -p conex-host
cargo build -p conex-sdk --example counter
```

启动与环境采用 `examples/s1/README.md`：Host 读取 `examples/s1/host.toml`，监听 `127.0.0.1:8788`；TS 由 Bun 执行 `examples/s1/counter-node.ts`，Rust 执行 `target/debug/examples/counter`。两节点使用各自开发令牌与 endpoint，互相消费对端。令牌只通过进程环境传递。

协调者读取两节点结构化 stdout，等待双方 `done` 后发送 SIGTERM，并等待全部子进程退出；两个节点和 Host 的退出码均为 0。节点消费流程完成后继续提供能力，直到收到显式终止信号，避免快节点提前退出导致慢节点无法发现提供端。

| 实际观察 | TS 消费 Rust | Rust 消费 TS |
|---|---|---|
| ready、发布、发现 | 成功 | 成功 |
| 初始 snapshot | count=0 / revision=0 | count=0 / revision=0 |
| read → add(3) → read | 0 → 3 → 3 | 0 → 3 → 3 |
| 第一次 update | count=3 / revision=1 | count=3 / revision=1 / seq=1 |
| add(0) 后 update | count=3 / revision=2 | count=3 / revision=2 / seq=2 |
| 溢出 add | -32602，int32 overflow | -32602，int32 overflow |
| 溢出后的 read | count=3 / revision=2 | count=3 / revision=2 |
| unsubscribe | removed=true | removed=true |
| 消费流程 | done | done |

最终 smoke 中，两方向的溢出错误均携带 `code=-32602`、`execution=not_started`、`retry=never`。此前实际日志暴露的空字段问题已修复：JSON/protobuf Failure 入站、宿主出站与调用路由保留已验证的 Error 元数据；已发送 non_replayable 调用缺少执行证据时仍保守结算为 outcome_unknown/unknown/never。

## 真实链路暴露并修复的问题

- Host bootstrap 只发送四项限额，未选择 WebSocket 子协议，未处理 SDK 的 generated CallParams 外层，且把中心提供能力错误地要求为节点提供能力。新增 S1 协商分支，保留旧应用连接路径；最终协商限额用于业务帧、队列、并发、模块与订阅上限。
- S1 hello/ready 的 bindingId、ULID LinkIdentity 与认证主体字段未和 SDK 对齐；现在采用生成契约的形状，TS S1 分支同样使用自己的 provides，而非镜像中心列表。
- Protobuf Value 的整数在 JSON 转发后成为 `3.0`，Rust typed int32 解码失败。wire 编码边界递归规范化安全范围内的整数值；小数、超安全范围数值和 uint64 字符串不变。
- TS 提供端把接收到的可信调用者身份原样放入 `state/update`/`state/end`；Host 正确拒绝客户端伪造身份字段。现在出站通知仅携带路由 context，完整身份保留在提供端内部用于取消归属检查。
- opening 退订路径在 if-let 临时 MutexGuard 内再次获取同一锁导致死锁；改为先取 link、释放 guard 后处理。
- Host 转发错误时曾仅重建 code/message，丢失 execution/retry/diagnosticId/details；现在保留完整已验证 wire Error，且没有执行证据时不伪造未开始。

## SDK 回归

实际命令：

```sh
bun test sdk/typescript/tests
bun run --cwd sdk/typescript typecheck
bun run --cwd sdk/typescript build
```

结果：137 pass、2 skip、0 fail，439 expect；typecheck/build 均退出 0。两个 skip 保持现有套件行为，不能作为已验证场景。

## Rust 与生成物回归

最终执行 `cargo test --workspace --no-fail-fast`：退出 0，462 passed，76 suites。新增真实 socket 回归覆盖 S1 bootstrap、降低限额后的帧拒绝，以及 JSON/protobuf 提供端错误元数据到消费者的完整传递。

`cargo fmt --all -- --check` 与 `cargo xtask generate --check` 均退出 0；生成物与 schema 一致。

## 未完成验收

- T08：`cargo xtask e2e --suite s1-capabilities` 自动驱动尚未实现；A01–A12 故障屏障、真实 socket 取消/提交后失联/换代/慢消费者验收未全部运行。
- T09：尚未将 S1 自动 suite 接入 `cargo xtask check`，未执行完整总门禁；不关闭 S1 Story。
- 未提交、未推送、未发布版本。
