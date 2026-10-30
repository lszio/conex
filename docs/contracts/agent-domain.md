# Agent 领域契约：私有 MVP

状态：P0 规范；实现与 live 支持程度见 [协议证据](../verification/agent-protocol-probes.md)。本文不声明 Adapter 已实现。依据：[架构基线](../design/2026-10-29-conex-private-platform.md)、[事件恢复](event-recovery.md)、[私有运行](private-runtime.md)。

## 1. 抽象屏障与 wire

Agent 是领域能力，不是内核分支。OMP/Codex 适配器提供同一 `conex.agent/1` 契约；视图仅依赖本契约及声明的可选能力。组合模块像普通模块一样提供自身契约，不暴露适配器实现对象。

所有下表操作经现有 `module/invoke` 的 `input:{method,argument}` 分派；目标、registrationId、剩余预算和认证上下文沿用 S1。操作名不是自动增加的顶层 JSON-RPC 方法。持久事件的查询也使用 invoke，通知使用现有完整状态订阅，详见恢复契约。不把消息 delta 塞进 S1 `state/update` 冒充持久事件。

新增结构的唯一源为 `schema/conex/agent_domain.proto`、`events.proto` 和 `entities.proto`；P1 才创建/生成。现有 `agent.proto`、`session.proto` 分别保留其原有运行协议意义。使用 package `conex`、lowerCamelCase JSON、严格重复/未知键拒绝；非空业务字符串由边界验证。uint64（位置、版本、时间、generation）均为规范十进制字符串，不用 JS Number。新枚举的外部表示为本文的小写字符串，经生成 Schema 与语义验证限制，不依赖生成器默认枚举大小写。

权限记为 `agent.read`、`agent.create`、`agent.execute`、`agent.cancel`、`agent.interact`；权限还须约束节点、模块、Workspace、Conversation。关系可见不授予上述权限。未经授权先返回 forbidden，不泄漏目标是否存在。

## 2. 身份与记录

ID 为仓库惯例的 ULID（26 字符，大小写规范沿用现有验证器），节点负责分配业务 ID，不接受后端路径代替 ID。全局引用为 `{nodeId,id}`；单节点调用可只用 id，目标 nodeId 来自已验证的路由。显示名、相同路径、相同后端标题均不合并对象。

| 类型 | 必须字段 | 可选字段及约束 |
|---|---|---|
| AgentEndpoint | id,nodeId,moduleInstanceId,generation,adapter,backendVersion,protocolVersion,experimental,capabilities,availability,catalogScope,limits | displayName；adapter 为 omp/codex；availability 为 ready/unreachable/unknown/stopped；catalogScope=node-managed/backend；limits 为实际支持的领域限额 |
| Workspace | id,nodeId,displayName,rootRef | rootRef 是节点批准的配置键，不是客户端任意路径；真实根路径仅节点保存 |
| Conversation | id,endpointId,workspaceId,createdAtMs,revision,quality | title,activeExecutionId；quality=current/stale/unknown；revision 为本记录修订，不等于事件位置 |
| Execution | id,conversationId,endpointId,moduleInstanceId,generation,state,revision,createdAtMs,updatedAtMs | parentExecutionId,terminalReason,cancelRequestedAtMs,backendRef；backendRef 仅受限节点记录，默认不进入客户端投影 |
| InteractionRequest | id,executionId,kind,state,revision,createdAtMs,choices,prompt | expiresAtMs；kind=approval/selection/input；choices 使用本请求范围内的键，不执行任意客户端传入动作 |
| Message | id,conversationId,executionId,role,state,revision,parts | role=user/assistant/tool/system；state=streaming/complete/incomplete；parts 为下述有界文本/工具说明 |
| Relationship | id,from,to,kind,source,quality,observedAtMs | source=backend/config/user；quality=current/stale/historical/unknown；详情沿用未来 entities.proto |

所有记录从节点投影导出；授权配置决定是否能通过中心返回。`generation` 是模块运行代际；中心 registrationId 是一次发布的身份，两者不能互换。重连不重建业务 Conversation；模块实例重启必须换代。后端映射以 `(adapter,backendInstanceKey,backendConversationId)` 唯一；外部 Codex 的 server identity 配置键属于映射，不根据 socket 地址推断永久身份。

proto 消息名与本表的对应（P1 实际生成，wire JSON 不含消息名）：`AgentEndpoint`→`conex.AgentAdapterEndpoint`（`agent.proto` 已占用同名控制面消息）、`Message`→`conex.AgentMessage`、`MessagePart`/`TextPart`/`ToolPart`→`conex.AgentMessagePart`/`AgentTextPart`/`AgentToolPart`（`rpc.proto` 已占用 `conex.Message`）。记录本身位于 `schema/conex/entities.proto`，事件/游标/快照位于 `events.proto`，操作输入输出位于 `agent_domain.proto`；此分层避免 `events → records` 与 `operations → events` 的循环导入。

Message.parts 首版为 `text:{text,offsetBytes,totalBytes}` 或 `tool:{name,phase,summary}`；text 是该 part 的明确窗口，offsetBytes/totalBytes 为 uint64 字符串。窗口外内容通过 agent/messages/read 分段取回，不把分页标成后端丢失。单个窗口最多 4096 UTF-8 bytes；snapshot 可返回空窗口和正确 totalBytes，客户端再读取正文。不承诺工具原始任意对象或二进制附件；真正无法表示/补读的后端内容才标记 incomplete 与原因。工具 summary 不包含凭据或审批秘密。流式事件携带 messageId、partIndex、offsetBytes、text，offset 从该文本 part 的 UTF-8 起点计算。

Conex 映射存节点数据库，后端保留原生历史；客户端不会取得任意 sessionFile 的读写权。稳定 Conversation 与 Workspace 跨节点进程重启保留；运行实例、连接、审批有效性不能仅凭稳定 ID 复活。

## 3. 能力声明

`capabilities` 是明确布尔记录：createConversation、continueConversation、readHistory、streamMessages、cancelExecution、approval、userInput、listSubagents、cancelSubagent、steerExecution、attachExternal、recoverActive。未声明或 false 即不支持。必需共同主路径在适配器验收时真实验证，不能为通过检查谎报 true。

`agent/endpoints/list` 还返回实际协议版本、experimental 标志和限制说明；界面禁用不支持操作，并保留错误显示。不由客户端要求打开某后端的所有 experimental API。

## 4. 操作表

记号：`Page={items,nextPageToken?}`；limit 默认 50、范围 1–100。页 token 是节点生成的受限不透明定位，绑定查询、主体、目标及截止时间，不能当授权使用；有效期 5 分钟。会话/执行/交互按 `(createdAtMs,id)`，无创建时间的端点/Workspace/关系按 id 稳定顺序分页；不承诺翻页间的全局事务快照，已删除记录可缺失，同 ID 去重。未知 token 返回 bad_request/reason=invalid_page_token，过期要求重新列表。

| method | argument | result | 权限 / 副作用 |
|---|---|---|---|
| agent/endpoints/list | `{limit?,pageToken?}` | `Page<AgentEndpoint>` | read / 无 |
| agent/workspaces/list | `{endpointId,limit?,pageToken?}` | `Page<Workspace>` | read / 无 |
| agent/conversations/list | `{endpointId,workspaceId?,limit?,pageToken?}` | `Page<Conversation>` | read / 无 |
| agent/conversations/create | `{endpointId,workspaceId,idempotencyKey,title?}` | `{conversation}` | create / 有 |
| agent/conversations/get | `{conversationId}` | `{conversation}` | read / 无 |
| agent/conversations/history | `{conversationId,beforeMessageId?,limit?}` | `{messages,hasMore,quality,gaps}` | read / 无；历史次序及缺口见 §8 |
| agent/messages/read | `{conversationId,messageId,partIndex,offsetBytes,maxBytes?}` | `{text,offsetBytes,nextOffsetBytes,totalBytes,state,revision}` | read / 无；maxBytes 默认4096、最大65536；UTF-8边界分页 |
| agent/executions/submit | `{conversationId,idempotencyKey,content}` | `{execution}` | execute / 有；content 是非空文本，最大 64 KiB UTF-8 |
| agent/executions/get | `{executionId}` | `{execution}` | read / 无 |
| agent/executions/cancel | `{executionId,expectedGeneration}` | `{executionId,requested,state}` | cancel / 有；仅请求，不承诺已停止 |
| agent/interactions/list | `{conversationId,executionId?,limit?,pageToken?}` | `Page<InteractionRequest>` | read / 无 |
| agent/interactions/respond | `{interactionId,expectedRevision,decision}` | `{interaction}` | interact / 有；§7 CAS |
| agent/relationships/list | `{conversationId,limit?,pageToken?}` | `Page<Relationship>` | read / 无；只投影有权端点 |
| agent/subagents/list | `{executionId,limit?,pageToken?}` | `{items,nextPageToken?,quality}` | read / 无；不支持返回 unsupported_capability |
| agent/subagents/cancel | `{executionId,expectedGeneration}` | `{executionId,requested,state}` | cancel / 有；须 cancelSubagent=true |
| agent/executions/steer | `{executionId,expectedGeneration,idempotencyKey,content}` | `{executionId,accepted}` | execute / 有；须 steerExecution=true；不创建第二顶层槽 |
| events/snapshot | `{conversationId}` | §恢复契约 | read / 无 |
| events/read | `{cursor,limit?}` | §恢复契约 | read / 无 |

所有状态改变在节点验证有效授权和当前实例后才准入；正文大小按完整编码测量，过限 payload_too_large。pageToken/幂等键长度上限 128 UTF-8 字节，幂等键必须非空，不含秘密。禁止输入 Principal、任意 backendId、cwd、命令、环境变量或审批策略覆盖。后台启动参数由批准的适配器配置产生。

`create` 原子创建节点侧空会话与幂等记录（operation=create，摘要含 endpointId/workspaceId/title）；首次受控后端创建/恢复成功才登记真实 backend_mappings 行，未取得后端 ID 不填假映射。继续会话不另起专用 RPC，使用既有 conversationId 执行 submit，适配器受控 resume。后端不支持列出全部历史时，list 仅列出本节点已登记会话，端点限制说明标明 catalogScope=node-managed，不能声称扫描了所有原生会话。

messages/read 仅允许已获准会话内的 messageId；offsetBytes 必须是已存 UTF-8 边界，超出 totalBytes 返回 bad_request。以同一数据库读版本返回追加式 part 的已提交前缀，旧字节不可重写；增量读取在返回的 nextOffsetBytes 接续。state=streaming 时 totalBytes 还会增加，客户端以事件 offset 衔接，不凭一页末尾判完成。message.created 的文本窗口为空，正文由有界 delta 追加，64 KiB 用户输入也不能装进一个超过 8192 bytes 的 created 事件。客户端保存按绝对 offset 定位的片段，不把 snapshot 尾部窗口当整条消息起点；相同区间不同内容必须 resync。

## 5. 提交、幂等与单执行槽

幂等记录键为 `(principalId,nodeId,endpointId,operation,idempotencyKey)`，记录规范化参数摘要及结果对象 ID；参数摘要包括 Conversation/Workspace 与内容，正文不进入中心日志。保存至少 7 天；事务查找先处理 expiresAtMs<=now 的行，删除后视为新键，不依赖后台 prune 时机。过期后同键可能创建新操作，不能安全重试；客户端超过窗口只能查询/人工重新提交，不自动重发。

1. 先授权，再在 SQLite `BEGIN IMMEDIATE` 中查同键；同摘要返回原对象当前投影（不是新后端执行），不同摘要返回 conflict/reason=idempotency_mismatch。
2. 校验会话及实例；顶层 `activeExecutionId` 非空即 conflict/reason=conversation_busy，返回已授权的 activeExecutionId。busy 不入队，也不消耗该幂等键。
3. 原子创建 Execution(state=accepted)、占槽、记幂等记录、写事件，再提交。没有已提交的 Execution 不向后端发送。
4. 分派前事务将状态转为 dispatching，再向固定实例发送一次；不能持有数据库锁等待后端。发送后无法确定结果进入 unknown。未发送且能证明拒绝才进入 failed。
5. 已接受的执行即使 RPC 回执丢失也可通过同幂等键查回。上层路由仍不自动重放副作用；领域同键查询式重试必须显式由客户端执行，且不能另换键绕过未知。

状态：`accepted → dispatching → running → succeeded|failed|cancelled`；非终态可转 unknown，unknown 经后端核对恢复 running 或确定终态。取消另设 `cancelRequestedAtMs`，不制造一个可误作终态的 cancelled 回执。消息结束并不使 Execution succeeded；必须等待后端活动静止。

终态只允许一次 CAS 转换；终态不再切换为另一终态，迟到冲突事件保留诊断而不覆盖。终态事务原子清除本执行拥有的槽、解决/失效剩余交互并写事件。派生子执行设置 parentExecutionId，不再抢顶层槽；顶层是否静止必须包含后端报告的后台活动，而不是只计本地已发现的子记录。

节点恢复先依据持久发送边界区分：仍为 accepted（从未提交 dispatching）的执行，在“发送前必须持久化 dispatching”的不变量下可证明未发送，原子标 failed(reason=not_dispatched) 并清其槽；绝不自动补发。dispatching/running/unknown 保留占槽与旧 backendRef、转 unknown 并核对。没有可靠核对接口时保持 unknown，向用户说明需要后端明确终止/人工核对；不能用超时清槽。持有原实例所有权且确认该后端退出后，可通过 Quiescent(state=failed,reason=backend_terminated) 清槽，旧交互全部失效；不能仅凭 PID 数字不存在或被复用判断实例身份。外部后端不能由适配器擅自 kill 来清槽。

## 6. 取消与故障

cancel 固定 `(executionId,moduleInstanceId,generation,backendRef)`；不得重新 resolve 到新实例。重复取消同一非终态执行返回 requested=true；已终态返回 requested=false 与当前 state。不支持取消返回 unsupported_capability，不伪造停止。unknown 且原运行实例不可达时，cancel/steer 返回 unavailable/reason=backend_unreachable，不能以新代际执行命令；新适配器只可通过保存的稳定 backendRef 做核对，并提交原执行的已证实观察，不借核对重新分派输入。

执行先自然完成时，取消不会把 succeeded 改 cancelled；只有后端确认取消导致活动停止才置 cancelled。消费者断开、module/cancel（撤销等待）、客户端窗口关闭均不自动发 agent/executions/cancel。取消后的工具外部副作用不承诺回滚。

## 7. 人工交互与竞答

InteractionRequest.state 为 `pending → responding → resolved|expired|invalidated|unknown`。unknown 是已尝试向后端响应但结果不明，禁止第二次回答；只能核对。prompt/choices 可在客户端显示但不写入中心日志。

- decision 形态：approval `{kind:"approval",choiceId}`；selection `{kind:"selection",choiceIds:[...]}`；input `{kind:"input",text}`。选择必须来自该请求的 choices，选择数量及文本限额由请求约束；额外字段拒绝。
- choices 为 `{id,label,effect}`；effect 只接受本次后端确实支持的 allow_once/deny/select/submit。后端附加范围明确显示，首版不把 allow_once 升级为 session/global allow。
- respond 事务检查主体/执行/实例/有效期、expectedRevision 与 pending；CAS 为 responding 并记录确定 decision，再发送一次。两端竞答只有一端赢；revision 不符 stale_revision，已被处理 conflict/reason=interaction_consumed，不能再次发送。
- 同一已成功决定的同主体重试、相同 expectedRevision/decision，返回当前 resolved 结果；不同内容返回 conflict。不把 unknown 当成成功回显。
- expiresAtMs 若后端没有截止时间，由节点上限 10 分钟产生；到期拒绝客户端新答，尝试后端的明确拒绝/取消接口，失败则标 unknown 并保持执行占槽。过期不自动批准。
- 客户端断线不改变 pending。后端断线后的审批须核对；恢复无法确认同请求仍有效就 invalidated，不重放旧批准。未知 server request 返回支持协议的拒绝或显式 unsupported，不能无限挂起或默认批准。
- OMP 18.13.2 的 ask timeout/cancelled+timedOut 可能选recommended或首项，不等于拒绝；适配器禁止把Conex过期/断线直接翻译为这种默认回答。必须选择实际明确拒绝项，无法无歧义拒绝时abort并等待真实静止/核对；后端自带会触发默认许可的timer也必须在开放能力前禁用或置于拒绝安全策略，不能用“尚未回答”承诺永不执行。工具输入主路径使用已实测可发select的rpc-ui模式，不能把rpc普通展示setWidget当人工交互。

## 8. 历史、关系与后端限制

节点 Message ID 与后端 item/message ID 显式映射；消息按节点记录的稳定 ordinal 输出，历史接口只向前翻已结束消息，活跃消息从 snapshot/events 获取，避免历史页与 delta 双写。历史接口 beforeMessageId 必须属于同会话且已获准；未知边界返回 bad_request，不按文本猜位置。后端未能补读时 `quality=unknown`、gaps 给出缺失范围/原因，不能返回空数组谎称历史完整。

Subagent 关系必须有后端 ID/父执行证据；无法提供则质量 unknown 或能力 false。Conversation、Execution、Workspace、Node 的关系使用实体 ID，与传输可达性分开。标签的 node/user 所有权遵守 private-runtime；agent/relationships/list 不允许通过关系线取得不获准的执行内容。

| 后端 | 规范映射 | 必须以实际版本证据确认 |
|---|---|---|
| OMP RPC/rpc-ui | 一个受控进程一个活动会话；prompt 对应执行；message_update 对应消息事件；abort 是取消请求 | 已观察prompt_result completed/aborted和isSettled；rpc-ui的select答复；tool approval与后台session_settled仍需Adapter验收；get_subagents生命周期有父工具关联 |
| Codex app-server | thread 对应后端会话；turn 对应执行；item 对应消息/工具；turn/interrupt 为取消请求；本版本正文增量为item/agentMessage/delta | 已观察turn/completed、interrupted、requestApproval→decline；实验关系需显式协商；resume/历史补读与断线审批有效性不由普通turn成功推断 |

上表是映射规则而非 live 通过声明；具体观察、错误及未覆盖项以协议证据页为准。已有 OMP TUI 附着不是恢复会话文件，首版 RPC 主路径不能冒充已实现 TUI 接管。

## 9. 机器错误

复用既有 ErrorCode 数值，不新增同义错误码。`details.reason` 为本契约枚举字符串，`diagnosticId` 非秘密关联 ID；reason 不取代执行确定性。

| 情况 | code / reason | execution / retry |
|---|---|---|
| 无效字段、路径输入、非法选择 | bad_request / invalid_argument | not_started / never |
| 未授权 | forbidden / permission_denied | not_started / never |
| 会话已占槽 | conflict / conversation_busy | not_started / never |
| 幂等内容冲突 | conflict / idempotency_mismatch | not_started / never |
| 代际或交互 revision 不符 | stale_revision / generation_mismatch 或 revision_mismatch | not_started / never |
| 不支持可选操作 | unsupported_capability / backend_unsupported | not_started / never |
| 已处理、过期交互 | conflict / interaction_consumed 或 interaction_expired | not_started / never |
| 资源不存在（已授权范围） | unknown_provider / entity_not_found | not_started / never |
| 后端不可达且未分派 | unavailable / backend_unreachable | not_started / never |
| 已分派结果不明 | outcome_unknown / backend_outcome_unknown | unknown / never |
| 事件游标不再可续 | resume_unavailable / resync_required | not_started / safe（只读） |

读操作超时可 safe；任何有副作用操作跨过可发送边界一律 unknown/never，由显式同键核对而非通用重试器处理。日志不含参数正文。所有错误字段还受既有 wire 的严格编码约束。

## 10. 必须保留的行为回归

- 两个数据库连接同时 submit 同会话：仅一个后端分派槽；另一方 busy；相同幂等键得到同一执行，换参数冲突。
- 已分派崩溃重启：旧执行 unknown、槽仍占用、未重发；后端确认完成后才清槽。
- 两客户端同 revision 回答：仅一次后端发送；相同成功答复可查回，不同决定冲突；响应结果未知不得重发。
- 自然完成与取消同时到达：一个终态、一个释放槽事件；客户端断线不停止执行。
- 非授权路径/后端 ID 输入被拒绝；关系查询不能扩大授权；未知能力不伪造成功。
