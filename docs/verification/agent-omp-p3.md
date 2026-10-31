# P3 OMP 适配器 — 实施记录

日期:2026-10-31。范围:[P3 计划](../plans/2026-10-31-conex-mvp-p3.md) §2–§6。运行平台 macOS(darwin/arm64);live 验收使用本机已有授权的真实模型。

## 1. 交付内容

| 交付物 | 位置 | 证据类型 |
|---|---|---|
| OMP RPC 适配器 | `crates/conex-agent/src/adapters/omp.rs`:一个活动会话一个受控进程、JSONL 帧编解码(物理 1 MiB / v2 分片重组 64 MiB,标准 base64 严格校验)、delta 模式事件映射、extension_ui 交互、子代理生命周期 | 源码观察 + live e2e |
| 应用注册与权限映射 | `adapters/mod.rs`(agent 权限类映射 `method_allowed`,node.rs 两个 invoke 路径与 assembly 共用);`implementation = "omp"` 仅在节点配置 `[omp]` 后可安装 | 单元测试(42 个 agent 测试含注册表/契约一致性) |
| 领域 store 扩展(v4) | `conex-agent-domain`:`backend_mappings` API、`admit_backend_subexecution`(派生执行不抢顶层槽)、`relationships` 表(backend/config/user 来源)、消息 `backend_key` 映射、history/interactions/relationships/endpoints/conversations 读路径、`ToolPartAppended` 观察 + `message_part` 事件(events.proto 新增位次 17,生成链已同步) | domain 35 测试全绿 |
| node_config | `[omp]`:backend_binary / model(精确 provider/model,客户端不可选)/ tools 白名单 / approval_mode(拒绝 yolo 与 auto-approve)/ max_concurrent_sessions(1..=16)/ workspaces(rootRef 键→真实根路径,路径不出节点) | 配置校验测试 |
| 入站数字归一化 | `conex_proto::wire::value_to_json` 改为 pub;local_api/direct_api 的 `params.input` 转换统一走它——protobuf Value 的 f64 数字收敛回 i64,uint32/uint64 字段可以 JSON number 传入 | live S1–S6(S2 之前的 Content 解码失败的根因修复) |
| schema 生成器 | `xtask/src/schema.rs`:非 presence 标量不再标 `required`——pbjson 省略默认值(false/0/""),缺失与默认在 wire 不可区分;必填语义由 MethodContract.prepare 与领域层(ULID/非空 content)执行 | 生成链 + 全量门禁 |
| live 验收入口 | `xtask/src/agent_omp_live.rs`(`cargo xtask e2e --suite agent-omp-live`),独立于 `cargo xtask check` | 见 §2 |

## 2. 实际运行结果(`cargo xtask e2e --suite agent-omp-live`,2026-10-31 通过)

真实进程:1 node(内置 omp 适配器)+ 真实 OMP 18.14.0 子进程 + 真实模型 `deepseek/deepseek-flash`(机器既有授权;隔离临时 workspace/session 目录;finally 清理自建进程)。CONEX_OMP_MODEL 可覆盖模型选择。

- S1 创建幂等:同 idempotencyKey 返回同一 conversation。
- S2 真模型流式:submit → delta → prompt_result completed + sessionSettled → **succeeded**;助手消息文本含模型答案。
- S3 取消:running 中 cancel → abort → **cancelled** 终态(abort 回执未冒充终态)。
- S4 人工交互:ask 工具触发 select(Continue/Stop)→ CAS 应答 `Stop` → resolved → turn 自然 completed(**拒绝后命令未获准**)。
- S5 并发:第二会话独立进程,不抢占。
- S6 重启恢复:node kill + 重启 → 同会话 submit 成功,history 保留(11 条消息)。
- S7 子代理:**items=0(模型未派生子代理)**——关系映射链路(生命周期帧→派生执行→Relationship)已实现并有单元覆盖,但 **live 证据未取得**,不宣称 A09 的 subagent 部分已验证。

单元/回归:`cargo test -p conex-agent`(42)、`-p conex-agent-domain`(35)全绿;全量门禁见 §4。

## 3. 实施中修正的真实缺陷

- **bootstrap 静默失败**:上一版把 `request()` 成功返回的 data 再当整个帧检查 `success` 字段,协商永远"失败"且错误被吞;`open_session` 同样在 data 里找 `data` 键取不到 sessionId。修正后 bootstrap 错误也必须落日志。
- **响应死锁**:bootstrap 阶段主循环未启动,`request()` 等 oneshot 回执无人分发——改为顺序泵帧(非本命令的帧交 handle_frame;启动期扩展 UI 请求显式拒绝,不静默挂起)。
- **OMP 旗标(18.14.0 实测)**:`--tools ask` 需要 `--mode rpc-ui` 且不能 `--no-extensions`(ask 工具由扩展 UI 提供,违反时进程直接退出);纯文本会话才用 rpc + --no-extensions。
- **入站数字 f64 化**:protobuf Value 数字是 f64,`{"limit":10}` 经 local_api 变 `10.0`,pbjson 拒绝浮点整数 → uint 字段全部不可用。统一经 `value_to_json` 归一化回 i64。
- **空 text part 不可往返**:pbjson 把全默认 part 序列化成 `{"text":{}}`,无法反序列化 → 消息改为零部件起步,首个部件由首次 TextAppended/ToolPart 创建;用户消息只有 end 带全文时由 end 补齐(delta 缺失是 delta 模式的常态)。
- **schema required 与 pbjson 矛盾**:非 presence 标量标 required 但 pbjson 省略默认值——生成器不再标注(见 §1)。
- **消息完整性判定**:message_end 权威文本长度与累计流字节比对,部分流到中途的差异判 incomplete,不谎称完整。

## 4. 明确未交付(不因本页关闭)

- **S7 子代理 live 证据**:模型未派生子代理;`agent/subagents/list|cancel` 与关系投影只有单元级验证,待模型愿意派生时补 live。
- **steer / userInput 类**:契约已声明(`steer` 返回 unsupported_capability),能力位 false;OMP steer/ask 多问题形态未实测。
- **OMP 工具审批(kind=approval,confirm 形态)**:交互主路径(select)已 live 验证;confirm 形态未在本轮触发。
- **TUI 附着调查(P3.5)**:未执行;端点 catalogScope=node-managed,可控/只读/不可附着的界面区分随 P7/P8。
- **live 模型前提**:bs/DeepSeek-V4-flash(LAN 10.25.7.127:3210)本轮不可达、zhipu-coding-plan/glm-4.5 配额用尽(2026-11-05 重置)、minimax 404;deepseek/deepseek-flash 可用并用于本轮验收。后续 live 需模型在线。

## 5. 门禁状态

`cargo xtask check` 全绿(fmt、clippy -D warnings、workspace 测试、bun、generate --check、三套 conformance、p0-ts / s1 / s2 / private-runtime / private-network e2e);`agent-omp-live` 独立入口通过(见 §2)。未提交/推送/部署。
