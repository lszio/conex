# conex

conex（connect + nexus）是一个可嵌入的双向能力路由内核及可选独立进程；工作协议名 `conex/1`。

**当前状态（2026-09-15）：** P0 已交付并通过门禁（分支 `refactor/arch`，验收提交 `e649ded`）。下一步是 [P1 执行计划](docs/plans/2026-09-15-conex-p1.md) 的首个工作包 P1-01 契约冻结。

## 文档

完整地图与阅读顺序见 [docs/README.md](docs/README.md)。

- 设计（权威）：[docs/design/2026-09-14-conex-design.md](docs/design/2026-09-14-conex-design.md)
- 路线图：[docs/plans/2026-09-15-conex-roadmap.md](docs/plans/2026-09-15-conex-roadmap.md)
- P1 执行计划（下一步）：[docs/plans/2026-09-15-conex-p1.md](docs/plans/2026-09-15-conex-p1.md)
- P0 执行计划（归档）：[docs/plans/archive/2026-09-15-conex-p0.md](docs/plans/archive/2026-09-15-conex-p0.md)
- P0 wire 契约：[docs/contracts/p0-wire.md](docs/contracts/p0-wire.md)
- P0 运行手册：[docs/runbooks/p0.md](docs/runbooks/p0.md)
- P0 验证记录：[docs/verification/p0.md](docs/verification/p0.md)

## P0 已交付

broker 平面只读数据连接，经同一 Registry/授权/审计路径：

- 协议：`.proto` 单一类型源 → Rust/TS/JSON Schema；冻结 ErrorCode 数值表；CIDv1 raw/SHA-256。
- 核心：Registry + MethodContract、身份映射、资源策略、目标准入、统一执行与审计、限额与部分结果。
- provider：受限 fs（cap-std）与 HTTP catalog（整分区授权、真实 TLS）。
- 入口：静态 bearer + 60 秒 binding 的 JSON-RPC/HTTP；TLS 或显式 loopback 明文；env/file 凭据。
- SDK：`@conex/sdk` typed consumer（list/read/search、searchMany 部分结果、错误映射）。
- 验证：跨语言共享向量、additivity 检查、真实 Rust host + Bun 的 fs E2E（catalog 由 provider 集成/授权测试覆盖）；逐项结果见 [P0 验证记录](docs/verification/p0.md)。

## 下一步（P1）

P1 在 P0 主干上增加两个 WSS Profile、conex-agent 反连、有界 blob 分块/持久提交/GC、Stream 与网络恢复、浏览器 ticket、Session/operation 状态与条件写。入口是 [P1 执行计划](docs/plans/2026-09-15-conex-p1.md) 的 P1-01 契约冻结；P0 遗留的 TLS pin、SIGTERM 排空与双 provider SDK E2E 缺口也一并排入。

## 快速开始

```bash
./scripts/bootstrap-toolchain.sh
source .toolchain/env.sh
bun install
cargo xtask generate
cargo test --workspace
bun test sdk/typescript/tests
cargo xtask check          # 完整 P0 门禁
```

## 三个例子

成功读取（inproc 或 HTTP 语义一致）：

```ts
const read = await client.read("notes-local", { resourceId: "hello.md" });
// read.text === "hello conex\n"；read.cid 与共享 CID 向量一致
```

拒绝（未授权路径在业务目标拨号/凭据解析前返回 forbidden；catalog 单文件权限不触发上游 GET）：

```ts
try { await client.read("notes-local", { resourceId: "../escape.md" }); }
catch (error) { /* error.code === -32002 */ }
```

部分失败（一个 provider 超时/不可用不影响其他结果）：

```ts
const results = await client.searchMany([
  { endpointId: "notes-local", input: { root: "", query: "conex" } },
  { endpointId: "catalog-work", input: { root: "", query: "conex" } },
]);
// 成功项带 result，失败项带 error，顺序与输入一致
```

## 单一类型源

`schema/conex/v1/*.proto` 与 `conformance/schema/*.proto` 是唯一结构类型源；`cargo xtask generate`
派生 Rust 类型（prost/pbjson）、JSON Schema 与 TypeScript 类型（ts-proto）。生成产物入库、禁止手改；
权威严格解码点是 `MethodContract.prepare`/`validate_output`。版本 pin 见 `tools/codegen.lock.json`。

## 工具链

Rust 与 protoc 不随仓库提供，安装在 gitignored 的 `.toolchain/`（见 `scripts/bootstrap-toolchain.sh`）。
