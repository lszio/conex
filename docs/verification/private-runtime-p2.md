# MVP-P2 自治节点实施与真实进程证据

日期：2026-10-29。范围：[P2 计划](../plans/2026-10-29-conex-mvp-p2.md)、[私有运行契约](../contracts/private-runtime.md)。操作入口：[节点运行手册](../runbooks/private-runtime.md)。本记录不把 counter 模块、协议调查或静态授权当成 OMP/Codex Adapter、Web/Emacs 直连或公网私有认证交付。

## 1. 实际交付

| 边界 | 文件 | 实现 |
|---|---|---|
| 应用包 | `crates/conex-agent/src/application.rs` | 精确 TOML schema、重复/未知字段拒绝、256KiB/64 上限、批准工厂/config/权限/导出/来源约束、稳定 Kahn 与具体环 |
| 受信装配 | `node_config.rs`、`assembly.rs`、`node_types.rs` | 显式 RuntimeFactory 环境；完整预检后使用同一个已解析顺序；拒绝导出端点碰撞；固定长度无歧义模块持久键 |
| 节点 | `node.rs`、`runtime.rs`、`main.rs` | 现有二进制 `--node-config`、单节点文件锁、有界64串行协调器、资源核对、应用隔离、幂等 stop 与稳定失败报告 |
| 持久所有权 | `crates/conex-agent-domain/src/{model,store,schema.sql}` | schema v2、v1 backup API 快照后原子迁移、node epoch、事务 generation、Created/Active/Unknown/Released、ready 的 owner-epoch CAS |
| 实际资源 | `process.rs`、`builtins.rs` | 固定自身可执行文件的受控 counter worker；控制秘密经继承 stdin；原控制通道/boot ID/start ticks 核对；外部 worker 只借连接 |
| 发布 | `publication.rs` | 同一 Node 共享一条中心连接、每应用可撤销租约；断线不重建业务；重连新注册；迟到发布按租约围栏 |
| 本地授权 | `access.rs`、`local_api.rs` | 0700目录/0600 Unix socket、OS peer UID、可信配置 principal、S1 严格解码、同契约输入/handler/输出及权限/generation 校验 |
| SDK 默认迁移 | Rust/TS `plugin`、Rust `client` | 单一默认 Instance；业务/发布/连接分离；本地先启动；显式有界异步 cleanup、并发缩短截止时间、失败不假报成功；借用的节点连接不被应用 stop 关闭 |
| 样例与门禁 | `examples/mvp/node/application.toml`、`deployment/agent-a.toml.example`、`xtask/src/{private_runtime_e2e,check}.rs` | 非秘密可运行样例；private-runtime 实际进程套件纳入总门禁；迁移 S2 样例及生成的 fresh-instance 消费者以显式等待发布 |

仅安装真实的 `controlled-counter` / `external-counter`，提供 `conex.counter` read/add。没有提前创建空 Adapter 或假 Agent handler；未知能力明确拒绝。Node boot 复用 P1 `recover_unfinished`，不重放旧执行；后端主动核对由 P3/P4 实际适配器负责。

## 2. 已运行的分项验证

| 命令/场景 | 观察结果 |
|---|---|
| `cargo test -p conex-agent` | 36 passed，含包/权限/来源/环/配置秘密诊断及模块键歧义回归 |
| `cargo test -p conex-agent-domain --test node_ownership` | 5 passed，含真实 v1 迁移快照保留、跨 Store 代际、ready 围栏、unknown 保留/Released 终态 |
| `cargo test -p conex-sdk --test s2_plugin` | 18 passed；实际握手阻塞不阻止本地启动、cleanup 失败稳定 Failed、并发短截止、本地 DI/派生/远程固定引用/重连 |
| `bun test sdk/typescript/tests/s2_plugin.test.ts` | 16 pass / 0 fail，90 expect；同类生命周期和清理失败/截止时间回归 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo xtask e2e --suite s2-plugins` | S2-A01…A08 全部 ok；真实 Rust/TS 派生 read/subscribe、停止隔离、停止后的不复活、新注册与旧引用 stale |
| `cargo xtask e2e --suite private-runtime` | 以下五个实际进程场景全部 ok |
| 单独运行入库 deployment/application 样例 | 仅替换受限临时 data/socket 路径；`local-counter` Active、无中心注册、本地 read 的 count 递增、受限 Unix API shutdown、退出码0 |
| `cargo xtask check` | 退出码0，`xtask check: all steps passed`；包含fmt、workspace clippy/test、bun frozen install/typecheck/test、生成一致性、P0/P1/S1 conformance及P0/S1/S2/private-runtime真实套件，未删旧门禁 |

### private-runtime 真实场景

套件启动临时真实 Host、Node、自有长运行 counter 进程及独立外部 worker；所有测试进程/目录由创建者回收。不是 mock transport，也不是只断一条虚拟连接。

| 输出标记 | 实际断言 |
|---|---|
| `P2-A04` | 真停并重启 Host；本地 counter 继续增长，PID/generation/nodeEpoch 不变；重新发布获得新 registrationId，旧远程引用返回 -32013 |
| `P2-A13` | 两个并发 stop 得到同一报告；旧本地路由拒绝；另一应用的**原有效远程引用**仍可调用，外部进程仍响应 |
| `P2-RECOVERY` | 正常 Node 重启改变 epoch、增加 generation；SIGKILL 后新节点通过持久化原控制通道核对并回收旧自有 worker，再启动新代际，不凭 PID kill |
| `P2-UNKNOWN` | 测试创建者移除旧 worker 控制 socket，再 SIGKILL Node；旧 worker 保持相同 boot/starttime 身份并存活，资源留 Unknown，冲突应用不重启，无关应用与外部进程继续运行 |
| `P2-CLEANUP` | 实际 SQLite trigger 拒绝所有权更新；并发 stop 返回相同 Failed 报告，不吞错误；Node shutdown 仍清理剩余自有应用，外部进程不受影响 |

同一套件还验证：direct-only 模块本地可读、没有中心注册；请求自报 principal 返回严格拒绝。该负路径不等于已经覆盖全部公网 N01–N16。

## 3. 修正过的真实集成缺陷

- 每个应用单独用相同 node 身份建立连接，会触发 S1 节点换代并清除前一个应用的目录。实际 suite 先观察到 catalog 只剩最后一个应用；修正为 **Node 单一 Publication + 各应用租约**，不放宽 Host S1 代次语义。另一应用原引用在停止邻居后仍有效，已实际验证。
- proto3 JSON 会省略数值零；counter handler 现在显式返回契约要求的 `count`/规范字符串 `revision`，包括初始零值，避免真实首读被严格输出校验拒绝。
- 原 Rust 测试只 abort Host 外层任务，已派生的 WSS 连接仍活着，不能代表中心停止。测试 fixture 改为独立 Tokio runtime 并关闭整个 runtime；P2 另用真实 OS Host 进程退出证明断中心。
- SDK cleanup 错误/截止不能被吞成 Stopped；两语言均保留失败终态，Node 在 Store 记录失败时仍继续处理其他资源并缓存同一报告。
- 持久键不能使用裸 `app/module` 拼接：`a/b + c` 与 `a + b/c` 会冲突，两64字节标识也会超出 Store 键上限。现在使用带 app 字节长度前缀的 SHA-256，实际 Store 回归证明两组代际独立。
- 交付后独立评审（Node 侧与 SDK 侧只读子代理）发现并修复三处边界缺陷，均有针对性回归/套件复跑：① 节点 shutdown 在 stop 耗尽截止时间后，发布清理立即超时会丢弃业务 stop 报告并误报失败——现在把完整 `stop_report` 附加进错误诊断；② 活链路上单次 `provide` 被拒后发布路径静默停滞到无关变更——现在按连接路径同样的退避持续重试；③ TS `stop()` 在模块 `start` 悬挂时过期，会清掉该模块 cleanup 上下文导致其 stop 钩子零次执行并报伪造错误——started 条目现在自带 context，迟到回滚仍逆序执行钩子（新增测试 `stop expiry during start still runs a late module's stop hook exactly once`）。

## 4. 图谱与变更边界

完成集成后重新 full index，`index_status=ready`，115922 nodes / 148625 edges；新的 Node Publication 符号已可检索。`detect_changes(depth=1)` 检出267个当前工作区变更文件，包含原有 P1/用户变更，**不能归因于本 P2**。它的 changed_files 未包含新建未跟踪的 `node.rs` / `publication.rs`；因此不能把该列表当成完整 P2影响面，已另确认新符号入图并以实际进程/SDK 回归验证。

通用 `from_str` 名称的图边曾连到 CRITICAL 鉴权节点；LSP 实际引用仅资源 Store 解码。按该证据收窄为内部 `ResourceState::parse_label`，未修改 Host 鉴权路径。其他导出 API 迁移先使用 LSP references 枚举调用方，未以 grep 代替图影响。

## 5. 明确未交付

- P3/P4 OMP/Codex Adapter、真实 Agent 会话/审批/取消产品链路。
- P5 JWT/grant、持久撤权同步、Web/Emacs HTTPS/WSS 直连、私有登录与公网认证策略。
- P6 服务器指标/关系模块、P7/P8 客户端、P9 两机公网/私网及正文 canary 全链路。
- Node counter 的 direct-only 仅指不进入中心数据路径、可经受限 LocalApi 消费，不冒充浏览器直连。

本轮未提交、推送、发布或部署；保留既有 dirty tree，不改用户凭据/模型配置，不删除既有会话或业务数据。
