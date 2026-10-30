# S2 插件运行时实施与真实进程验证记录

> 历史记录：下列2026-10-19结果对应当时连接绑定实现。MVP-P2 已把默认 Instance 改为本地业务/发布分离；旧“断开即 Broken 并清理业务”的结论已失效。当前契约见 [S2](../contracts/s2-plugins.md)，重新执行及真实中心重启证据见 [P2](private-runtime-p2.md)。停止后的不复活仍成立，不能把“停止”与“链路暂断”混为一谈。

## 范围

本记录对应 `docs/plans/2026-10-19-conex-s2.md` §7（U04 收口）。U01（TS 插件运行时）、U02（Rust 插件运行时）、U03（双实例样例）已就绪后，本阶段交付 `cargo xtask e2e --suite s2-plugins` 验收套件、总门禁接入与文档回填。S2 不新增 wire 向量（零 wire 变更）；P0/P1/S1 既有门禁全部保留。

## 自动验收（2026-10-19）

实际执行：

```sh
cargo xtask e2e --suite s2-plugins
```

退出码 0。连续多次重复运行均稳定全绿（含修复过程中的验证运行），单次约 2–3 秒。套件驱动真实三进程（临时端口的 conex-host + Rust 实例 `target/debug/examples/plugin` + TS 实例 `bun run examples/s2/plugin-node.ts`），另按场景拉起 TS 插件变体进程（A04/A05 拒绝路径、A08 新代次）与一个真实 SDK 消费 probe（独立连接，负责跨实例 read/add/subscribe 与陈旧引用调用）。屏障只来自中心状态轮询（`module/list`）与样本 stdout 的显式 phase 事件；任何退出路径 kill/wait 全部子进程并删除临时目录（运行后 `pgrep` 确认无残留进程）。

实际输出（预期 = 契约 §8 断言，实际 = 真实观测）：

| 场景 | expected | actual（逐字观测） |
|---|---|---|
| S2-A04 | missing_dependency{module:a,dependency:ghost}, zero sockets, zero publications | kind=missing_dependency module=a dependency=ghost sockets=0 registry unchanged, exit 0 ok |
| S2-A05 | circular_dependency naming a and b, zero sockets | kind=circular_dependency cycle=["a", "b"] sockets=0, exit 0 ok |
| S2-A02 | derived view = ×2 of own counter via in-process handle; own revision sequence | snapshot 0/rev1; add(+5) -> 5/rev1; derived update 10/rev2; read 10/rev2 ok |
| S2-A03 | add via TS derived forwards across instances to the bound Rust counter | bind snapshot 5/rev1; add(+3) forwarded 8/rev2; upstream read 8/rev2; derived view 16/rev2; rust observed 8/rev2 ok |
| S2-A07 | bidirectional derived consumption read+subscribe; equal view values, independent revisions | rust: snapshot 16/rev2, forwarded 12/rev3, update 24/rev3; ts: snapshot 24/rev4, update 28/rev5; probe: rust-derived 28/rev5 vs ts-derived 28/rev4 ok |
| S2-A01 | two instances side by side; independent counts and revision sequences | rust observed ts 0/rev0 and its own overflow add rejected (-32602); probe: ts 0/rev0 -> add(+1) -> 1/rev1 while rust stays 14/rev4 ok |
| S2-A06 | stop unpublishes, ends subscriptions with unavailable, rejects calls; survivor unaffected | ts exit 0 after stopped; subscription end -32005; invoke -32002 not registered; list lacks s2-ts-*; rust survivor 14/rev4 then add 15/rev5, exit 0 ok |
| S2-A08 | no auto-revival; new instance = new generation; old ModuleRef explicitly stale | absent across 5 polls; new registration != old; old ref -32013; fresh read 7/rev1; stopped cleanly, registry empty ok |

证据层说明：

- A02 的「零网络帧」是结构性质（`deps.local` 返回进程内句柄，路径上不存在信封/超时/取消，契约 §5）；e2e 只能观察 ×2 视图跟进，逐帧计数由 S2 单测覆盖（`sdk/typescript/tests/s2_plugin.test.ts`、`crates/conex-sdk/tests/s2_plugin.rs`，随总门禁的 `bun test` / `cargo test --workspace` 执行）。
- A04/A05 为单测 + e2e 双覆盖：e2e 以 TS 插件变体进程触发（类型化 `PluginCompositeError`、进程内 socket 计数为 0、host 注册表前后不变）；Rust 侧组合拒绝同语义由 U02 单测覆盖，不另起 Rust 变体进程（避免在 crates/ 下新增 e2e 专用示例）。
- A07 的 probe 读数（两派生模块同为 28 但 revision 分别为 5 与 4）来自独立消费者连接，等值不同序列即「自有 revision 序列、不继承上游」的跨进程证据。
- A06 的订阅 end 错误码 -32005（unavailable）与 S1「提供端失联/撤销 → unavailable」一致；停止后调用得 -32002（forbidden，module is not registered）是注册表无活动记录的预期路径。
- A08 先以 5 次 ×100ms 轮询做「不复活」负观察，再以同插件类新实例验证：新 registrationId ≠ 旧、旧引用调用 -32013（stale_revision）、新实例读数 7/rev1（全新状态）、干净停止后注册表再次为空。

## 编排确定性

驱动控制样本启动顺序：Rust 实例先发布（中心 list 屏障），其 A02 阶段提交 +5 之后才启动 TS 实例，使 TS 的绑定快照确定为 5/rev1；此后样本经中心计数互为屏障自动推进。样本自身的相对门（`untilCount`、`awaitCount`）对并发提交宽容；驱动断言均取自屏障后的确定性读数或结构不变量（×2 视图、独立序列、退出码）。

## 总门禁

实际执行 `cargo xtask check`（已含 `S2 plugins (real host + plugin runtimes)` 步骤，置于 S1 套件之后）：**退出码 0，`xtask check: all steps passed`**；未删除任何既有 P0/P1/S1 步骤。门禁内的 S2 套件再次输出 S2-A01…A08 全部 `ok`（输出与上表逐字一致）。

首次门禁在 fmt 步骤失败：`cargo fmt --all -- --check` 指向 U03 样例 `crates/conex-sdk/examples/plugin.rs` 的既有格式债（非本切片文件）。作为集成所有者执行 `cargo fmt --all`（纯格式化，无语义变更）后复跑门禁通过。

| 门禁步骤 | 结果 |
|---|---|
| fmt / clippy `--workspace --all-targets --locked -- -D warnings` | 退出 0，无新增豁免（`cargo fmt --all` 一并修复样例文件格式债） |
| `cargo test --workspace --locked` | 582 passed、0 failed（含 `conex-sdk` 的 `s2_plugin` 集成测试） |
| bun frozen install / typecheck | 退出 0 |
| `bun test sdk/typescript/tests` | 148 pass、0 fail（150 项，含 2 个既有 skip；含 U01 `s2_plugin.test.ts`） |
| `generate --check` | 生成物无漂移（零 wire 变更，无新 shared vectors） |
| P0 / P1 / S1 conformance 与 e2e | 全部通过，未改动 |
| S2 plugins e2e | S2-A01…A08 全部 ok |

提交前执行 `node .gitnexus/run.cjs detect-changes --scope all --repo .`：完整结果（无 error/partial/truncated 标记），12 文件、27 个映射符号、8 条受影响流程，risk=high。改动集中在 SDK 导出面（U01/U02）、双实例样例（U03）、验收驱动与文档（U04）；risk=high 是变更图谱信号，不解读为零风险，跨进程行为以上述真实 e2e 证据为准。未获授权不提交不推送。

## 遗留与边界

- S2 契约 §1 的非目标保持：不做插件打包/安装器、不做透明重绑定、不加 wire 字段；实例 `Broken`（连接意外断开）路径与优雅停止共用同一清理，e2e 以优雅停止为观察面（断线复活由 A08 的负观察窗口与单测覆盖）。
- 样本在 e2e 速度下的并发窗口内，「溢出后自身计数不变」的绝对读数不可与对端并发提交区分（对端 +3 合法推进 revision）；e2e 改为断言溢出拒绝事件本身（-32602）与样本的失败即退出口径，绝对值隔离证据由 probe 侧（0/rev0 → +1 → 1/rev1，对侧不受影响）承担。样例 README 的慢速人工编排表仍展示 5/rev1 读数。
