# conex 目标架构迁移盘点

> 日期：2026-10-17。对照 [目标架构](2026-10-17-conex-architecture.md)。
> 范围：静态源码与接口核对、GitNexus 流程检索。没有运行服务、外部 OIDC、SDK 互操作、文件传输、build 或 tests；本页不是交付或安全验证记录。
> 本轮不修改运行代码、不删除配置或数据、不提交代码。下文“保留”表示候选机制可继续使用，不表示可以免验收迁移。

## 1. 结论

不整仓重写。保留通用路由、契约校验、授权先于目标连接、身份来源区分、WSS 反连和文件分块的已有机制；调整 SDK 的提供端能力、节点与工作空间模型、插件生命周期和工作台应用边界。

最重要的缺口：

- TS SDK 已有调用和推送回调，但尚不能据此认定为通用的能力提供端。
- Rust agent 已有远端 provider 运行机制，不等于独立、可嵌入、提供与消费对称的 Rust SDK。
- 当前 tenant/endpoint 规则和 group 加入凭据不是 Space 成员授权。
- OIDC 验签代码存在，但其身份结果到用户会话、统一 Principal 和 Space 权限的完整链路仍需重整。
- 当前 React 工作台是具体业务页面，不是已完成的 Astro 插件卡片宿主。

## 2. 证据口径与图谱限制

- **CONFIRMED**：下列路径中可直接看到的结构或执行分支，不等同于运行验证。
- **MISSING（限定范围）**：在列明的入口、模块和配置中未找到目标机制，不声称所有仓库、部署或外部依赖中都不存在。
- **DOC**：仅来自历史设计或验证记录的声称，不据此确认代码或现网行为。

本轮 `node .gitnexus/run.cjs status` 报旧索引落后于工作树，随后执行 `analyze --index-only` 完成刷新。刷新报告 8,952 nodes、22,134 edges、537 flows；同时报告跨语言字段链接缺失及流程入口/分支截断，并跳过一个大型生成文件。因此图谱只用于定位，空结果不能证明没有调用方或没有能力。

在显式绑定 `--repo conex` 的图中查询 `broker registry authorization dispatch` 与 `SDK websocket client provider registration`，定位到 `Host.invoke`、agent dispatch、SDK WebSocket 和 Host WebSocket 分派；结论由源码交叉核对。SDK 与 core/host 两个范围分别只读调查，没有并发编辑。

## 3. 保留与调整矩阵

| 范围 | 源码证据与状态 | 目标处置 |
|---|---|---|
| 数据导向注册 | **CONFIRMED**：[Registry](../../crates/conex-core/src/registry.rs#L20-L25) 存工厂、路由、端点表；[install_routes](../../crates/conex-core/src/registry.rs#L69-L155) 校验归属、广告方法与注册冲突 | 保留表驱动及冲突拒绝机制；增加插件、接入、模块模型，不能只把现有 endpoint 改名后宣称完成 |
| 执行安全路径 | **CONFIRMED**：[Host.invoke](../../crates/conex-core/src/execution.rs#L27-L150) 先解析路由、prepare、tenant 检查、Policy 授权和有界准入，再执行目标连接 | 保留顺序和公共执行路径。接入新的 Space 判断与可信远程上下文；明确当前路径仍固定使用 P0 protocol/version，不能当作任意协议已通用 |
| 当前 ACL | **CONFIRMED**：[PolicyRule](../../crates/conex-core/src/policy.rs#L8-L16) 和 [authorize](../../crates/conex-core/src/policy.rs#L79-L104) 比较 principal、tenant、endpoint、action 和资源范围；该规则没有 Space 成员轴 | 保留默认拒绝、资源范围和策略版本机制；补齐 Space 成员与资源归属，不把 tenant 字符串或 group key 直接当 Space 授权 |
| 外部身份映射 | **CONFIRMED**：[IdentityMap](../../crates/conex-core/src/identity.rs#L8-L36) 已将带来源的 IdentityKey 映射为 principal/tenant，并另设 actor_peer_id | 保留模型；必须在真实登录和节点登记路径使用并验证，不能把类型存在当成已接线。设备 Node 与 Principal 关联，但不把 Node ID 直接等同于用户 ID |
| OIDC | **CONFIRMED**：[OidcVerifier](../../crates/conex-host/src/oidc_jwt.rs#L110-L238) 包含 issuer、audience、可配置 nonce、时效及 RS256/JWKS 校验代码；[票证路径](../../crates/conex-host/src/tickets.rs#L306-L334) 使用 subject 作 principal、固定 tenant 为 `oidc` 并返回 ticket | 保留验签候选实现，重新核对登录安全与身份映射。完成 issuer+subject 到内部身份、会话及 Space 的链路，不将现有票证返回视为完整 OIDC 工作台登录 |
| 浏览器与 group | **CONFIRMED**：[WebSession](../../crates/conex-host/src/web_auth.rs#L65-L75) 跟踪一个会话的多个链接；[group_key](../../crates/conex-host/src/clients.rs#L41-L54) 来自自报文本，[set_profile](../../crates/conex-host/src/clients.rs#L350-L379) 更新组，[列表](../../crates/conex-host/src/clients.rs#L432-L441) 按 key/可见性过滤 | 保留用户会话与标签页链接分离。group 只保留为现有应用隔离机制，不能升级命名为 Space；另建稳定节点登记、Space 成员和撤销规则 |
| TS 通信 SDK | **CONFIRMED**：[WebSocket hello](../../sdk/typescript/src/ws-client.ts#L410-L417) 发送空 provides/requires；[hello 回答及订阅](../../sdk/typescript/src/ws-client.ts#L560-L582) 已有特定业务的双向交互；[公开导出](../../sdk/typescript/src/index.ts#L25-L42) 包含 dashboard 和 ws-client | 保留连接、请求关联和回调机制。新增通用 handler 注册、能力发布及提供端调度；把特定 hello/dashboard API 与通用通信边界分开。不能把“已有推送应答”写成“已有通用 provider SDK” |
| Rust agent | **CONFIRMED**：[握手](../../crates/conex-agent/src/runtime.rs#L96-L105) 声明提供的方法并使用 ProtobufWss；[注册](../../crates/conex-agent/src/runtime.rs#L142-L156) 发布配置端点；[EndpointProvider](../../crates/conex-agent/src/runtime.rs#L357-L385) 使用共享 Handler 与 fs provider | 复用提供端机制，抽出可嵌入通信能力并补齐消费端；agent 作为部署应用使用 SDK，而不是让所有 Rust 消费者依赖完整 agent 进程 |
| Rust 公开包边界 | **CONFIRMED**：[workspace](../../Cargo.toml#L3-L14) 有 core、host、agent、proto 等成员，没有独立的公开 SDK 成员 | 可在迁移计划中确定 SDK crate 边界；不为目标名称预建空包，也不把 crates 的存在当作对称 SDK 已完成 |
| 文件与内容 | **CONFIRMED**：[工作台列表](../../web/src/lib/files.ts#L105-L130) 访问同源 `/web/files`；[上传](../../web/src/lib/files.ts#L158-L178) 已有声明分块、发送和提交调用 | 分块机制是复用候选，不重做已经存在的算法；页面专用 HTTP 与通用内容能力的归属另行收口。已有分块代码不证明目标恢复、权限、保留和有界内存语义均满足 |
| UI 宿主 | **CONFIRMED**：[当前应用](../../web/src/app.tsx)、[页面 hook](../../web/src/hooks/use-hello-page.ts)、[依赖](../../package.json#L13-L30) 是 React 与 hello/share 业务；**MISSING（上述范围）**：目标卡片类型注册、接入/模块绑定和 Astro 外壳 | 在保持现有页面可用的前提下迁移到 Astro 外壳，评估复用独立交互组件；新增卡片注册与服务端持久化，不能只换构建框架 |
| 插件与领域契约 | **CONFIRMED**：已有 [Manifest 结构](../../schema/conex/manifest.proto) 和 Registry；**MISSING（上述 SDK、agent 与页面入口范围）**：完整 Plugin→Integration→Module 生命周期、依赖注入和通用派生模块 | 复用既有类型生成与注册原则；先定义插件专项契约，再实现启动停止、依赖校验、发布/撤销和组合。不将 capability manifest 直接当作可执行代码加载器 |

## 4. 明确不延续的耦合

以下是后续迁移中的删除目标，不是本轮已删除项，也不是无调用方判断：

1. 通用通信 SDK 对 hello/dashboard 业务的默认绑定。保留有用能力时应作为应用契约或扩展提供，迁移全部调用方后移除旧耦合，不留兼容别名。
2. 将浏览器 link 或自报 group 当作长期节点身份、组织成员或授权边界的设计。
3. 新通用插件继续依赖页面 cookie、CSRF 或 `x-conex-link-id` 的接入方式；这些可以属于浏览器会话边界，不能成为所有 SDK 的必要条件。
4. 新工作台继续通过集中式 hello 页面 hook 增加每个数据源与卡片的具体分支。
5. 旧设计里 Node 专指密文存储/中继节点的术语，以及旧 P0–P4 路线作为新方向默认计划的口径。

不会仅因目标架构更新而删除现有文件共享、内容存储、旧 API、用户数据或历史验证记录。真正切换前先查影响、迁移调用方与配置、确认数据处置，再删除旧默认路径；不以双版本运行无限延期切换。

## 5. 当前反模式与对应修正

| 当前边界问题 | 证据 | 修正验收 |
|---|---|---|
| 消费端、推送回调被误当作对称 SDK | ws-client 的空 provides 与特定 hello API，见矩阵 | TS/Rust 均能注册真实 handler 并被另一语言调用，不经过 UI 专用方法 |
| 身份/隔离字段存在被误当作成员授权 | tenant ACL、group key、OIDC ticket 路径，见矩阵 | 两个主体跨 Space 请求被拒绝；撤销使受影响订阅结束 |
| 页面业务成为公共协议的默认入口 | index.ts dashboard 导出、页面专用 files HTTP，见矩阵 | 无浏览器的程序能使用相同模块；卡片不负责凭据与路由 |
| 现有注册表被误当作完整插件运行时 | Registry 和 Manifest 只有已列明职责，见矩阵 | 配置多个接入、依赖拒绝、停止释放资源、派生模块再发布均有真实验收 |

## 6. 下一步实施边界

[S1 专项契约](../contracts/s1-capabilities.md) 已整理为待评审设计，尚未实施。评审后再编写可执行计划，不立即改整个 Host：

1. 明确通用 provider 注册/撤销、目标绑定、远程调用上下文、错误和取消规则。
2. 定义“状态快照→更新”的无静默缺口语义、重连后的新快照及事件中断语义。
3. 选定真实 TS→Rust 和 Rust→TS 调用/订阅场景，列出现有 SDK、agent、core 和 Host 的实际修改点；修改符号前执行影响分析。

   特别注意：[现有 WS 重连代码](../../sdk/typescript/src/ws-client.ts#L653-L718) 会重排并发送 pending 调用；S1 的非重放副作用不能原样复用此策略。通信重连与调用重试必须分开。
4. 在 S1 通过后推进 S2 插件运行时；Space 和 OIDC 的 S4 完成前，仅在受限开发环境使用新增能力。

插件契约打包、页面持久化表结构、OIDC 浏览器回调及内容恢复细节属于对应专项设计，不在此静态盘点中伪造为已完成的实施方案。独立的文件传输增强不阻塞 S1。
