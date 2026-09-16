# P1 流、ACK、信用与恢复契约（设计 §6.2）

状态：**P1-01 冻结**（2026-09-15）。
类型源：`schema/conex/v1/stream.proto`。
向量：`conformance/vectors/p1/stream.json`，Rust（`crates/conex-proto/tests/stream.rs`）与 TS（`sdk/typescript/tests/stream.test.ts`）两侧分类一致。

## 1. 帧逻辑形态

```text
StreamFrame {
  session_id:    ULID,
  attachment_id: ULID,
  epoch:         u64,            # 由 session/resume 原子递增
  stream_id:     ULID,
  seq:           u64 (>= 1),     # 单方向单调递增
  message:       bytes           # 单帧字节；零字节禁止
}
```

- 每条 Stream 单方向；两个方向各用独立 `stream_id`。
- 业务消息先经 `conex-proto` 解析为 `v1::Message`，再用 `StreamFrame` 承载；JSON/Protobuf Profile 各自决定 `StreamFrame` 的 wire 形态。
- `epoch` 属于本帧经过的 attachment，**不能**用一个全局 Session epoch 同时替换所有 peer 的连接。

## 2. ACK

- `stream/ack.last_received_seq` 是已**连续**接收的最大 seq；不是「窗口外的字节通知」。
- ACK 表示接收端已将数据放入承诺的有界接收状态；**不**确认业务执行（业务完成由 Call 结果表示）。
- P1 默认重放状态在进程内；host 重启**不**承诺该状态仍在。

## 3. 信用（credit / flow）

- 信用模型：`consumedBytes + windowBytes`。发送端累计发送字节不能超过二者之和。
- `stream/flow.consumedBytes` 是累计已消费字节（接收端视角）；`windowBytes` 是协商时固定窗口，后续 `flow` 不得用于扩展。
- **重复或旧的 flow**（`consumedBytes <= 之前接受的最大值`）忽略，**不**重复加信用；消费计数超过实际发送量使流失败。
- 零字节数据帧**禁止**发送（防止绕过字节信用消耗 CPU）。
- 信用计费单位：不可变的 `seq + message` 经选定 Profile 编码后的字节数，业务元数据同样计入；有严格大小上限的路由头与传输头不计入信用；完整帧仍受 `maxFrameBytes` 限制。
- 重放复用原消息字节和累计位置，**不**消耗第二份逻辑信用，也**不**因 epoch 头变化重新计费。

## 4. 资源边界

- 默认最大帧 1 MiB、每流窗口 4 MiB、每 Session 接收队列与发送重放缓存各 8 MiB。
- Session 总预算优先于每流预算；另有主体与进程总限额。
- 控制通道（ack/flow/cancel/ping）使用独立有界控制队列与速率限制，**不能**因数据信用归零而死锁；控制队列耗尽时断开 Link 并记录错误。

## 5. 慢消费者与取消

- credit 耗尽先停发；持续阻塞默认 30 秒后可按方法策略取消该流并返回 `slow_consumer`。
- `SlowConsumerSignal` 是接收端通知；`call/cancel` 仍可由发送端主动发起，结果与取消并发时已确定的结果优先保留。
- 数据信用为零仍能取消；`stream/reset` 必须能在零 credit 下成功。

## 6. 恢复期的信用锚定（设计 §6.2）

`session/resume` 成功后接收端必须：

1. 按断线前的 `consumedBytes` 重新广告信用；
2. 新窗口不得小于断线时该流尚未确认的已发送字节；否则发送端会越过 `consumed + window`；
3. 若新 Link 协商出更小窗口，接收端必须先确认足够字节，或显式 `stream/reset` 并把未确认部分标为需重放；**不能**直接用更小窗口把流判为 `slow_consumer`。

## 7. 窗口与重放窗口

- 重放缓存默认最长 120 秒；实际可恢复范围由时间与字节预算共同决定并向对端报告。
- 接收端按 `(sessionId, attachmentId, sender, streamId, seq)` 识别重复，先检查本次 epoch 是否有效；epoch 是连接所有权代次，**不**属于逻辑去重键；恢复**不能**清空去重记录。
- 对端提交的 ACK/消费位置**不能**越过实际发送范围，也**不能**让已丢失缓存重新被宣称可恢复。

## 8. 已知答案向量边界

| 用例 | 期望 |
|---|---|
| 乱序 seq | 接收端把它当作后续重放；不计费直到前序 seq 到位 |
| 重复 seq | 接收端忽略；不计费 |
| 窗口边界（消费恰好等于窗口） | 发送端可继续 ack/flow，但不能继续发数据帧 |
| 重复 flow（更小 consumedBytes） | 忽略；不增加窗口 |
| 旧 flow（超过实际发送） | 流失败（`slow_consumer` / `bad_request`，具体由实现选择） |
| 零字节数据帧 | 拒绝；bad_request |
| 控制队列耗尽 | 断开 Link 并审计（不在 stream 协议层返回） |
| session/resume 后窗口更小 | 接收端必须先确认或 stream/reset，不允许 silent 关闭 |
| `expected_epoch` 与新 epoch 不匹配 | 旧连接被 fencing |

## 9. 待定（占位待定）

- 流携带的业务 `Message` 在 protobuf Profile 下的 wire 形态属于 P1-04；本契约只规定 `StreamFrame` 的语义字段。
- Session 资源租约（设计 §7.1）的具体上限属于 P1-03；流层只读取协商结果，不在流层重定义。