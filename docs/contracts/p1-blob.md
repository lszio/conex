# P1 blob 状态机、持久提交与 GC 契约（设计 §5.4、§5.5）

状态：**P1-01 冻结**（2026-09-15）；§2 的 commit 校验语义与 §7 的用例在 2026-09-16 由 P1-01b 修正并落地于 `crates/conex-content`（见 [P1 计划 §2.1](../plans/2026-09-15-conex-p1.md)）。
类型源：`schema/conex/v1/blob.proto`；分块/根 CID 规则见 [p1-chunking 契约](p1-chunking.md)。
向量：`conformance/vectors/p1/blob.json`（生命周期、崩溃、GC 与 commit 拒绝用例），Rust 侧由 `crates/conex-content/tests/blob.rs` 消费；manifest 根与黄金字节来自 `conformance/vectors/p1/chunking.json`。

## 1. 状态机

```text
blob/put(open) → uploading → verified → blob/commit → committed
                     ↘ cancel/expire → staging 回收
```

- `blob/put` 只创建/续开上传，**不**承载数据；返回 `uploadId / chunkSize / leaseMs / maxBlobBytes / inlineThresholdBytes / alreadyHaveChunkCids`。
- staging 默认租约 1 小时，可在配额内续期（设计 §5.5）；到期未 commit 的 upload 由服务端回收，已接收字节不进入持久化。
- 任何中间崩溃点恢复后**不得**产生「缺块却 committed」的根（见 §3 崩溃矩阵）。

## 2. 上传/提交原子性

`blob/commit` 一次性原子校验。**叶子列表不来自调用方**：服务端只使用该 `uploadId` 自己收到并校验过的块（receipt 中的 `(index, cid)`，索引必须是连续的 `0..n`），否则 `bad_blob`：

1. 叶子数与 `ceil(declared_size_bytes / chunk_size)` 一致（`content_length == 0` 时为 1）；缺块时返回首个缺失索引或缺失块的 CID 列表。
2. `declared_root_kind` 与长度自洽（`content_length <= chunk_size` 必须为 `raw`，否则必须为 `manifest`），否则 `bad_blob`。
3. 服务端按 `conex_proto::cid::addressing_for_parts(chunk_size, content_length, leaves)` 重算根 CID，必须等于 `declared_root`；不等即 `bad_blob`，不建立任何持久引用。
4. 每个叶子块必须存在，且其持久字节长度等于推导长度 `min(chunk_size, content_length - i*chunk_size)`；块被截断/篡改即 `bad_blob`。
5. 规范化 manifest 树的每个节点（含分层 `child_manifest_cids`）作为对象持久保存，并与其叶子一起写入 `refs/<root>.blocks`，使可达集完整（`blob/get` 与 GC 都以此为准）。
6. 当前策略对 `BlobAccess` 授权（`providerEndpoint / plane / spaceId? / resourceId`）。
7. 持久性等级 `persistence` 被后端支持；不支持就拒绝，不允许降级后返回成功。

写入顺序为 `refs/<root>.blocks` → `refs/<root>.record.json` → `refs/<root>.committed`（**commit 点**），随后才更新内存引用计数并清除 staging。

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
| 单块 ≤ 256 KiB 上传+commit | `committed` 返回 raw 根 CID |
| 多块分块上传+commit（`two_chunk_upload_commits_as_manifest`，`rootVector = two_full_chunks`） | `committed` 返回与冻结向量相同的 manifest 根；根对象已持久并被引用 |
| 块 SHA-256 与声明 `chunk_cid` 不一致 | `bad_blob`；upload 终止但已接收块可保留供重新打开上传续传 |
| `declared_root` 与按 upload 自身块重算的根不一致 | `bad_blob`；不产生 committed |
| `declared_root_kind` 与长度矛盾 | `bad_blob` |
| 块被截断/长度不符 | `bad_blob`（长度与 CID 一起校验） |
| 缺块 commit | `bad_blob` + 首缺索引或缺失块 CID 列表 |
| 持久性等级不支持 | `unavailable`，**不**降级 |
| GC 触发 | 不可删 commit 持有的块（含 manifest 对象）；可通过 `blob/have` 验证 |

## 8. 实现落点与仍未接入的部分

- 持久后端：`crates/conex-content`（cap-std 受限根、`blocks/`+`refs/`+`staging/`+`pins/`、全部写走 tmp+rename、`recover()` 重建状态）。持久性等级 `local`。
- inline/ref 边界：`crates/conex-content/src/transfer.rs`，默认 `inlineThresholdBytes = 64 KiB`（设计 §5.6）；`MAX_BLOB_BYTES = 1 GiB`。
- **未接入**：`blob/put|chunk|commit|pin|unpin|have|get|cancel` 的 MethodContract 注册与 `prepare/validate_output` 严格解码仍属 P1-08/P1-10；本节的判定逻辑已在 `conex-content` 内可直接调用，但尚无 wire 入口。
- **未接入**：经 Stream 的块传输（P1-04/P1-07）与跨进程会话/上传恢复（P4）。