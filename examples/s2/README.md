# S2 双实例插件样例（U03）

两个进程各承载一个插件接入实例（一实例 = 一条 S1 连接），同一插件设计
`counter-plugin`（模块 `counter` + `derived-counter`，契约 `conex.counter`），
不同 endpoint 前缀与身份。中心（conex-host）零改动；所有组合语义来自
SDK 内的插件运行时（TS `definePlugin`/`Instance`，Rust `Plugin`/`Instance`），
见 [S2 契约](../../docs/contracts/s2-plugins.md) §5–§7。

## 文件

| 文件 | 作用 |
|---|---|
| `host.toml` | 开发专用静态批准：两个 service token、4 个发布批准（每实例 counter/derived-counter）、8 个消费批准、`conex.counter` 契约描述 |
| `plugin-node.ts` | TS 实例 B：`counter` + `derived-counter`（**远程模式**，显式绑定对端 counter 的 `ModuleRef`） |
| `crates/conex-sdk/examples/plugin.rs` | Rust 实例 A：`counter` + `derived-counter`（**本地模式**，`deps.local` 进程内句柄） |

## 模块语义

- `counter`：S1 计数器同款契约语义——`counter/read`、`counter/add`、状态
  `counter/value`；int32 溢出在变更前拒绝（count/revision 不变），
  `delta=0` 仍推进 revision；每实例独立 count/revision。
- `derived-counter`：同一 contract 再发布（自有 4 元组、**自有 revision
  序列**）：上游计数的 ×2 视图；`counter/read` 返回自己的视图，
  `counter/add` **转发给上游**（本地模式 = 进程内调用零网络帧；远程模式 =
  经绑定 invoke），视图经上游观察自行跟进。先快照后发布：拿到上游首个
  快照并提交之后，自身的状态订阅才对外可用（revision 从 1 起）。

两个实例的分工：Rust 实例演示 A02（本地派生零网络），TS 实例演示 A03
（跨实例显式远程绑定）；两者的派生模块都被对端语言消费（A07 双向）。
派生模块的另一种模式分支在两侧 SDK 均已实现，由 S2 单测覆盖
（`sdk/typescript/tests/s2_plugin.test.ts`、`crates/conex-sdk/tests/s2_plugin.rs`）。

## 令牌与摘要

令牌经环境变量 `CONEX_TOKEN` 注入，进程从不输出明文；`host.toml` 只存
摘要（64 位小写十六进制 SHA-256）：

```sh
printf %s "$TOKEN" | sha256sum
```

`host.toml` 的两个摘要来自开发专用明文（**不是秘密，仅限本机开发**）：

| principal_id | endpoint 前缀 | 开发明文 | token_hash |
|---|---|---|---|
| `s2-rust` | `s2-rust` | `conex-s2-dev-token-rust` | `f03397fa…a1c5` |
| `s2-ts` | `s2-ts` | `conex-s2-dev-token-ts` | `1945420a…28e` |

换用自己的令牌：按上式生成摘要替换 `host.toml`，并把明文放进启动环境。

## 运行

前提：`cargo build -p conex-host`、Bun 已安装。`host.toml` 监听
`127.0.0.1:8789`；持久根落在 `examples/s2/.runtime/`（已 gitignore）。所有
命令都从仓库根执行。

终端 1 —— 中心：

```sh
cargo run -p conex-host -- examples/s2/host.toml
```

终端 2 —— Rust 实例 A（counter + derived，本地模式）：

```sh
CONEX_URL=ws://127.0.0.1:8789/wss \
CONEX_TOKEN=conex-s2-dev-token-rust \
CONEX_SPACE=dev \
CONEX_ENDPOINT=s2-rust \
CONEX_PEER_ENDPOINT=s2-ts \
cargo run -p conex-sdk --example plugin
```

终端 3 —— TS 实例 B（counter + derived，远程模式绑定 A 的 counter）：

```sh
CONEX_URL=ws://127.0.0.1:8789/wss \
CONEX_TOKEN=conex-s2-dev-token-ts \
CONEX_SPACE=dev \
CONEX_ENDPOINT=s2-ts \
CONEX_PEER_ENDPOINT=s2-rust \
bun run examples/s2/plugin-node.ts
```

环境变量契约：`CONEX_URL`、`CONEX_TOKEN`、`CONEX_SPACE`、
`CONEX_ENDPOINT`、`CONEX_PEER_ENDPOINT`（后两者是 endpoint **前缀**，模块
endpoint 为 `{前缀}-counter` / `{前缀}-derived-counter`）；集成 id 固定为
`"integration"`。**令牌只经本机环境注入；宿主与节点日志均不含秘密。**

## 期望输出

每个节点每步输出一行结构化 JSON（`event` ∈ `start`/`ready`/`discover`/
`publish`/`bind-snapshot`/`snapshot`/`add`/`read`/`update`/`update-seen`/
`overflow-rejected`/`phase-done`/`idle`/`peer-gone`/`stopping`/`stopped`/
`done`/`error`）。两个实例各自跑完消费演示后输出 `idle` 并驻留。

按默认编排（先起 Rust，再起 TS；各阶段经中心状态量互为屏障，无固定
等待），A 实例计数器的关键读数：

| 步骤 | 动作 | s2-rust-counter | s2-rust-derived-counter（×2） | s2-ts-derived-counter（×2） |
|---|---|---|---|---|
| A02 | Rust add(+5) | 5 / rev 1 | 10 / rev 2（进程内跟进） | — |
| A01 | Rust 读对端 | — | — | TS counter 0 / rev 0 不受影响 |
| A03 | TS 经 B.derived add(+3) | 8 / rev 2（转发生效） | 16 / rev 3 | 16 / rev 2 |
| A07 | Rust 经 B.derived add(+4) | 12 / rev 3 | 24 / rev 4 | 24 / rev 3 |
| A07 | TS 直加 A.counter (+2) | 14 / rev 4 | 28 / rev 5 | 28 / rev 4 |

两个派生模块终值同为 28 但 revision 值不同（5 与 4）——即"自有 revision
序列、不继承上游"的直观证据。`overflow-rejected` 出现在 Rust 侧（对
`s2-rust-counter` add `i32::MAX`，code -32602，随后 read 仍 5 / rev 1）。

A02 的"零网络帧"是结构性质：`deps.local` 返回上游模块 start 返回的进程内
句柄，路径上不存在信封/超时/取消（契约 §5）；线级行为只呈现为派生视图
的 ×2 跟进。逐帧计数由 S2 单测覆盖（订阅事件计数为 0）。

## 停止语义

1. TS 实例 B 收到 SIGINT/SIGTERM：`Instance.stop()` 逆拓扑序 unpublish
   （`s2-ts-derived-counter` → `s2-ts-counter`）→ 关闭订阅/消费者 → 关连接，
   输出 `stopping`/`stopped` 后 **退出码 0**。
2. Rust 实例 A 轮询 `module/list` 观察到对端 endpoint 全部消失，输出
   `peer-gone`，随后对**自己的** counter 继续 read/add（停止隔离：停 A 不
   影响 B，反之亦然），再 `stop()` 自己并 `done` 退出码 0。
3. 最后在终端 1 按 Ctrl-C 停止中心。持久根可整目录删除重建。

实例一旦停止/断开不会复活；同插件类再次实例化是新一代次，旧 `ModuleRef`
显式失效（S1 `stale`/`-32013` 族），由 S2 单测（A06/A08）覆盖。
