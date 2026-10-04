# conex

conex（connect + nexus）是一个可嵌入的双向能力路由内核及可选独立进程；工作协议名 `conex`。

**当前状态（2026-10-04）：** 公开落地页已上线 <https://conex.lszio.space>（分支 `landing`）：产品叙事、请求链路演示、免凭据访客只读控制台、`/llms.txt`，访客打开即用。P0、P1、Connected landing、多主机内容 M0–M5 已交付。本轮修复三个真实缺陷：容器反代后的明文监听被配置校验拒绝（新增 `allow_plaintext_bind`）、`/content` 缺 `Content-Length` 导致视频无法拖动、基础镜像浮动 tag 拉取卡死导致部署永久挂起（改为 digest 固定）。逐项证据：[落地页验证记录](docs/verification/landing-page.md)。历史：多主机内容 M0–M5（[验证记录](docs/verification/multi-host-content.md)）；剩余 M4（Notez 接入）与 M6（验收收口）见计划。

## 文档

完整地图与阅读顺序见 [docs/README.md](docs/README.md)。

- 落地页验证记录（当前）：[docs/verification/landing-page.md](docs/verification/landing-page.md)
- Dokploy 部署运行手册：[docs/runbooks/dokploy.md](docs/runbooks/dokploy.md)
- 设计（权威）：[docs/design/2026-09-14-conex-design.md](docs/design/2026-09-14-conex-design.md)
- 路线图：[docs/plans/2026-09-15-conex-roadmap.md](docs/plans/2026-09-15-conex-roadmap.md)
- 多主机内容计划（当前）：[docs/plans/2026-10-01-conex-multi-host-content.md](docs/plans/2026-10-01-conex-multi-host-content.md)
- 多主机内容验证记录（当前）：[docs/verification/multi-host-content.md](docs/verification/multi-host-content.md)
- Connected landing 契约：[docs/contracts/connected-landing.md](docs/contracts/connected-landing.md)
- P1 执行计划（归档）：[docs/plans/2026-09-15-conex-p1.md](docs/plans/2026-09-15-conex-p1.md)
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

## 多主机内容已交付（M0–M5）

- 契约：`endpointId + resourceId` 定位 + revision；`source/read` 返回 `text ⊕ content`；`blob/get` 互斥目标（committed 块 / remote 范围）；长度一律十进制字符串（JS 安全）。
- 安全：`[web_guest]` 访客配额（会话数/空闲过期/签发限速）；blob 所有权（Owner 持久化、可达集证明、逐方法归属校验）；访客连接面板隔离。
- 传输：Agent 链 protobuf 二进制（`DataChunk` 原始字节，无 base64）；Host 授权探针 + 反向链路逐 256 KiB 切片；同源 `/content`（Range/206/416/ETag/If-Range/nosniff/attachment 策略/流式取消）。
- 页面：主机分组目录、文本/图片/视频原生展示、DOCX（mammoth）与 ZIP（fflate）受限预览、二进制元数据下载；错误八类分示；1440/390 双视口 38 项浏览器检查全过。

## 下一步

多主机内容计划的剩余工作：**M4**（接入真实 Notez 服务，需要真实 Notez 仓库与运行实例）与 **M6**（验收收口：multi-host-content / public-content-security e2e 套件注册、容量门槛、跨主机 TLS 验证）。入口：[多主机内容计划](docs/plans/2026-10-01-conex-multi-host-content.md)。

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

`schema/conex/*.proto` 与 `conformance/schema/*.proto` 是唯一结构类型源；`cargo xtask generate`
派生 Rust 类型（prost/pbjson）、JSON Schema 与 TypeScript 类型（ts-proto）。生成产物入库、禁止手改；
权威严格解码点是 `MethodContract.prepare`/`validate_output`。版本 pin 见 `tools/codegen.lock.json`。

## 工具链

Rust 与 protoc 不随仓库提供，安装在 gitignored 的 `.toolchain/`（见 `scripts/bootstrap-toolchain.sh`）。
