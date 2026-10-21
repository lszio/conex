# S2 插件运行时运行手册

## 支持范围与安全边界

- 插件是 **SDK 侧进程内的组合单元**：一个接入实例 = 一条 S1 连接；同一插件类可创建多个实例，每实例独立连接、独立模块注册、独立状态序列。中心（conex-host）零改动。
- 本地依赖是进程内句柄（组合期解析，不经网络）；显式远程绑定把依赖名固定映射到另一实例已发布模块的 `ModuleRef`。缺失依赖、循环依赖、重复模块名在启动前拒绝——类型化本地错误，不过网。
- 停止实例 = 逆拓扑序 unpublish → 关闭订阅与消费者 → abort handler → 关闭连接；实例停止/断开后不自动复活，重建是新代次，旧 `ModuleRef` 显式失效（S1 `stale`/`-32013` 族）。
- 仍受 S1 边界约束：静态批准只用于受限开发，**S4 完成前禁止公开多用户使用**；开发配置仅绑定 loopback。令牌只经环境变量注入，进程与日志不输出明文。

## 三进程样例

按 [examples/s2/README.md](../../examples/s2/README.md#运行) 运行。插件设计同款：模块 `counter` + `derived-counter`（契约 `conex.counter`），Rust 实例（`crates/conex-sdk/examples/plugin.rs`）跑本地派生，TS 实例（`examples/s2/plugin-node.ts`）跑显式远程绑定。

| 变量 | Rust 实例（s2-rust） | TS 实例（s2-ts） |
|---|---|---|
| `CONEX_URL` | `ws://127.0.0.1:8789/wss` | 同左 |
| `CONEX_TOKEN` | Rust 专属批准令牌 | TS 专属批准令牌 |
| `CONEX_SPACE` | `dev` | `dev` |
| `CONEX_ENDPOINT` | `s2-rust` | `s2-ts` |
| `CONEX_PEER_ENDPOINT` | `s2-ts` | `s2-rust` |

启动顺序：先 Host（`cargo run -p conex-host -- examples/s2/host.toml`），再 Rust 实例，再 TS 实例。两个实例经中心状态量互为屏障，无固定等待；各阶段输出结构化 JSON（`start`/`publish`/`bind-snapshot`/`snapshot`/`add`/`read`/`update`/`phase-done`/`idle` 等）。`idle` 后实例驻留提供服务，直到收到 SIGINT/SIGTERM。

停止语义：先停一个实例（SIGTERM → 逆序 unpublish → `stopped` → 退出码 0），另一实例观察到 `peer-gone` 后继续服务自己的模块（`read`/`add` 仍可用），随后自行 `stop()` 退出。Host 最后 Ctrl-C；持久根 `examples/s2/.runtime/` 可整目录删除重建。

## 自动验收

```sh
cargo xtask e2e --suite s2-plugins
cargo xtask check        # 已包含上一行
```

suite 生成临时端口、令牌摘要与数据目录，启动真实 Host + Rust 实例 + TS 实例三进程，另有两个 TS 插件变体进程（组合期拒绝、新代次）和一个真实 SDK 消费 probe。屏障只来自中心状态轮询（`module/list`）与样本 stdout 的显式 phase 事件；A08 的唯一 sleep 是「不复活」的负观察窗口。输出 S2-A01…A08 的预期/实际，任一失败非零退出；成功与失败都 kill/wait 全部子进程并回收临时目录。

覆盖与证据层（契约 §8）：

| 场景 | 证据 |
|---|---|
| A01 双实例并存隔离 | 样本事件（对端 0/rev0、溢出拒绝）+ probe 独立读写 |
| A02 本地依赖派生 | Rust 实例事件（×2 视图、自有 revision）；零网络帧的结构性质由 S2 单测覆盖 |
| A03 显式远程绑定 | TS 实例事件（bind 快照、转发 add）+ Rust 实例观测 |
| A04 缺失依赖拒绝 | 插件变体进程：`missing_dependency`、零 socket、注册表不变；Rust 侧同语义在 `crates/conex-sdk/tests/s2_plugin.rs` |
| A05 循环依赖拒绝 | 插件变体进程：`circular_dependency` 带环节点、零 socket；Rust 侧同上 |
| A06 停止释放 | SIGTERM → `stopped` 退出 0、订阅 end（-32005）、调用 -32002 未注册、list 缺失、幸存实例继续服务 |
| A07 派生模块跨语言 | 双向样本事件 + probe 独立 read（28/rev5 vs 28/rev4：等值不同序列） |
| A08 断开不复活/新代次 | 负观察窗口 + 新 registrationId ≠ 旧 + 旧 ref -32013 + 新实例全新状态 |

## 排障

- `missing_dependency` / `circular_dependency`：检查模块图声明（`depsLocal`/`depsRemote` 与 `remotes` 绑定表）；拒绝发生在任何网络帧之前，不存在半连接。
- 绑定失效（-32013 `stale_revision`）：实例换代后旧 `ModuleRef` 不再有效，重新 `module/list` 发现并重新配置绑定；不要试图复用旧引用。
- 停止后调用 -32002：模块已随实例注销，属预期；需要继续消费请启动新实例。
- 端点冲突（-32014）：同 endpoint 的旧实例未退出时新实例发布会冲突；先确认旧进程已终止。
- 样本 `error` 事件 / 非零退出：对照 phase 事件定位阶段；套件失败时错误信息带场景号与最后一个观测。

实际执行结果见 [S2 验证记录](../verification/s2-plugins.md)。
