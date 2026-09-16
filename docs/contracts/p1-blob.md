# P1 blob 状态机、持久提交与 GC 契约（设计 §5.4、§5.5）

状态：**P1-01 冻结**（2026-09-15）。
类型源：`schema/conex/v1/blob.proto`。
向量：`conformance/vectors/p1/blob.json`，Rust（`crates/conex-proto/tests/blob.rs`）与 TS（`sdk/typescript/tests/blob.test.ts`）两侧分类一致。

## 1. 状态机

```text
blob/put(open) → uploading → verified → blob/commit → committed
                     ↘ cancel/expire → staging 回收
```

- `blob/put` 只创建/续开上传，**不**承载数据；返回 `uploadId / chunkSize / leaseMs / maxBlobBytes / inlineThresholdBytes / alreadyHaveChunkCids`。
- staging 默认租约 1 小时，可在配额内续期（设计 §5.5）；到期未 commit 的 upload 由服务端回收，已接收字节不进入持久化。
- 任何中间崩溃点恢复后**不得**产生「缺块却 committed」的根（见 §3 崩溃矩阵）。

## 2. 上传/提交原子性

`blob/commit` 一次性原子校验：

1. `declared_root` 与 `contentCid(reassembled bytes)` 一致。
2. 所有叶子块均在该 `uploadId` 上传过且校验通过（`chunk_cid == cid_for_raw(chunk_bytes)`）。
3. `declared_size_bytes == sum(chunk_lengths)`。
4. 当前策略对 `BlobAccess` 授权（`providerEndpoint / plane / spaceId? / resourceId`）。
5. 持久性等级 `persistence` 被后端支持；不支持就拒绝，不允许降级后返回成功。

`committed` 仅在持久性达成后返回；上传 ACK、CID 校验通过、加入内存队列都**不**等于 committed。失败时返回实际状态 + 缺块列表（`bad_blob`）。

## 3. 崩溃矩阵（验收）

| 时刻 | 崩溃后状态 | 验收 |
|---|---|---|
| `blob/put` 返回前 | 上传未建立 | 客户端可见超时；服务端不留 staging 记录 |
| `blob/chunk` 写入 staging 前 | 块未持久 | 客户端重传相同 `chunk_index / chunk_cid`；服务端必须幂等接受 |
| `blob/chunk` 已 fsync 后 | 块持久 | 重启后仍可继续 `blob/commit` |
| `blob/commit` 校验完成、写根引用前 | 持久根未建立 | 客户端重试 commit；服务端必须返回 `committed` 而**不**产生「缺块却 committed」 |
| `blob/commit` 写根引用之后 | 持久根已建立 | 必须返回 `committed`；任何重试必须返回同一 `resource_root_cid / receipt_id` |
| `blob/commit` 部分子块写入后 | 子块持久、根未建立 | 服务端恢复后必须允许同 `declared_root` 的 commit 继续完成 |

矩阵中的每个时点对应 `conformance/vectors/p1/blob.json` 的崩溃用例；测试在两端用 mock 后端复现故障注入并断言「无假 committed」。

## 4. 持久性等级

```text
local          所有块及根引用完成本地持久写入，崩溃恢复可见；不覆盖磁盘/整机丢失
```

`PersistenceLevel` 枚举以 `PERSISTENCE_LEVEL_LOCAL = 1` 为 P1 broker 默认值。`replicated(n)` 属于 P3 范围（P3-a），不进入本契约。

后端必须声明支持的等级；不支持就拒绝，不能静默降级后返回成功。

## 5. Pin 与 GC

- `blob/pin` 返回绑定 `principal / space / root` 的 `pinId`；到期或显式 `blob/unpin` 才能释放。
- GC 只回收不被任何有效根引用且已过保留宽限期的块；历史只追加表示保留期内不原地篡改，**不**表示永不删除。
- 上传/commit 进行中的租约覆盖的块不能被 GC 删除；并发 GC 与 commit 的竞态下，commit 必须在 leaseMs 内完成（否则返回 `unavailable` + 续期指引）。

## 6. ticket / ticket-less 边界

- `blob/*` 操作沿用 `CallContext` 与资源策略；不接受未经授权的根 CID。
- 没有 `expected_root` 的 commit 一律拒绝（bad_request）；客户端必须先拿到 `contentCid` 再发起 `blob/put`。

## 7. 已知答案向量边界

| 用例 | 期望 |
|---|---|
| 单块 ≤ 256 KiB 上传+commit | `committed` 返回 `raw_cid` |
| 多块分块上传+commit | `committed` 返回 `manifest_cid` |
| 块 SHA-256 与声明 `chunk_cid` 不一致 | `bad_blob`；upload 终止但已接收块可保留供重新打开上传续传 |
| `declared_root` 与重算不一致 | `bad_blob` |
| 缺块 commit | `bad_blob` + 缺块 CID 列表 |
| 持久性等级不支持 | `unavailable`，**不**降级 |
| GC 触发 | 不可删 commit 持有的块；可通过 `blob/have` 验证 |

## 8. 待定（占位待定）

- 持久后端选用（`conex-content` 在 P1-06 落定）；本契约只规定接口，不规定实现。
- blob 流的 inline/ref 边界（设计 §5.6）属于 P1-07 工作包；`inlineThresholdBytes` 在 P1-06 实现前以零值占位。