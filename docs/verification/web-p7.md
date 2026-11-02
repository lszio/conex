# P7 Web 领域工作台 — 实施记录

日期：2026-11-03。范围：[总计划 MVP-P7](../plans/2026-10-29-conex-private-mvp.md#mvp-p7web) 五个子项全部交付；平台 macOS (darwin/arm64)，验收为真实 Chrome（Playwright channel=chrome）对真实 hub+node 进程的操作，模型为 `deepseek/deepseek-flash`（本机 `omp` 18.15.0 环境授权；`bs/DeepSeek-V4-flash` 网关当日晚间不可达，见 §4）。

## 1. 交付内容

| 交付物 | 位置 | 说明 |
|---|---|---|
| 私有域客户端（TS SDK） | `sdk/typescript/src/private.ts` | `PrivateHubClient`（引导码登录/目录/grant，CSRF 不落 URL）、`NodeDirectClient`（grant→30 秒一次性票据→`conex-ticket.<t>` 子协议 WSS→JSON-RPC `node/invoke`，断线自动重建并重取 grant） |
| 事件/域校验修复 | `sdk/typescript/src/events.ts`、`agent-domain.ts` | `message.part`（工具活动）事件入投影；delta 续接点改为 `offsetBytes+已应用字节`（totalBytes 是后端声明的完整大小）；空 `events` 数组按空页处理；`Buffer.byteLength` 换成 TextEncoder（浏览器可用） |
| 节点自报清单 | `conex-sdk/client.rs::access_node_identity`、`conex-agent/runtime.rs` | 发布链身份注册携带 instances/modules/generation/contract/methods（纯元数据），随 5 秒同步循环刷新、变化才重发 |
| hub 私有目录 | `conex-host/private_access.rs::private_directory`、`http.rs` | `GET /private/directory`：已注册节点 + 自报清单，私有会话 cookie 读取 |
| Web 工作台 | `web/src/{app.tsx,views/,hooks/use-workbench.ts,domain/session.ts}` | 入口分发（私有未配置→legacy 访客页；401→登录；200→工作台）；会话视图（端点/Workspace/会话选择、流式、取消、审批横幅、busy/unknown/终态徽标、正文分段取回）；服务器视图（真实采样、stale 语义）；图谱视图（关系/执行/Tags 增删）；节点地址按 nodeId 记忆（P5 已知地址直连） |
| 票据 CORS 修复 | `conex-agent/direct_api.rs` | 实际 POST 响应补 `Access-Control-Allow-Origin` 等头（原先只有预检带，浏览器照样拦截） |
| 私有 CSP | `conex-host/web.rs`、`serve.rs` | `[private]` 部署放宽 `connect-src`（工作台必须直连节点地址），script/style 仍钉死 'self' |
| proto3 JSON 修复 | `conex-agent/adapters/omp.rs`、`conex-proto/agent_domain.rs`、契约 | 端点列表手工拼 JSON 的 None 改省略；adapter 词表补 `builtin`（P6 内置模块端点自报值） |
| 领域存储修复 | `conex-agent-domain/store.rs` | `create_conversation` 同时登记 `entities` 行（P6 标注的存在性校验从此对真实会话成立）；OMP 适配器跳过提交回显的 user `message_start`（重复"你"的根因） |
| env64 凭据后端 | `conex-access/lib.rs::read_secret` | `env64:NAME`：base64 的 pkcs8 经 secret manager 环境变量注入（部署用） |
| 浏览器验收 | `scripts/verify-private-workbench.ts` | 真实 Chrome 全场景（下表）；截图见 `docs/verification/assets/p7-0*.png` |

## 2. 实际运行结果（2026-11-03，真实 Chrome + 真实模型，11/11）

`CONEX_VERIFY_ORIGIN=http://127.0.0.1:8790 bun scripts/verify-private-workbench.ts`，前置：真实 hub（loopback HTTP + `[private]`）+ 真实节点（OMP/metrics/relations，直连入口 7451）：

- ok login view accepts the bootstrap code and renders the workbench（引导码单次消费）
- ok directory shows the registered node identity（节点 principal + epoch）
- ok direct channel establishes（grant→票据→WSS 全链路）
- ok agent endpoint and workspaces discovered（目录 inventory 驱动）
- ok conversation created; composer enabled
- ok real model streaming completed（真模型 `deepseek/deepseek-flash` 回复 "READY" 渲染）
- ok pending interaction surfaced in the approval banner（ask 工具触发的 selection）
- ok deny answer resolved the interaction and the turn completed（拒绝后回合完成）
- ok cancel reached the cancelled terminal state（取消→`cancelled` 终态徽标）
- ok server metrics render real live samples（CPU/内存/磁盘/平台 macos/aarch64）
- ok graph view tags the conversation（relations 模块 tags/put→list 往返）

回归：`cargo xtask check` 全门禁（含 private-network e2e 回归、conformance 向量、clippy、TS/Rust 类型检查与单测）通过，见 git 记录。

## 3. 实施中修正的真实缺陷

- **票据响应无 CORS 头**：节点直连只给 OPTIONS 预检回了 Allow-Origin，实际 POST 没有——浏览器按 CORS 拦截（`Failed to fetch`）。补齐实际响应头。
- **快照空窗口 vs Follower 累计语义冲突**：store 的 created 事件携带"空窗口 + 正确 totalBytes"（契约 §4 明文），Follower 却把 totalBytes 当续接点，首条 delta 必 resync。改为 `offsetBytes + 已应用字节` 续接。
- **提交回显重复用户消息**：admit 已记录用户消息（created+delta+completed），OMP 对同一提交的 user `message_start` 又映射成第二条。适配器跳过 user 角色回显。
- **`message.part` 事件未入 TS 投影**：工具活动会让 Follower 以 unknown type resync（P1 交付时 store 尚无该事件）。
- **Web 会话槽位泄漏**：`/web/logout` 是写操作需 CSRF；无头的验收登出没带头，8 个/主体的槽位被耗尽（429 web session capacity exceeded）。
- **entities 表缺口**：`create_conversation` 不登记会话实体，P6 标注对真实会话必失败。创建即登记。
- **入口分发丢 HTTP 状态**：SDK 错误未透传 `status`，401 被当成"非私有部署"渲染 legacy 页。
- **浏览器 `Buffer` 不可用**：域校验用 `Buffer.byteLength`，浏览器无此全局，所有带长度上限的 input 校验抛错。
- **端点列表 None→null**：手工拼 JSON 的 `Option::map` 产出显式 null，TS 严格校验拒绝。改省略 + 词表补 `builtin`。
- **空 events 数组**：proto3 JSON 省略空数组，Follower 把"无新事件"当成协议故障。

## 4. 明确未交付 / 环境注记（不因本页关闭）

- **`bs/DeepSeek-V4-flash` 网关不可达**（`10.25.7.127:3210` 超时）：当日 live 用 `deepseek/deepseek-flash` 完成；`bs` 恢复后无需重验（同一适配器代码路径，P3 live 已记录该模型）。
- **ZCode 内置浏览器（IAB）沙箱拦截一切跨源回环请求**（连普通 HTTP 服务都拦）：验收用 Playwright 驱动的真实 Chrome 完成；这解释了为何不能用 IAB 验收，不是产品缺陷。
- **TS SDK 的 direct/relay 选择**（P5 转交项）：direct 路径已交付并验收；relay 选择留给 Emacs/P9（Web 只消费 direct-only 模块）。
- **公网 HTTPS→loopback 直连**：Chrome/Safari 对 `127.0.0.1` 豁免混合内容，但 Chrome 的 Private Network Access 预检（`Access-Control-Allow-Private-Network`）未来全面强制时，节点票据端点需补 PNA 头——归 P9 两机公网验收时处理。
- **A08 的 Emacs 半边**（P8）未开始；本页只覆盖 Web。
- 事件流仍是有界轮询（800ms）驱动：直连入口没有服务端推送通道，契约允许（head 提示只是加速）。
