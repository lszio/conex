<div align="center">

# conex

**connect + nexus —— 可嵌入的双向能力路由内核**

一个中心 host 统一做认证、授权、端点目录与路由；文件、HTTP 目录、反向连接的 agent
作为 provider 接入同一份契约。Rust 内核 + TypeScript SDK + 一个能真连真跑的工作台。

**当前状态（2026-10-06）：** 落地页是**工作台**：顶部状态栏常驻本标签页的名字、短链接、分组 key 与在线数，面板切换内容。**一个标签页就是一个客户端**——链接在取 WSS 票据时铸造，同一浏览器的两个标签页共享 cookie 却是两个独立客户端，文件归属也按链接区分。问候从消息泡泡改为右上角提示；`client/hello` 带 `payload` 时是**提问**：目标 SDK 不自动确认，回答由提示里的「确认并回复」回传。`group` 由自述标签升级为**隔离键**（SHA-256 派生），不同组互相不可见、不可问候、不可访问文件；`client/status` 报出每组实测延迟（未测量显示「尚未测量」而非 0 ms）。实测首屏 4.1s → 0.54s（bundle 827KB 未压缩 → 104KB gzip），后台标签页不再被保活策略踢下线。契约见 [connected-landing §9](docs/contracts/connected-landing.md)，证据见 [工作台验证记录](docs/verification/landing-page.md)。P0、P1、Connected landing、多主机内容 M0–M5 已交付。剩余 M4（Notez 接入）与 M6（验收收口）…

[在线工作台](https://conex.lszio.space) · [文档导航](docs/README.md) · [设计 v6](docs/design/2026-09-14-conex-design.md)

</div>

## 目录

- [这是什么](#这是什么)
- [快速开始](#快速开始)
- [用它做什么](#用它做什么)
- [架构](#架构)
- [性能参考](#性能参考)
- [开发与门禁](#开发与门禁)
- [贡献](#贡献)
- [路线图](#路线图)
- [文档](#文档)
- [许可与联系](#许可与联系)

## 这是什么

中心服务要接多个异构供给方，同时不能把调用者的身份、私有侧凭据和回调边界一起交出去。
conex 解决的是这一层：

- **双向**——私有侧 agent 主动出站 WebSocket 反连中心，不开放入站端口，秘密值留在自己这侧。
- **数据导向**——按注册键定位构造器、方法契约与处理器，业务主路径不按 provider 名字分支。
  第二个 provider 走与第一个完全相同的装配路径。
- **契约先行**——一份 `.proto` 派生 Rust、TypeScript 与 JSON Schema；跨语言共享向量验证一致性。
- **失败诚实**——未实现、不可用、权限拒绝、结果未知、传输中断是不同的错误码，不合并。

工作台里发生的事都是真实的：在线客户端列表、hello 往返延迟、提问—回答往返、同组文件共享，
跑在一个真实的 `conex-host` 进程上，没有模拟数据。

## 快速开始

需要 Rust（stable，见 `rust-toolchain.toml`）、protoc 和 [Bun](https://bun.sh)。
工具链不随仓库提供，装到 gitignored 的 `.toolchain/`：

```bash
./scripts/bootstrap-toolchain.sh
source .toolchain/env.sh
bun install
```

### 1. 三进程演示：中心 host + 两个反连 agent

一条命令起全栈，无需手写配置：

```bash
cargo xtask landing-demo        # 需要时自动 build:web 与两个二进制
```

启动后打开打印出来的 `http://127.0.0.1:<port>/`。**一个标签页就是一个客户端**——
同一浏览器开两个标签页会拿到两条链接、两个独立客户端（cookie 说明「谁在浏览」，
链接说明「哪个标签页在说话」）。关掉一个标签页，它在列表里就消失。

详见 [connected-landing 运行手册](docs/runbooks/connected-landing.md)。

### 2. 工作台开发循环

```bash
bun run dev:hello                # 访客免凭据的本地 host，固定 127.0.0.1:8080
```

### 3. 把 conex 当作 broker 用

```bash
cargo run -p conex-host -- examples/p0/host.toml
```

`examples/p0/host.toml` 是 loopback 明文的最小配置：静态 bearer + 受限 fs provider。
入库的 token 是 SHA-256 摘要，配置里不存明文：

```bash
printf %s "dev-token-a" | sha256sum     # 填进 [[tokens]] token_hash
```

TypeScript 侧：

```ts
import { ConexClient } from "@conex/sdk";

const client = await ConexClient.connect({
  url: "http://127.0.0.1:8787/rpc",
  tokenProvider: async () => "dev-token-a",
  requires: ["source/list", "source/read", "source/search"],
});

// 成功
const read = await client.read("notes-local", { resourceId: "hello.md" });

// 拒绝：未授权路径在拨号与凭据解析之前就返回 forbidden
try { await client.read("notes-local", { resourceId: "../escape.md" }); }
catch (error) { /* error.code === -32002 */ }

// 部分失败：一个 provider 超时/不可用不影响其他结果
const results = await client.searchMany([
  { endpointId: "notes-local", input: { root: "", query: "conex" } },
  { endpointId: "catalog-work", input: { root: "", query: "conex" } },
]);
// 成功项带 result，失败项带 error，顺序与输入一致

client.close();
```

生产部署必须配 `[tls] cert/key`。`allow_loopback_http = true` 只允许 loopback 监听，
容器里监听非 loopback 地址要用 `allow_plaintext_bind`（TLS 由前置反代终结）。
完整字段说明见 [P0 运行手册](docs/runbooks/p0.md)。

## 用它做什么

已交付且有真实验收证据的能力：

| 能力 | 说明 | 证据 |
|---|---|---|
| Broker 只读数据连接 | fs 与 HTTP catalog 两个 provider 经同一 Registry / 授权 / 审计路径 | [P0 验证记录](docs/verification/p0.md) |
| 私有侧反连 | 两条 WSS Profile（JSON text + protobuf 二进制），bearer 在 upgrade 认证 | [P1 验证记录](docs/verification/p1.md) |
| 二进制与恢复 | 内容寻址 blob、Stream 信用/ACK/重放、1 GiB 弱网续传 | [P1 验证记录](docs/verification/p1.md) |
| 真 OIDC | RS256 id_token 验签，issuer pin + aud + nonce + JWKS | [P1 验证记录](docs/verification/p1.md) |
| 多主机内容 | `endpointId + revision` 定位、访客配额、blob 所有权、同源 `/content`（Range/ETag/206/416） | [多主机内容验证记录](docs/verification/multi-host-content.md) |
| 交互工作台 | 分组隔离、hello / 提问—回答、组内文件共享、实测延迟展示 | [落地页验证记录](docs/verification/landing-page.md) |
| 容器化部署 | 单镜像多阶段构建，基础镜像按 digest 固定；Dokploy compose 栈 | [Dokploy 运行手册](docs/runbooks/dokploy.md) |

## 架构

```
                 ┌──────────────────────────────────────────┐
   浏览器 UI ──▶ │  conex-host（唯一装配根）                  │
   （WSS + ticket）│  认证 · 授权 · 端点目录投影 · 路由 · 审计    │
                 └──┬──────────────────────┬────────────────┘
                    │ Registry / MethodContract（数据导向分派）
        ┌───────────┴───────────┐          ┌────────────────────┐
        │  inproc / HTTP 传输    │          │  provider 实现      │
        └───────────┬───────────┘          │  source-fs（cap-std）│
                    │                      │  http-catalog        │
   ┌────────────────┴──────────────┐       │  反连 agent          │
   │  私有侧 conex-agent（出站 WSS）  │◀─────┘                     │
   │  凭据留在本侧，不开放入站端口      │                            │
   └───────────────────────────────┘                            │
                                                               │
              ┌────────────────────────────────────────────────┘
              ▼
   共享类型源：schema/conex/*.proto
     └─ cargo xtask generate ─▶ Rust (prost/pbjson) · TypeScript (ts-proto) · JSON Schema
        └─ conformance/vectors/{p0,p1} 跨语言共享向量
```

| crate / 目录 | 职责 |
|---|---|
| `crates/conex-proto` | 类型源、JSON 映射、方法与错误契约、Profile 标识 |
| `crates/conex-core` | Registry、路由、授权接口、调用与会话状态机（无具体 provider 依赖） |
| `crates/conex-host` | 装配、监听、策略与凭据实现、审计出口 |
| `crates/conex-agent` | 私有侧反连、受限 workspace 与上游凭据 |
| `crates/conex-content` | CID、blob 上传、pin、持久性、GC |
| `crates/conex-source` | source 能力的数据源抽象与快照 |
| `crates/conex-provider-fs` | 受限文件系统 provider（cap-std 约束在 root 之下） |
| `crates/conex-provider-http-catalog` | 静态 JSON HTTP catalog provider（整分区授权、真实 TLS） |
| `crates/conex-transport-http` | HTTP / JSON-RPC 2.0 承载 |
| `crates/conex-assembly` | 把 provider 装进 host 的装配根 |
| `sdk/typescript` | `@conex/sdk`：`ConexClient`（HTTP）、`ConexWsClient`（cookie → ticket → WSS） |
| `web/` | 工作台前端（Bun + React + Tailwind） |
| `schema/conex/*.proto` | **唯一结构类型源**，生成物入库且禁止手改 |
| `conformance/vectors/` | 跨语言共享一致性向量 |

## 性能参考

由 `cargo xtask bench --suite hello` 测得：启动真实 `conex-host`，用真实 SDK 客户端跑，
结果写入 `web/src/perf-data.json` 供工作台性能页渲染。**数据为本机 loopback（127.0.0.1，
debug 构建），不代表公网延迟**；线上实测 hello 往返为 19–20 ms。

| 场景 | 样本 | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| hello 单客户端给自己 | 15 | 0.37 ms | 0.46 ms | 0.46 ms | 0.46 ms |
| hello（8 客户端在线） | 60 | 0.73 ms | 41.29 ms | 41.57 ms | 41.57 ms |
| hello（64 客户端在线） | 60 | 0.67 ms | 41.32 ms | 41.77 ms | 41.77 ms |
| client/list（8 客户端） | 160 | 0.45 ms | 0.72 ms | 0.75 ms | 0.79 ms |
| client/list（64 客户端） | 640 | 2.20 ms | 2.81 ms | 3.09 ms | 3.68 ms |

两点诚实结论：

1. **注册表规模不影响 hello 延迟。** 8 与 64 客户端在线时分布几乎相同，链路成本与在线人数无关。
   真正随人数增长的是 `client/list`（0.45 → 2.20 ms），这也是页面轮询间隔取 3s 的原因。
2. **p95 的 41 ms 是真的，本轮未定位。** 慢样本稳定落在每轮第二个发送者，第一个永远正常；
   量级与 Linux delayed-ACK 吻合，但未做抓包确认，因此性能页照实展示，不做粉饰。

复现：

```bash
cargo xtask bench --suite hello            # 60 轮
cargo xtask bench --suite hello --rounds 15
```

二进制通道的量级见 [P1 验证记录](docs/verification/p1.md)：1 GiB 经 Stream 走 4096×256 KiB
分块往返，坏块被拒，断线后用 `alreadyHaveChunkCids` 续传（e2e 约 488 s）。

## 开发与门禁

**单一类型源**：`schema/conex/*.proto` 与 `conformance/schema/*.proto` 是唯一结构类型源。
`cargo xtask generate` 派生 Rust 类型（prost/pbjson）、JSON Schema 与 TypeScript 类型（ts-proto）。
生成物入库、**禁止手改**；权威严格解码点是 `MethodContract.prepare` / `validate_output`。
工具版本 pin 在 `tools/codegen.lock.json`。

```bash
cargo xtask generate                  # .proto → Rust / TS / JSON Schema
cargo xtask generate --check          # CI 用：检测生成漂移
cargo test --workspace --locked
bun test sdk/typescript/tests
bun run typecheck
cargo xtask conformance --vectors conformance/vectors/p0
cargo xtask conformance --vectors conformance/vectors/p1
cargo xtask e2e --suite p0-ts
cargo xtask check                     # 上面全部 + fmt + clippy -D warnings
```

`cargo xtask check` 是提交前唯一的门禁，与 [.github/workflows/ci.yml](.github/workflows/ci.yml)
跑同一套步骤。常用 e2e 套件：

| 命令 | 覆盖 |
|---|---|
| `cargo xtask e2e --suite p0-ts` | 真实 Rust host + Bun SDK 的 fs 读写路径 |
| `cargo xtask e2e --suite p1-stream-1gib` | 1 GiB 经 Stream 弱网续传 |
| `cargo xtask e2e --suite connected-landing` | 三进程拓扑、agent 生命周期 |
| `cargo xtask e2e --suite connected-landing-web` | 浏览器端连接与票据 |
| `cargo xtask e2e --suite connected-landing-connections` | 连接注册与回收 |

浏览器验收：`CONEX_VERIFY_ORIGIN=http://127.0.0.1:8080 bun scripts/verify-landing-scenes.ts`
（三个独立 `BrowserContext` = 三个独立访客，跑分组隔离、文件共享与归属）。

## 贡献

**在动手之前先开一个 issue 或 PR 描述你想做什么**，尤其是契约变更——
设计 §0 的硬要求是「扩展在已定义契约内只新增实现与注册项；增加新语义需演化契约」。
先看 [设计 v6](docs/design/2026-09-14-conex-design.md) 与对应 [docs/contracts/](docs/contracts/)。

约定（仓库已在执行）：

- **分支**：从 `dev` 开 `feat/`、`fix/`、`docs/`、`refactor/` 前缀的分支。`dev` 是当前集成分支
  （`refactor/arch` 与 `feat/landing-workbench` 均已合入）。
- **提交信息**：Conventional Commits，英文祈使句，标题以 `:` 结尾（一行说清这一提交做了什么，
  长的部分写进正文）。主题取 `feat` / `fix` / `docs` / `refactor` / `perf` / `test` / `build` /
  `ci` / `chore`，scope 可选（如 `fix(host):`）。
- **提交正文**：说清**为什么**，不是**改了什么**——diff 已经说了改了什么。解释一个取舍、
  一个被否决的替代方案、或一个之前会出错的场景。正文用英文，与标题一致。
- **单个提交是一个完整想法**：不混入无关的 fmt 债、依赖升级或重命名。
- **门禁必须绿**：`cargo xtask check` 与 CI 跑同一套步骤。不提交跳过门禁的 `#[ignore]`
  或 `.skip`。
- **不手改生成物**：`schema/generated/`、`sdk/typescript/src/generated/` 一律改 `.proto` 后重新生成。
- **文档口径**：验证记录只写**实际执行并通过**的结果；未实现能力必须明确标注，
  不把计划的 "Expected" 复制成结果。契约变更同步 `docs/contracts/*.md`。
- **不提交秘密**：配置里只放 SHA-256 摘要与占位值，不提交真实 token、私钥或证书。

加一个 provider 的完整清单（设计 §2.2 的扩展验收）：

1. 在 `schema/conex/` 里扩展方法契约（若需要新消息）；
2. 实现 provider，注册进 `crates/conex-assembly` 的装配根；
3. 在 `conformance/vectors/` 加重或改向量；
4. 第二个 provider **不得**改动 `conex-core` 的执行分支——加内核分支即验收失败。

## 路线图

| 阶段 | 范围 | 状态 |
|---|---|---|
| P0 | broker 只读数据连接、JSON-RPC/HTTP 与 inproc、两个 provider、TS consumer | ✅ 已交付 |
| P1 | 私有 agent 反连、独立二进制上传、网络恢复、受控写入 | ✅ 已交付 |
| 中心服务与落地页 | 中心 host + 反连 agent + 浏览器会话/ticket + 端点目录 | ✅ 已交付 |
| 多主机内容 M0–M5 | 内容契约、访客配额、blob 所有权、二进制通道、`/content`、浏览页面 | ✅ 已交付 |
| 多主机内容 M4 / M6 | Notez 接入（需真实 Notez 仓库与实例）、验收收口 | ⏳ 未开始 |
| P2 | ACP CLI 网关与 MCP 适配 | ⏳ 未开始 |
| P3 | 自有空间加密历史、可选发现与受控中继 | ⏳ 未开始 |
| P4 | 单项生产能力（HA、更多承载） | ⏳ 未开始 |

完整的阶段依赖与工作包见 [路线图](docs/plans/2026-09-15-conex-roadmap.md)；
当前正在推进的工作见 [多主机内容计划](docs/plans/2026-10-01-conex-multi-host-content.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [docs/README.md](docs/README.md) | 文档导航入口（规范 / 契约 / 计划 / 证据 / 操作） |
| [设计 v6](docs/design/2026-09-14-conex-design.md) | 唯一权威设计与分阶段验收 |
| [P0 wire 契约](docs/contracts/p0-wire.md) | JSON-RPC 2.0 信封、参数、错误码、HTTP 行为 |
| [Connected landing 契约](docs/contracts/connected-landing.md) | 中心 host、浏览器会话/ticket、agent 反连、客户端契约 |
| [P1 契约](docs/contracts/p1-stream.md) | Stream 信用/ACK/重放状态机（blob / operation / session 同目录） |
| [P0 运行手册](docs/runbooks/p0.md) | broker 模式：生成、启动、token、凭据、SDK 调用 |
| [Connected landing 运行手册](docs/runbooks/connected-landing.md) | 三进程演示、token 三元组、日志与门禁 |
| [Dokploy 运行手册](docs/runbooks/dokploy.md) | 单容器公开演示形态与多主机生产形态 |
| [验证记录](docs/verification/) | 每个阶段的实际执行结果与已知缺口 |

架构与实现细节在改契约前务必读设计文档；证据口径以各验证记录为准。

## 许可与联系

[MIT License](LICENSE) — Copyright © 2026 lszio。可自由使用、修改、分发（含商用）与再许可，
需保留版权与许可声明；软件按「原样」提供，不附带担保。贡献代码即视为同意以该许可发布。

- 仓库：<https://github.com/lszio/conex>
- 在线工作台：<https://conex.lszio.space>
- 报告缺陷或安全问题：请开 issue；安全敏感内容请先私信维护者，不要公开细节。
