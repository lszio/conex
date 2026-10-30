# 持久领域事件、快照与恢复契约

状态：P0 规范，不表示持久事件服务已上线。依据：[Agent 领域](agent-domain.md)、[私有运行](private-runtime.md) 与 [架构](../design/2026-10-29-conex-private-platform.md)。适用节点 SQLite 的 Conversation 恢复；不增加全局总序或持久中心队列。

## 1. 两种机制，不重复造传输

- S1 `state/subscribe` 的完整状态与 seq 只负责一条连接的快照/通知，断线不恢复该订阅。
- 持久事件属于节点领域存储，以 `module/invoke` 下的 `events/snapshot` 和 `events/read` 获取。现有 `state/subscribe` 的状态源 `agent.eventHead`、argument=`{conversationId}`，发送完整 `{streamId,epoch,head,firstAvailable,quality}` 作为读取提示；不在通知里携带聊天正文。
- 客户端收到 head 提示后按 cursor 取事件页，合并提示并限制一次一个 read 在途；有 hasMore 时继续读取，不对每个 token 建一次并发请求。两客户端复用同一节点流。
- 读与订阅仍按原契约和路径授权。中心可以获准瞬时转发 read 的正文，但不得写入通用回放缓存、日志或数据库。direct-only 的 head、快照、事件读和派生内容均不能通过中心。

因此无需新增顶层 events/subscribe 方法，不改 S1 `state/update` 的完整状态语义。未来 push event transport 需显式协商，不是首版隐式支持。

## 2. 流、游标与事件

一个 Conversation 一个 stream，`streamId=conversationId`；`epoch` 是创建该流时的 ULID。正常节点/模块/连接重启不改变 epoch；从备份恢复或重建事件历史必须换 epoch，并使旧 cursor 明确失效。registrationId/module generation 不代替 epoch。

Cursor 为 `{streamId,epoch,after}`，after 是最后已应用位置的 uint64 十进制字符串。空流 after=`"0"`。位置从 1 连续递增，不复用、不按时间生成。SQLite 首版只使用 0..9223372036854775807；到上限停止写入并报告 quota_exceeded，不溢出或归零。cursor 不携带授权；每次读验证当前认证/授权。

事件结构：`{streamId,epoch,position,recordedAtMs,type,entityId,entityRevision,data}`。时间只用于展示/保留，不用于排序。去重键为 `(nodeId,streamId,epoch,position)`；同键不同内容视为协议故障，终止同步并要求重同步，不能静默覆盖。

类型与 data：

| type | data |
|---|---|
| conversation.updated | `{conversation}` |
| execution.updated | `{execution}` |
| interaction.created / interaction.updated | `{interaction}` |
| message.created | `{message}`，文本窗口为空；后续有界 delta 写入正文 |
| message.delta | `{messageId,partIndex,offsetBytes,text}`；仅追加文本 |
| message.completed | `{messageId,revision,state}`；state=complete/incomplete |
| relationship.updated | `{relationship}` |
| recovery.gap | `{reason,backendRange?}`；明确不能恢复的后端区间 |

revision 属于实体，与事件位置不同。事件写入时携带更新后的 revision。单个投影事务可产生多个事件；相应位置连续，观察者不能读到未提交的半个事务。message.delta 的相同去重键只追加一次，offsetBytes 必须等于当前 part 的已知 totalBytes；客户端可以尚未加载较早窗口，但不能缺少从 snapshot 边界之后的连续 delta。窗口中的相同字节区间不符要求 resync，不能靠拼接文本猜重。

## 3. 快照与读取

`events/snapshot` argument=`{conversationId}`。在同一个 SQLite 只读事务内读取：

```text
{conversation, executions, interactions, messages, relationships,
 cursor:{streamId,epoch,after:head}, firstAvailable,
 history:{hasMore,beforeMessageId?}, relationshipsHasMore, quality,gaps}
```

executions 包括全部活动顶层与已知活动子执行，另含预算内最多 20 个终态；interactions 包括所有未确定交互及预算内最多 20 个已解决项；messages 返回最近最多 100 条和全部活跃消息的元数据与有界文本窗口；relationships 返回预算内最多 100 条，剩余以 relationshipsHasMore 标记并经 agent/relationships/list 分页。所有可选历史项可减少，正文窗口可缩至空并带正确 totalBytes，客户端通过 agent/messages/read 分页读取；不把分页标成 incomplete。关键活动集合按下表硬限额保证可编码；超限后端输入不能无限追加到快照，需记录有界 gap/诊断、将执行置 unknown 并隔离端点，拒绝新输入且不自动批准。不能截断关键审批语义后伪装完整。

快照所有投影和 cursor 来自同一事务版本，不能先查 messages 后另查 head。正文不出现在 head 通知中。历史接口补读早期已结束消息，不拿历史页改变当前活动投影。

`events/read` argument=`{cursor,limit?}`；limit 默认 32、范围 1–32。result：

```text
{events:[...], nextCursor:{streamId,epoch,after:lastReturnedOrInput},
 head,firstAvailable,hasMore}
```

head/min/事件页同一读事务取得。返回严格 after 之后连续事件，按编码字节预算取尽可能多的完整事件；不拆半个事件。空页仅在 after=head 时合法；尚有事件却因单条过大而空页必须报 payload_too_large。hasMore 表示 nextCursor.after < 本次 head，不承诺之后没有新事件。超过 head 的 cursor 返回 bad_request/reason=cursor_ahead，不帮调用者跳过事件。

第一次连接流程：

1. 获取 snapshot，原子替换客户端投影并记录 cursor。
2. 订阅 `agent.eventHead`，等待 S1 initial snapshot；不用该订阅 revision 或 seq 覆盖持久 cursor。
3. 立即 events/read(snapshot.cursor)，循环至本次 head；处理之后的 head 提示。同一流只有一个读取循环。
4. 每 5 秒主动读取一次 head/read 作连接健康与遗漏核对，返回空页时不生成虚假业务事件；head 提示仅加速，不是唯一真相。
5. 断线后显示 stale；重新认证、发现新 registrationId、建立新 head 订阅，从旧持久 cursor 读取。过期则 fresh snapshot 原子替换，绝不重发旧输入/审批。

这样 snapshot 与 subscribe 之间即使发生提交也能在日志续读，不依靠跨两个 RPC 加锁。保留窗口可能在其间失效，必须显式重同步而不是承诺无限不丢。

## 4. 写事务、后端与崩溃

同一节点数据库事务执行：校验状态/CAS → 更新投影 → 分配连续 position → 写完整事件 → 更新 head/byte count → commit。提交后才发 head 通知；通知失败不回滚业务提交。数据库 I/O 在专用阻塞工作线程执行，不在 async executor 中持锁等待网络或后端。

外部后端执行不能纳入 SQLite 事务。先持久化意图/占槽，再在分派前持久化 dispatching，发送一次。恢复时 accepted 按发送边界不变量确认为从未分派，记 failed/not_dispatched 并清槽；dispatching/running/unknown 保留占槽并核对，不自动重放。后端补读按 backend message/item ID 去重；缺失原始 delta 时可用真实历史重建快照，但必须产生 recovery.gap，不伪造曾记录到的连续 delta。

审批的 responding/decision 与事件同事务；是否已抵达后端无法确认时状态 unknown，严禁另一个客户端再答。节点重启后旧待审批须核对有效性，不能凭数据库 pending 自动批准。

## 5. 数值限额与保留

下列是首版规范默认值；不是已经实现或生产容量承诺。实现可降低协商上限，不得宣称比实际传输更大的帧预算；调用双方使用所有跳最小值。

| 项目 | 默认 / 硬边界 | 超限处理 |
|---|---|---|
| 线上编码帧 | 262144 bytes（沿用 S1 默认） | payload_too_large；发送前编码核对 |
| 事件 read / snapshot result 预算 | 196608 bytes，且给完整信封至少留 16384 bytes；实际取更小值 | 页减少完整事件/历史项；关键快照不完整则错误 |
| 单事件编码 | 8192 bytes | 文本按 UTF-8 边界拆成 <=4096-byte delta；其他事件不能拆则拒绝写入并标记采集故障 |
| 列表/输入 | 领域表 limit<=100；输入<=65536 UTF-8 bytes | 提交前拒绝，不产生执行 |
| read 一页 | 最多 32 个事件且满足字节预算 | hasMore 续读 |
| 活动执行 / 未确定交互 | 每会话 16 / 16 | 新增拒绝 quota_exceeded；真实后端额外活动记录 gap 并隔离该端点，不伪造静止 |
| 活跃消息与元数据 | 每会话最多32条；每条非正文元数据<=2048 bytes、parts<=8、工具summary<=512 bytes；活动Execution元数据<=1024 bytes、Conversation<=2048 bytes | apply前检查，超限进入有界故障/隔离；不累计无限活动项 |
| 交互元数据 | 每条完整编码<=4096 bytes（含prompt与choices） | 超限不能缩掉审批含义或默认批准，执行unknown/端点隔离并报告backend_payload_too_large |
| head 待发提示 | 每流合并为 1 个最新 head；每 Link 最多 64 个订阅 | 只合并 head 提示，不能合并/丢弃持久业务事件 |
| Link 队列 | 1048576 bytes，内含 262144-byte 控制预留；沿用 S1 | slow_consumer/end；控制帧也无法发送则断 Link |
| 单流事件保留 | 最多 24 小时且最多 64 MiB 编码事件，任一触顶清理旧前缀 | 旧 cursor resync_required |
| 节点事件总量 | 最多 512 MiB 编码事件 | 按最老提交时间清理完整流前缀；不删除非终态投影 |
| 幂等记录 | 至少 7 天，最多 100000 条未过期记录/节点 | 到限拒绝新副作用，不提早驱逐有效键 |
| read 预算 / head 核对间隔 | <=8000 ms / 5000 ms | 超时显示 stale，不改变执行终态 |

事件保留字节不等于 SQLite 文件大小；WAL、索引、投影和空闲页另外计量。部署必须监控文件系统容量，不能声称 512 MiB 就是整个数据盘上限。磁盘不足时停止新准入，保留未知状态；不能返回执行成功却丢失事务。自动清理与读取在数据库事务边界协调，清理只更新 firstAvailable、删连续前缀；不等待慢客户端，不保证游标租约。已完成消息可继续由后端历史提供，但事件清理后不伪造原事件。

### P0 编码与 SQLite 实测依据

2026-10-29，Python sqlite3 3.54.0，临时 SQLite/WAL，1000 个 ASCII 4096-byte 文本 delta：单事件编码 4307–4316 bytes；32 个事件页编码 138142 bytes；一次 executemany+commit 10.23 ms；关闭前 db+WAL 9267704 bytes。工作目录/数据库自动删除，无模型、无真实会话数据。此测量只校验编码余量与本机写入量级，不是真实 Adapter 流速、Unicode 最坏大小或生产压测。8192-byte 硬检查/196608-byte 页预算仍须逐帧执行；UTF-8 内容直接编码，不按字符数计算。24h/64MiB/512MiB 是有界单用户起步政策，不由这次 1000 条测量推导持续吞吐能力；后续改变默认需更新契约/配置与保留测试。

## 6. 失效、重同步与撤权

定义 firstAvailable 为当前最早保留位置；空日志但曾写入时为 head+1。cursor 可续接条件是 epoch 匹配且 `firstAvailable-1 <= after <= head`。after=firstAvailable-1 合法；after 再小一位必须 resume_unavailable/reason=resync_required。返回安全的 stream/epoch/min/head 与原因，不附带未授权正文。

以下情况强制重新 snapshot：epoch 更换、前缀已清理、事件重复键内容冲突、delta offset 缺口、后端无法核对投影。客户端用新快照替换而不是拼接旧状态；明确显示 gaps/quality，不能把缺失片段显示为完整记录。节点不可达只显示 stale/unknown，不回退到中心的“最后正文缓存”。

每页与每次订阅更新前重新验证授权和当前代际。授权到期或本地撤权：拒绝后续 read，结束 head 订阅并清队列；已交付客户端的正文无法远程收回，不承诺撤权抹除客户端既有缓存。新中心注册并不让旧注册句柄自动可用，但显式重新发现后可以继续使用同 epoch 持久 cursor。

## 7. 正负验收输入

| 输入/场景 | 必须结果 |
|---|---|
| snapshot at 10，订阅前已写 11、12 | 从 after=10 读取 11、12；无 snapshot 窗口缺口 |
| 读页 11、12 重复交付 | 按同流 epoch/position 去重，消息不重复追加 |
| 同 position 但不同 text；delta offset 不符 | 协议故障与 resync，不猜测合并 |
| firstAvailable=20，after=19 / 18 | 前者正常；后者 resync_required |
| after=head+1、旧 epoch、其他流 cursor | ahead 为 bad_request；旧 epoch resync；其他流需重新授权，不能越界 |
| 读事务中并发清理 | 返回该事务一致页或显式失效，不返回跳号的“连续”页 |
| 发送审批后进程崩溃 | unknown，旧答复不重发，槽不释放 |
| 慢客户端堵满 Link | slow_consumer/end 或断线；重新续读或 resync，不静默丢消息 |
| 中心只做获准转发 | 磁盘/日志无测试正文；direct-only 的 head/read 均拒绝 |
| 撤权后持旧 cursor | forbidden，不能凭游标绕过授权 |

P1 保留数据库并发/CAS/保留边界的真实回归，TS 消费者以临时真实节点存储演练 snapshot→read→重启，不只测试 mock 回显或字段复制。P3/P4 才证明后端 live 内容映射；P5/P9 才证明实际公网权限与双客户端。
