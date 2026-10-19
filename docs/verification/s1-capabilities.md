# S1 实施与真实进程验证记录

## 范围

本记录对应 `docs/plans/2026-10-17-conex-s1.md`。T01–T07 的三进程 smoke 见下文；T08 已增加真实 Host/TS/Rust 的 A01–A12 驱动及明确标注的 SUPPORT 行为回归。S1 使用 JSON WSS 与受限 HTTP binding，不代表 S2–S5 已完成。

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

## T01–T07 Rust 与生成物回归

该阶段执行 `cargo test --workspace --no-fail-fast`：退出 0，462 passed，76 suites。新增真实 socket 回归覆盖 S1 bootstrap、降低限额后的帧拒绝，以及 JSON/protobuf 提供端错误元数据到消费者的完整传递。

`cargo fmt --all -- --check` 与 `cargo xtask generate --check` 均退出 0；生成物与 schema 一致。

## T08 自动验收（2026-10-19）

实际执行：

```sh
cargo test -p conex-host --test s1_state terminal_and_disconnect_release_provider_observers
cargo xtask e2e --suite s1-capabilities
```

两条命令均退出 0。清理回归运行 1 项、0 失败。S1 suite 输出全部 12 个场景 `ok`；临时配置/令牌摘要/目录由驱动生成，仅连 loopback。Fixture Drop 在 ready 失败时同样 kill/wait 并 join JSONL reader；Host 启动失败也 kill/wait，不留下子进程。

| 场景 | 实际观测 | 证据层 |
|---|---|---|
| A01 | TS→Rust、Rust→TS 均 read=0、add(3)=3、read=3，独立域各为 3 | 真实进程 |
| A02 | 挂接屏障前 snapshot count=5/rev=2，窗口内提交后 update seq=1/count=10/rev=3 | 真实进程 |
| A03 | 旧注册调用 -32013；关闭旧 fixture 后，新 registrationId 仍可发现 | 真实进程 |
| A04 | 两条链重叠使用相同 wire request ID，分别返回先后提交值；伪造主体收到 -32602 边界错误；未批准调用 -32002，两提供端无额外提交 | 真实进程与原子执行次数观察 |
| A05 | 提交前取消得到 -32011/not_started/never，count/执行次数不变；排队预算耗尽未发送业务帧 | 真实进程 + `queued_budget_exhaustion_settles_not_started_and_never_sends` SUPPORT |
| A06 | 提交后屏障收到取消 requested=true，释放后原 invoke 成功，执行次数仅 +1 | 真实进程 |
| A07 | 已确认提交 count=20 后消费者断线，原调用 -32012/unknown/never；重连只读到该次提交，没有重放 | 真实进程 |
| A08 | 提供端关闭以 -32005 结束，换代后新订阅收到 snapshot；撤销注册同样终结 | 真实进程 + `registration_end_ends_subscriptions` SUPPORT |
| A09 | 活动订阅期间另一真实链仍读到 count=0；数据水位耗尽时控制预留可发送 slow_consumer，控制空间耗尽则关闭消费者；快照灰窗超限仍先快照后 end | 真实独立链 + `control_reserve_and_full_control_space_close_consumer`、`grey_window_overflow_delivers_end_after_snapshot` SUPPORT |
| A10 | 序列缺口、revision 回退、非法 quality 均终结为 -32017；未受影响订阅保持 seq=1；伪造 ID 不影响新订阅；重复序列和迟到快照不复活 | 真实故障字节注入 + `active_seq_must_start_at_one_and_be_contiguous`、`opening_timeout_cleans_up_and_late_snapshot_does_not_revive` SUPPORT |
| A11 | 真实 HTTP binding list/read/cancel，取消后仍保留一次 add(2) 提交；subscribe/register 均 -32004；Rust 9 与 TS 107 项 S1 向量通过 | 真实 HTTP→WSS + shared vectors |
| A12 | int32 溢出 -32602/not_started/never 且域不变；已提交后损坏输出 -32012/unknown/never，执行仅 +1；消费者 snapshot/update 精确保留 9007199254740993→9007199254740994 | 真实进程 |

SUPPORT 使用真实 Broker/StateRelay 与受控内存链、队列水位，不声称模拟了操作系统 TCP 缓冲压力或完成性能压测。命名支持用例必须实际输出对应 `test ... ok`，过滤到零项不会被算作成功。

本轮真实失败暴露并修复：

- 非法 sample.quality 被 JSON WSS 总闸拒绝后，订阅未结束。现在只在通用 wire/身份/context 校验通过后，将异常 state/update 交给所有权优先的中继校验；只结束可归属订阅，未放松身份边界。
- terminate 先删除 hop、再查 hop 发 unsubscribe，导致上游观察者残留；消费者断线也未释放上游。现在先原子移除并取得固定代次清理记录，再在锁外发 unsubscribe。新增回归验证准确 hop、兄弟隔离、断线重复清理不重复发送。
- 纯 S1 Host 没有旧 endpoints，HTTP capability 表未为 token tenant 建表；HTTP hello 还只返回四项限额。现在为静态主体 tenant 宣告真实可用 HTTP 方法并返回六项正整数限额，业务批准仍独立执行；HTTP 不广告发布/订阅支持。
- 总门禁暴露 A05 驱动的取消竞态：requested=true 只表示 Host 受理，立即释放提交屏障可能让尚未观察取消的 handler 合法提交。fixture 现在同时等待真实取消信号与提交屏障；驱动保持提交阻塞，等待原 invoke 确定停止，不以 cancel 响应替代结算。
- 总门禁 Clippy 检查暴露 Rust SDK 内联生成 Error 至少 160 字节；改为 `Box<conex_proto::Error>`，仅错误路径分配，迁移全部构造点，保留 code/execution/retry/details。私有状态投递资源按所有权打包，其余 lint 修正不改变协议行为；没有增加 lint 豁免。

## T09 总门禁

最终实际执行 `cargo fmt --all && cargo xtask check`，退出 0，输出 `xtask check: all steps passed`。完整门禁中的真实 S1 suite 再次输出 A01–A12 全部 `ok`，包括修正握手后的 A05。

| 门禁 | 最终结果 |
|---|---|
| fmt / Clippy `--workspace --all-targets --locked -- -D warnings` | 退出 0，无新增豁免 |
| Rust workspace tests | 全部通过；包含 queued cancel、预算耗尽、注册换代、上游观察者释放与 SDK 取消/未知结果回归 |
| Bun frozen install / SDK+Web typecheck | 退出 0 |
| TS tests | 137 pass、2 个既有 skip、0 fail；439 expect |
| generated parity | up to date |
| P0 / P1 shared conformance | Rust/TS 一致，退出 0 |
| S1 shared conformance | Rust 9 pass，TS 107 pass，0 fail |
| P0 real e2e | 1 pass、0 fail |
| S1 real e2e + SUPPORT | A01–A12 全部 ok；支持用例实际运行，不是零项过滤通过 |

`xtask check` 新增 S1 向量与 suite，未删除既有 P0/P1 步骤；CI 仍通过同一入口，不复制额外完整 suite。Rust SDK 依赖仅 proto/运行时/序列化/WebSocket，不依赖 Host；SDK 通用入口不要求 hello 工作台。运行手册和两级 README 已说明静态批准、JSON WSS 共同支持、HTTP 无订阅，以及 S4 前禁止公开多用户使用。

提交前执行 GitNexus `detect_changes --scope all` 并读取原始完整结果：26 文件、299 个映射符号、84 条受影响流程，risk=critical；返回无 error/partial/truncated 标记。改动位于 Host、SDK、验收驱动与相关文档，不把图谱结果解释为零风险；未映射的 TS fixture 由实际跨进程路径补证。

T01–T07 提交为 `d504a97`；T08/T09 代码、运行手册及本记录随本次收口提交。未发布版本、未部署公开服务；S2–S5 不在本次交付范围。
