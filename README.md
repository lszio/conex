# conex

conex（connect + nexus）是一个可嵌入的双向能力路由内核及可选独立进程；工作协议名 `conex/1`。

- 设计：[docs/design/2026-09-14-conex-design.md](docs/design/2026-09-14-conex-design.md)
- 路线图：[docs/superpowers/plans/2026-09-15-conex-roadmap.md](docs/superpowers/plans/2026-09-15-conex-roadmap.md)
- P0 执行计划：[docs/superpowers/plans/2026-09-15-conex-p0.md](docs/superpowers/plans/2026-09-15-conex-p0.md)
- 阶段验证记录：[docs/verification/](docs/verification/)

当前处于 **P0**：broker 平面只读数据连接、JSON-RPC/HTTP 与 inproc、最小 CID、env/file 凭据。
`dev` 分支保留上一代 TypeScript CONEX（Anytype ↔ Apple）；两者关系见设计 §0.1。

## 工具链

Rust 与 protoc 不随仓库提供，安装在 gitignored 的 `.toolchain/`：

```bash
./scripts/bootstrap-toolchain.sh
source .toolchain/env.sh
bun install
```

精确版本记录在 [tools/codegen.lock.json](tools/codegen.lock.json)。

## 常用命令

```bash
cargo xtask generate          # 生成 Rust / TS / JSON Schema 产物
cargo xtask generate --check  # 校验产物与源码一致（CI 用）
cargo test -p conex-proto     # Rust 契约测试
bun test sdk/typescript/tests # TypeScript 契约测试
bun run typecheck
```

## 单一类型源

`schema/conex/v1/*.proto` 与 `conformance/schema/*.proto` 是唯一结构类型源。
`cargo xtask generate` 从同一 descriptor 派生 Rust 类型（prost/pbjson）、JSON Schema 与
TypeScript 类型（ts-proto）。生成产物入库但禁止手改；权威严格解码点是
`MethodContract.prepare`/`validate_output`（设计 §5.2），JSON Schema 只作文档与 TS 边界输入。
