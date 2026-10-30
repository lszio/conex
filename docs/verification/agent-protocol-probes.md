# Agent 协议探针：P0 实际证据

日期：2026-10-29。范围：[P0计划](../plans/2026-10-29-conex-mvp-p0.md)。这是协议调查，不是MVP交付；A01–A16产品验收尚未执行。

## 1. 执行与隔离

使用用户指定的`bs/DeepSeek-V4-flash`启动独立OMP CLI worker：私有契约worker完成；协议worker超过3600秒命令截止时间，未写最终报告。主会话接管其已写出的探针/临时JSON记录，审查驱动，并补做实际元数据/schema、交互拒绝和Codex外部服务重连续读。不能把worker超时标成协议全部通过。

全部模型实验限定自己创建的临时Workspace与会话/进程，不连接现有活跃TUI；没有登录/导出凭据、改provider、安装依赖、提交或部署。OMP现有授权可用；Codex现有授权可用。临时Codex WS服务器绑定127.0.0.1，使用探针随机生成的独立capability-token，不复用/输出用户秘密。主会话实验的进程在finally terminate/wait，必要时kill/wait；只处理自己创建的进程。临时驱动/原始schema不是产品第二套SDK，完成后清理。

元数据永久入口：

```sh
omp --help
codex --version
codex app-server --help
python3 scripts/probe-agent-protocols.py
```

版本实读：OMP **18.13.2**；Codex **0.163.0**。脚本实际退出0：

```json
{"omp":{"ok":true,"ready":true},"codex":{"ok":true}}
```

脚本不调用模型，只发get_state/initialize。下述live结果另外记录，不拿握手代替真实交互。

## 2. 安装版本 schema

主会话再次在独立临时目录执行两种导出，均退出0；不是直接依赖在线最新文档：

```sh
codex app-server generate-json-schema --out <owned-unique-temp-directory>
codex app-server generate-json-schema --experimental --out <another-owned-unique-temp-directory>
```

| 模式 | JSON文件数 | 合并bundle字节 | `codex_app_server_protocol.schemas.json` SHA-256 |
|---|---:|---:|---|
| stable | 318 | 730500 | `408e09849d5d4931992a4227a7f3eb80b15d24901f84aca0562e816634651a82` |
| experimental | 452 | 890029 | `43e8c1495557ca91e33a63bbfae36c311c206253afae2974c979dbdf5d940c71` |

读取结构：v2/ThreadStartParams、ThreadListParams、ThreadReadParams、CommandExecutionRequestApprovalResponse、ServerRequest/Notification。thread/list的parentThreadId/ancestorThreadId受experimentalApi控制；cwd过滤器为精确工作目录，可限制探针清理到自己的数据。thread/read includeTurns完整历史hydration已标deprecated，正式Adapter优先分页thread/turns/list与thread/items/list；不能用空items断言历史不存在。

OMP参考安装包`rpc.md`，实现定位：packages/coding-agent/src/modes/rpc/{rpc-mode,rpc-types}.ts、session/agent-session-events.ts。它是自定义JSONL，不是JSON-RPC2。ready支持协议1/2；v2超大帧chunk重组、物理帧1MiB/逻辑64MiB。正式Adapter须保留自己的限额，不照搬无界输出spool语义。

## 3. OMP：worker记录与主会话补证

worker隔离驱动：`omp --mode rpc --session-dir <own-temp> --no-extensions --no-skills --no-rules --no-context-files --no-lsp --cwd <own-temp>`，使用现有模型。主会话读取其JSON记录及驱动，以下行明确为worker记录，而非主会话独立重跑：

| 场景 | 实际记录 | 结论/边界 |
|---|---|---|
| 纯文本prompt | message_start/end各2，message_update18；prompt_result completed、sessionSettled=true | 真模型完成，不以agent_end单独判静止 |
| 同会话继续 | sessionId相同；第二次prompt_result completed；history4条user/assistant/user/assistant | 稳定会话与继续路径可用 |
| 中途取消 | 4.01秒后abort response success；prompt_result aborted、sessionSettled=true；isStreaming=false/isSettled=true | 已观察取消后的静止，不只abort回执 |
| 子任务 | subagent_lifecycle started：id=ListDir、agent=scout、parentToolCallId存在；后续event/progress | 能记录真实派生来源；末次get_subagents=[]不证明所有历史都可查询 |
| 重启恢复 | 自有旧进程exit0；新进程open_session resumed=true、相同sessionId、history15条、isSettled=true；新进程exit0 | 已完成历史恢复可用，不是接管活跃TUI/恢复中途审批 |
| 普通rpc人工交互尝试 | 收到setWidget展示；写文件/澄清两次interaction=null | **未覆盖人工拒绝**，setWidget不是审批；不计通过 |

主会话补做真实工具输入：改用文档明确支持工具UI的`--mode rpc-ui`，`--tools ask --approval-mode always-ask`，同样禁用自动发现上下文/扩展、临时目录、指定bs/DeepSeek-V4-flash；要求ask提出“继续/拒绝”。观察到extension_ui_request.method=select，驱动提交拒绝安全回答，最终prompt_result completed、sessionSettled=true。共message_update60、tool_execution_start/end各1。主会话只输出方法/终态计数，不记录问题秘密。

重要安全差异（安装文档已实读）：ask timeout或`cancelled:true,timedOut:true`可能选recommended/首项，**不能当拒绝**。select/input取消为undefined、confirm取消为false。Agent契约已要求按真实请求显式拒绝，无法拒绝时abort并核对；正式Adapter还必须证明后端timer不会在断客户端后默认放行。OMP工具审批本身（区别于ask输入）、后台任务与session_settled交错、活动审批重连仍属于P3必要live验收；不能把一次select当这些全部通过。

## 4. Codex：真实执行、取消、拒绝

worker用自有stdio app-server，thread/start approvalPolicy=on-request，临时cwd。记录：

- 新thread两次turn，threadId稳定，两个turn均completed。
- turn/interrupt回执成功，最后turn状态interrupted。
- 审批尝试未出现approval_surface，不计拒绝通过。
- experimentalApi=false时parentThreadId/ancestorThreadId查询明确返回`requires experimentalApi capability`；不能声称默认支持。
- 创建的该thread/delete返回成功，app-server退出0。

worker初版驱动只收集筛选后的turn/completed，且查历史未完整hydration；它的“没有delta/没有items”不足以证明协议不支持。主会话补证使用真实协议，不改后端实现：

### 4.1 人工审批拒绝（主会话实际运行）

启动自有stdio server，initialize后发送initialized；thread/start使用`approvalPolicy=untrusted, approvalsReviewer=user, sandbox=read-only, ephemeral=true`。要求运行只打印测试标记的python命令，不写文件/不联网。收到真实服务端请求：

```json
{"method":"item/commandExecution/requestApproval","resultSubmitted":{"decision":"decline"}}
```

回答decline后观察serverRequest/resolved、turn/completed(status=completed)。拒绝某命令后turn可正常完成，不能把turn completed翻译成“命令获准”。观察item/agentMessage/delta **35**帧、item/started与completed各4。此版本实际delta名含`item/`前缀，旧假定agentMessage/delta会漏消息。临时ephemeral thread/delete返回错误（不标清理成功），进程由创建者回收；未声称它写入持久会话库。

### 4.2 外部服务所有权与已完成会话恢复（主会话实际运行）

Unix socket+proxy尝试在90秒期限未取得完整结果，创建者已回收server；**未标通过**。转用安装help明确支持的认证WebSocket接入，非绕过认证：

1. 自有server监听`ws://127.0.0.1:<ephemeral-port>`，`--ws-auth capability-token --ws-token-sha256 <own-digest>`；token随机、只放Authorization头。
2. 消费者A initialize→initialized→thread/start(read-only/untrusted)→turn/start“只回复标记，不使用工具”。
3. 收到turn/completed=completed，实际item/agentMessage/delta5帧。
4. A关闭socket；server仍存活。消费者B重新认证initialize→thread/read(includeTurns=true)；相同threadId，1个turn、2个items；thread/resume成功。
5. B仅删除本实验thread，delete成功；关闭B连接；最后创建者terminate/wait回收server。

脱敏结果：

```json
{"turnStatus":"completed","serverSurvivedDisconnect":true,"sameThread":true,
 "turnCount":1,"itemCount":2,"resumeOk":true,"ownThreadDeleted":true,"creatorReapedServer":true}
```

此前只创建未执行的空thread时，跨消费者read未成功；没有足够错误记录解释其原因，不能将其推论为所有会话不可恢复。完成真实turn后的路径已取得上述正面证据。活动执行的无损delta恢复仍未证明；正式P4需保留unknown/失效处理并单独测。

### 4.3 待审批时消费者断开（主会话实际运行）

独立认证WS服务器、read-only/untrusted/user审批。观察到item/commandExecution/requestApproval后，**不回答**，直接关闭消费者A；server仍存活。消费者B重新认证并thread/resume成功；thread/read(includeTurns=true)显示turn仍为inProgress、commandExecution状态列表为空。没有证据证明旧审批已重放或变成了可安全重答的新请求，不能据此自动重发批准。实验最后显式turn/interrupt回执成功、只删除本实验thread成功、由创建者回收server。

```json
{\"approvalObservedBeforeDisconnect\":true,\"serverSurvivesDisconnect\":true,
 \"resumeOk\":true,\"turnStatusesAfterReconnect\":[\"inProgress\"],\"commandStatuses\":[],
 \"cleanupInterruptAccepted\":true,\"ownThreadDeleted\":true,\"creatorReapedServer\":true}
```

产品处理已冻结：等待活动/审批状态核对；无法确认旧请求有效就invalidated，执行仍保持unknown/占槽，不把重新resume当成旧审批重放成功。interrupt回执本身不充当已取消终态证明。

## 5. 能力差异与产品处理

| 操作/边界 | OMP | Codex | 状态与产品处理 |
|---|---|---|---|
| 元数据/版本 | ready/get_state、协议2 | initialize、安装schema | 本轮主会话实测 |
| 创建/继续/历史 | 一个受控进程活动会话，节点管理目录 | thread/turn/item，分页API | OMP worker与Codex主会话live；不扫描任意用户会话 |
| 流式/完成 | message_update、prompt_result+settled | item/agentMessage/delta、turn/completed | live；消息结束不直接等于整个执行静止 |
| 取消 | abort→aborted/settled | interrupt→interrupted | worker记录；P3/P4正式入口需重验 |
| 人工输入/审批 | rpc-ui select拒绝已测；工具approval仍待测 | requestApproval→decline已测 | 禁止默认批准，未覆盖能力不能谎报 |
| 子执行关系 | lifecycle/progress含parentToolCallId | parent/ancestor查询experimental gating | 真实OMP started关系；Codex实际派生/完整查询仍未覆盖，标unknown/不支持 |
| 已完成会话恢复 | owned进程重开相同session历史 | authenticated WS消费者重连同thread读取/resume | 已观察，不等于活动审批可恢复 |
| 外部所有权 | RPC自有进程；活跃TUI附着未测 | WS外部服务不随消费者退出 | 后者主会话已测；前者不冒充TUI接管 |
| 无损delta补读 | 后端不能承诺曾有完整delta | 历史hydration不等于重放每个delta | 节点持久流提供恢复，缺口明确gap/resync |

P0可据此固定领域/恢复/安全契约。未知项的安全产品处理为：能力明确false/unknown、执行保持unknown并占槽、旧审批invalidated、不重放命令、不自动允许；P3/P4仍必须完成各自必需live，不因此删掉A05/A06/A09。P0不是“所有后端能力已认证”。

## 6. 计划与验证边界

已写三项专项契约、P1/P2/P5代码计划及A01–A16覆盖表。P1计划SQL在临时SQLite实际执行，验证唯一顶层槽、unknown占槽、子执行允许、审批CAS行数1/0、消息state/revision拒绝，具体非产品测量见event-recovery。

未运行Rust产品Store（尚未创建）、新Web/Emacs、两机公网、节点OS沙箱、完整MVP门禁。所有私有grant/票据/撤权规则是规范决策，不是本轮已经实现并运行的安全产品。没有提交或部署。

## 7. 最终交付检查

主会话实际检查11份本轮Markdown、152条相对链接：无断链，代码围栏平衡；Python代码与永久元数据脚本一致且AST解析成功；应用包TOML解析成功；A01–A16覆盖齐全。P1初始schema与P2追加表在同一临时SQLite实际执行，unknown顶层槽拒绝第二顶层、子执行允许、审批CAS两次行数1/0。

按本轮明确创建的14个临时cwd限制Codex thread/list清理查询，返回0个剩余持久thread；没有查询无关会话。临时worker脚本、JSON报告、schema目录与Workspace已按确切路径移除；自己的活跃探针进程检查为0。永久只保留无模型元数据探针，live调查序列与脱敏结论保留在本页，不留下第二套产品协议客户端。

bs/DeepSeek-V4-flash只读审查：领域恢复的两个阻断项（未分派幽灵占槽、无界快照）已复审确认修正；剩余用户消息事务、restore换epoch与审批失效路径已补入P1。私有运行/P2/P5最终审查结论为无剩余具体阻断，签发TTL与逐路purpose校验的提示也已写明。此结论是规范/计划一致性审查，不是已实现安全产品的渗透测试。交付前再次运行永久元数据脚本，退出0，OMP ok/ready与Codex ok均为true。
