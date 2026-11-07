# S1 模块消费主体：验证记录

> 状态：已实施（2026-11-06）。实现提交在 `feat/module-principals`（M0–M5 各自独立提交），本文只记录实际执行并通过的结果；未执行项如实标注。
> 契约：[s1-module-principals](../contracts/s1-module-principals.md)（2026-11-05 评审定案）。计划：[实施计划](../plans/2026-11-05-conex-s1-module-principals.md)。

## 1. 交付范围与运行结果

| 工作包 | 交付 | 证据 |
|---|---|---|
| M0 | `RequestContext.consumer_module = 8`（唯一新 wire 字段）；JSON 双向编解码；控制方法 context 拒绝该字段 | 生成链 `xtask generate --check` 过；全门禁绿 |
| M1 | 中心三步判定（形状→节点归属→批准）于 `module/invoke`/`state/subscribe`；转发 `principal_id` 盖章模块主体；配置接受模块主体 | 单元：`s1_calls` 19/19（含 M-A01/A02/A03 语义） |
| M2/M3 | 双 SDK 组合身份 + 节点接线（详见 §3 设计修订） | `module-principals` e2e |
| M4 | **已实施（2026-11-08）**：P1 统一审计汇落 `Broker::invoke`（预留先于分发、收尾含错误自带诊断阶段）；判定拒绝记录含契约 §3 四元组（连接主体、模块主体、generation、spaceId/registrationId+方法）；`FileAuditSink` 由 End 摘要改为完整记录 | `s1_calls::audit_record_captures_module_consumer_denial` / `audit_record_allows_operator_calls_without_module_dimension` |
| M5 | 新 e2e 套件 `cargo xtask e2e --suite module-principals`（真实 host + 两个真实节点 + TS 第三消费者） | 实际输出见 §2 |
| M6 | 契约交叉引用随 M1 提交；本页为证据页 | — |

## 2. e2e 实际输出（真实进程）

`cargo xtask check`（含本套件）全绿时实际打印：

```text
M-A04 actual=approved module consumer passes judgment; derived read/add forward upstream; republished capability serves a third operator consumer ok
M-A01 actual=unapproved module principal forbidden; operator approval on the same node does not extend to it ok
M-A02/M-A03 actual=raw-frame foreign claim forbidden and malformed shape bad_request; catalog unchanged under probing ok
M-A06 actual=module approval removal forbids new calls; operator direct approval unaffected; composite degrades explicitly ok
module-principals acceptance: all scenarios passed
```

拓扑：一个真实 `conex-host`、两个真实 `conex-agent` 节点（各自凭据主体）、一个 TS `ConexWsClient` 第三消费者。节点 B 发布演示计数器；节点 A 跑两个 `derived-counter` 应用（已批准/未批准各一），经**钉死的远程绑定**消费 B；提供端 SDK 本地 exposure 白名单面向最终消费者（含模块主体）。

| 场景 | 覆盖 | 备注 |
|---|---|---|
| M-A01 | 未批准模块主体 forbidden，同节点操作员批准不外溢；hub 判定失败零业务帧 | e2e |
| M-A02 | 冒认他节点 forbidden | e2e（2026-11-08 原始帧探针：操作员连接声明他节点主体 → -32002）+ 单元级（`s1_calls`，真实 Broker+Router） |
| M-A03 | 形状非法 bad_request、先于披露 | e2e（同原始帧探针：4 种畸形声明逐个 -32602，且探测前后 `module/list` 目录与注册恒等）+ 单元级（同文件 + core `principal_tests`） |
| M-A04 | A14 骨架：批准的主体消费两个操作（read/add 转发上游）→ 再发布 → 第三操作员消费者 | e2e |
| M-A05 | 本地依赖零帧直调 | 既有 S2-A02 回归（`xtask e2e --suite s2-plugins` 在全门禁内） |
| M-A06 | 撤销模块批准 → 新调用 forbidden；操作员直连上游不受影响；**经组合的能力显式降级**（组合的上游授权被撤，其对外能力诚实失效，不伪造成功） | e2e（host 重启换配置 + 注册轮换重钉） |
| M-A07 | 操作员路径逐字节回归 | 全门禁内既有 S1/S2 套件未改一行全绿 |

## 3. 实施中的设计修订与发现（契约已同步）

1. **node 段改为链路认证 peer（安全增强）**：原设计允许配置声明 node 段；实现中发现 hub 认证维度是**连接 principal**（`linkIdentity.peerId`），与节点本机 `node_id` 是两个标识。定稿：node 段由 SDK 在调用时取握手认证的 peer 身份，**配置不可声明**——冒认在形状校验之外再被 peer 归属判定拦截。契约 §2/§4 已同步。
2. **潜伏 bug（P2 起就存在）**：节点内应用（空 url、借用共享发布链路）声明 remote_deps 时，SDK 在启动期用空 url 自拨必然失败（"empty string"）。修复：启动期跳过自拨，`RemoteDep` 在链路缺失时按既有语义显式 Unavailable。
3. **SDK JSON 信封漏发 consumerModule**：`context_json` 未发射新字段（声明被静默丢弃 → 按节点 principal 判定）。修复并镜像 `conex_proto::wire` 规则。
4. **提供端 exposure 面向最终消费者**：转发帧 caller 现为模块主体，提供端本地 exposure 白名单必须包含它（契约 §9.2 决策的直接后果）；部署节点时需把获准的模块主体列入 exposure_principals。
5. **错误码保真**：派生 handler 原经 `node::failure` 把上游错误塌缩为 bad_request；改为 sdk Error 直接透传，hub 判定码对调用者可见。
6. **P2 演示计数器按 100ms tick 走时**（`count = ticks + adjustment`）：跨读断言以走时容差约束（精确 +3 落在界内），不假设绝对值稳定。

## 4. 未覆盖与边界

- M-A02/M-A03 的原始 WSS 帧探针（2026-11-08）只覆盖操作员连接上的冒认与畸形声明；节点连接（发布链路）上的同型声明由单元级判定语义覆盖（SDK 不会发送非法声明，节点侧无探针形态）。
- 绑定图落表（基线 §6 的图存储）仍是独立工作项；审计记录是它的原料，已随 P1 统一审计汇落地（[计划](../plans/2026-11-08-conex-p1-audit-sink.md)）。
- e2e 的注册轮换由 host 重启驱动（静态批准无热重载）；"不重启撤权"语义仍属动态批准 API 的非目标（契约 §7）。
