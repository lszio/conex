# P4 Codex 适配器 — 实施记录

日期:2026-11-01。范围:[P4 计划](../plans/2026-10-31-conex-mvp-p4.md) §2–§5。**live 验收被后端形态漂移阻断,未通过;协议映射与离线验证已交付**——P4 计划 §6 允许 live 与离线分别报告,本页如实分列。

## 1. 已交付

| 交付物 | 位置 | 证据类型 |
|---|---|---|
| Codex app-server 适配器 | `crates/conex-agent/src/adapters/codex.rs`:owned stdio 形态、JSON-RPC 双向(newline 分帧)、`thread/start`→Conversation 映射(backend_mapping=threadId)、`turn/start|completed`→执行状态机、`item/agentMessage/delta`→流式(先于 item/started 的 delta 自动开消息)、`item/commandExecution/requestApproval` 族→审批交互、未知 server request 显式 JSON-RPC error 拒绝 | 源码观察 + 单元测试 |
| 应用注册 | `implementation = "codex"`(节点配置 `[codex]` 后可安装);approvalPolicy/sandbox 全部来自批准配置 | 配置校验 + 44 agent 测试 |
| 能力诚实声明 | approval/cancelExecution/recoverActive=true(P0 live 证据);steer/userInput/listSubagents/cancelSubagent=false(P0 未覆盖,不谎报);steer 方法显式 unsupported_capability | 单元测试断言 |
| 终态语义 | `turn/completed.status=interrupted` 才置 cancelled;interrupt 回执不充当终态(P0 §4 冻结处理) | 单元测试 |

## 2. live 验收:未执行(后端形态漂移,如实记录)

P0(2026-10-29,codex-cli **0.163.0**)实测 `codex app-server` 直连 stdio 可用(thread/start、turn/interrupt、requestApproval→decline 均 live)。本轮(2026-11-01)同版本复测:

- `codex app-server` 直连:**stdin 写入 initialize 后零输出静默退出**(多帧格式、P0 精确 payload 均复现)。
- 新形态为 `app-server daemon`(后台常驻)+ `app-server proxy`(stdio→control socket);`daemon start` 成功、control socket 出现,但 **proxy 的帧格式未随 schema 文档化**(JSON-RPC newline 分帧直写无响应),schema 生成物(`generate-json-schema`)不包含传输层。
- 结论:协议 schema 稳定(事件名与 P0 一致),但**接入传输形态已变**;适配 owned stdio 形态按 schema 实现,live 需要后端形态核实(升级行为确认或 proxy 帧协议文档化)后补齐。

A06(真模型会话、外部 app-server 未误停)未通过,不宣称交付。

## 3. 已知边界与债务

- 读方法面与 OMP 适配器为两份同语义实现(store 直读)——已知重复,集成所有者合并时统一(P4 计划 §1 共享层条款)。
- 外部认证 WS 形态(`--ws-auth capability-token`)未实现:本机接入形态核实前不做半套;配置校验不含该形态。
- 审批未决时消费者断开 → invalidated 处理依赖 `recover_unfinished`(P1 已交付),P0 §4.3 的"恢复核对"在 live 恢复后验收。

## 4. 门禁状态

`cargo test -p conex-agent`(44,含 codex 契约/能力诚实断言)全绿;全量门禁随 P6 一并汇报。未提交/推送/部署。
