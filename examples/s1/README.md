# S1 双计数器样例（T07）

两个独立进程提供并消费同一预安装契约 `conex.counter`：TS 节点
`counter-node.ts`（Bun）与 Rust 节点 `crates/conex-sdk/examples/counter.rs`
（cargo example）。二者互不依赖，只经中心（conex-host）路由。

## 文件

| 文件 | 作用 |
|---|---|
| `host.toml` | 开发专用静态批准：两个 service token、发布/消费批准、`conex.counter` 契约描述 |
| `counter-node.ts` | TS 节点：提供 `counter/read`、`counter/add`、状态 `counter/value`，消费对端 |
| `crates/conex-sdk/examples/counter.rs` | Rust 节点：同一行为，仅用 conex-sdk |

两个节点都满足：`add` 在 int32 越界前拒绝（count/revision 不变；
`delta=0` 仍递增 revision）；修改与状态发布共享同一序列化边界（慢订阅被
SDK 显式终结，不回滚已提交的 add）。不向正式 SDK 加任何屏障 API。

## 令牌与摘要

每个节点的令牌由环境变量 `CONEX_TOKEN` 提供，进程从不输出明文。中心
`host.toml` 只保存摘要（64 位小写十六进制 SHA-256）：

```sh
printf %s "$TOKEN" | sha256sum
```

`host.toml` 中的两个摘要来自开发专用明文（**不是秘密，仅限本机开发**）：

| principal_id | endpoint | 开发明文 | token_hash |
|---|---|---|---|
| `counter-ts` | `counter-ts` | `conex-s1-dev-token-ts` | `d4caba8e…28ce` |
| `counter-rust` | `counter-rust` | `conex-s1-dev-token-rust` | `fbe92e8a…0700` |

换用自己的令牌：按上式生成摘要替换 `host.toml`，并把明文放进启动环境。

## 运行

前提：`cargo build -p conex-host`、`cargo build -p conex-sdk --examples`，
Bun 已安装。`host.toml` 监听 `127.0.0.1:8788`；P1 持久根落在
`examples/s1/.runtime/`（已 gitignore）。想换端口/临时目录，复制配置并改
`listen` 与三个 root 即可。

终端 1 —— 中心：

```sh
cargo run -p conex-host -- examples/s1/host.toml
```

终端 2 —— TS 节点：

```sh
CONEX_URL=ws://127.0.0.1:8788/wss \
CONEX_TOKEN=conex-s1-dev-token-ts \
CONEX_SPACE=dev \
CONEX_ENDPOINT=counter-ts \
CONEX_PEER_ENDPOINT=counter-rust \
bun run examples/s1/counter-node.ts
```

终端 3 —— Rust 节点：

```sh
CONEX_URL=ws://127.0.0.1:8788/wss \
CONEX_TOKEN=conex-s1-dev-token-rust \
CONEX_SPACE=dev \
CONEX_ENDPOINT=counter-rust \
CONEX_PEER_ENDPOINT=counter-ts \
cargo run -p conex-sdk --example counter
```

两个节点都从自己提供能力开始，随后发现并消费另一语言节点；每步输出一行
结构化 JSON（`event` 为 `start`/`ready`/`publish`/`discover`/
`snapshot`/`read`/`add`/`update-seen`/`overflow-rejected`/
`termination`/`done`），供人工与 T08 驱动读取。流程：订阅对端
`counter/value`（快照 rev 0）→ read=0 → add(3) → read=3 → add(0) 仍发
rev+1 更新 → 越界 add 被拒且 revision 不变 → unsubscribe → `done`。

`done` 仅表示本节点消费对端的演示流程完成；已发布的提供端继续服务，不会
因此断开连接或退出，避免先完成的节点在另一节点发现前消失。两个节点均
输出 `done` 后，在各终端按 Ctrl-C（SIGINT），或向节点进程发送 SIGTERM，
节点才关闭 SDK 连接并退出；自动化驱动也应在读到两个 `done` 后显式终止
节点，再停止中心。无需固定等待时间或退出宽限期。

环境变量契约：`CONEX_URL`、`CONEX_TOKEN`、`CONEX_SPACE`、
`CONEX_ENDPOINT`、`CONEX_PEER_ENDPOINT` 五个；集成 id 固定为
`"integration"`。**不要把任意环境变量直接传给远端执行器；令牌只经本机
环境注入，宿主与节点日志均不含秘密。**
