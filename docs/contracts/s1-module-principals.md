# S1：模块消费主体（module principal）契约草案

> 状态：**用户已评审的目标契约（2026-11-05）**；尚未实施、未生成 schema、无验收证据，实施前本文不改变任何现有方法的行为。三个取舍已定案，见 §9。
> 目的：让组合模块经**显式远程绑定**消费能力时获得模块粒度的授权与归属，闭合 [MVP 验收 A14](../design/2026-10-29-conex-private-platform.md#14-mvp-验收矩阵) 的消费侧。
> 权威关系：演化 [S1 契约](s1-capabilities.md) §4（静态批准）、§5.2（context）与 §7（调用）的**消费判定**；不改发布、代次、订阅状态机与执行边界。组合模块再发布能力沿用既有 S1/S2 发布路径，本文不涉及。

## 1. 问题与决定

现状：S2 显式远程绑定（`RemoteDep`）经节点的发布连接转发，中心只见连接的 operator principal。`ConsumeApproval` 键里的 `principal_id` 实际匹配的是**节点**——节点上任何模块都能消费该节点获准的一切。操作员调用与组合模块消费不可区分，能力绑定图（基线 §6「谁经哪个契约使用哪个提供者」）无法落表。

采用以下决定：

1. **新增一类消费主体**：模块主体，标识「某节点上某应用中的某模块」。消费批准的 `principal_id` 接受它，精确元组、默认拒绝的模型不变。
2. **节点声明（attestation）**：节点在转发帧中声明调用来自哪个模块主体；中心校验形状、节点归属与消费批准。中心不独立验证进程内归属——节点是 MVP 唯一的可信装配根（基线 §10.2），这不是新引入的信任，是把它显式化。
3. **批准键是稳定身份，不含 generation**。generation 随帧记录、只用于绑定图与审计，不参与授权；「旧句柄不能接管新实例」约束的是发布句柄（基线 §5），不是消费归属。
4. **本地依赖不进本模型**：`deps.local` 是进程内直调、零网络帧（S2 契约 §5 边界不变），无需也不得要求消费批准。
5. **现有操作员批准完全不变**：`consumer_module` 缺省的请求按现状判定；迁移是纯增量的配置能力。

## 2. 原语与命名

| 名称 | 定义 |
|---|---|
| 模块主体 | 字符串 `module/<nodeId>/<applicationId>/<moduleId>`；三段均为既有 `is_valid_identifier`（字母数字与 `-` `_` `.`），区分大小写 |
| nodeId | 消费方节点的**链路认证 peer 身份**（握手 `linkIdentity.peerId`，即连接凭据对应的主机身份）；必须等于承载该请求的已认证连接的节点。由 SDK 在调用时注入，**配置不可声明**——冒认他节点在形状校验之外再被 peer 归属判定拦截 |
| applicationId | 节点配置中应用包的稳定标识（配置声明、启动校验，随包安装） |
| moduleId | 应用包内发起消费的模块名 |
| consumerModule | `RequestContext` 新增字段（**tag 8**）：模块主体字符串；空 = 连接主体自身 |

选字符串而非嵌套消息：principal 在批准表、`Caller`、转发上下文里都是不透明字符串，一种形状一个校验器。选新字段而非复用 `principal_id`：`principal_id` 的语义是「认证所得主体」，不得被请求内容稀释（S1 §4.1「直接客户端提交 principalId 视为 bad_request」）。

## 3. 中心侧判定

适用于 `module/invoke` 与 `state/subscribe` 的转发（`module/cancel` 见 §5）：

| 步骤 | 条件 | 失败结果 |
|---|---|---|
| 1 形状 | `consumer_module` 非空时必须是合法模块主体形状 | `bad_request`，`execution=not_started`，`retry=never` |
| 2 节点归属 | 主体中的 `nodeId` == 该连接认证的节点 | `forbidden`，同上（权限先于披露，不泄漏原因细节） |
| 3 消费批准 | `(principal=该模块主体, spaceId, providerEndpointId, contractId)` 精确存在，且方法/状态在批准投影内 | `forbidden`，同上 |

全部失败都发生在发送边界之前，不触达提供端、不执行 handler。通过后：

- 转发上下文的 `principal_id` 填**模块主体**，`tenant_id` 沿连接——提供端的本地权限与领域绑定面向最终消费者（如 Agent 域的 `AuthorizedScope` 绑定到模块主体而非节点）。
- operator principal 不再随转发下传；它保留在中心的审计与绑定图记录里：`(连接主体, 模块主体, generation, 目标, 方法)`。
- `consumer_module` 为空的请求按现状判定与转发，字段不参与任何分支。

## 4. 节点与 SDK 侧

- **身份在调用时由 SDK 组装，作者不输入**：node 段取当前链路握手认证的 peer 身份（hub 盖章，配置不可声明）；application/module 段来自所在实例的组合上下文。TS 与 Rust SDK 对称。
- 节点连接层对转发帧附 `consumer_module`；本节点模块消费**本节点发布**的能力同样走此路径（不因同节点而绕过批准）。
- **本地依赖直调不附该字段**：零帧路径没有上下文可填，也无需批准（§1 决定 4）。

## 5. 取消与订阅的归属扩展

- `module/cancel` 的「同主体/scope」判定扩展为：请求声明的模块主体与原调用记录一致才可取消；操作员连接（空 `consumer_module`）可取消本连接全部在途。取消不要求对该方法的消费批准（S1 §7.3「失去权限不阻止停止自己的在途请求」）。
- `state/subscribe` 建立后，订阅归属记录消费者为模块主体；订阅生命期内的授权撤销以建立时的批准为准，不逐帧重查（与现状 operator 订阅一致）。
- 终止码沿用 S1 §8.3；授权撤销 → `forbidden`。

## 6. 验收场景

实施后的通过条件，不是当前结果：

| 编号 | 场景 | 必须观察到的结果 |
|---|---|---|
| M-A01 | 无模块批准的组合消费 | 组合模块远程 invoke `forbidden`，即使其节点 operator 有同目标批准；提供端 handler 未执行 |
| M-A02 | 冒用他节点模块身份 | 连接 A 声明节点 B 的模块主体 → `forbidden`，不转发任何业务帧 |
| M-A03 | 主体形状非法 | `bad_request`，先于一切披露；目录与注册状态不因探测变化 |
| M-A04 | A14 骨架 | 组合模块经模块主体消费两个真实能力、再发布普通能力，第三个消费者只见再发布能力；绑定图记录两层消费 |
| M-A05 | 本地依赖回归 | `deps.local` 直调零网络帧、无批准要求，现有 S2-A02 不变 |
| M-A06 | 撤销模块批准 | 新调用 `forbidden`；在途调用按 S1 §7 收敛，不追溯已执行副作用 |
| M-A07 | 操作员路径回归 | 现有 S1 A01–A12 与 S2 全套不改动仍通过；`consumer_module` 缺省行为逐字节不变 |

## 7. 非目标

- 模块自有凭据或独立认证——节点仍是唯一认证方；这是单用户 MVP 的边界，不是多用户方案。
- 动态批准 API、批准继承或通配主体；批准仍是静态精确元组。
- 本地依赖纳入授权模型；跨节点模块身份联邦；direct-only/relay-allowed 策略任何变更。
- 发布侧（`PublishApproval`）与代次激活语义的任何变更。

## 8. 影响清单（实施时逐项 impact）

| 位置 | 变更 |
|---|---|
| `schema/conex/rpc.proto` | `RequestContext.consumer_module = 8`（本轮唯一新 wire 字段）；重跑 `xtask generate` |
| `conex-core/capability.rs` | `ConsumeApproval` 接受模块主体；主体形状校验入口 |
| `conex-host` capability/calls、broker、ws_transport | 判定三步、转发上下文填充、审计/绑定图记录 |
| `conex-sdk`（Rust `plugin.rs` / TS `plugin.ts`） | `RemoteDep` 组合身份自动填充 |
| `conex-agent` | 连接转发路径附带 `consumer_module` |
| `conformance/vectors` + S1/S2 e2e 套件 | M-A01…A07 向量与场景 |
| [s1-capabilities.md](s1-capabilities.md) | §4.1/§5.2/§7 增补交叉引用（实施提交内完成） |

## 9. 已评审的取舍（2026-11-05 定案）

1. **逐请求声明**（定案）vs 连接级声明注册表（连接 ready 时申报可代表的模块集合，中心按集合校验）：后者状态更多、能防声明洪泛，MVP 单用户不需要；批准表本身已封顶可声明主体。
2. **提供端只见模块主体**（定案），operator 留在中心审计：领域绑定（Agent scope、审计归属）面向真实消费者；「代表执行」若某领域需要，应是该领域契约的显式字段，不是通用通道默认值。
3. **批准键不含 generation**（定案）：重启不失效、配置稳定；换代窗口内旧实例仍可消费由节点启动期拒绝，中心无法独立观测进程死亡。
