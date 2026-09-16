# P1 分块与 manifest 契约（conex/1 broker 分块）

状态：**P1-01 冻结，§3/§4 于 2026-09-16 由 P1-01b 修正并重新冻结**（见 [P1 计划 §2.1](../plans/2026-09-15-conex-p1.md)）。
类型源：`schema/conex/v1/chunking.proto`（`ChunkEntry / ManifestEntries / ChunkManifest / ContentAddress / BlobRef / BlobAccess`）。
权威实现：`crates/conex-proto/src/cid.rs`（Rust）与 `sdk/typescript/src/content.ts`（TS）。二者是同一规则的两侧实现，**不得**在别处重建 manifest 字节。
向量：`conformance/vectors/p1/chunking.json`，由 `conformance/tools/gen_manifest_goldens.py`（`protoc --encode` + Python 标准库，与 prost/ts-proto 无共享代码）生成；Rust（`crates/conex-proto/tests/chunking.rs`）与 TS（`sdk/typescript/tests/chunking.test.ts`）两侧必须逐字节复现。

## 1. 统一内容寻址函数（设计 §5.3，P0 固定，P1 沿用）

```text
contentCid(bytes, chunkSize = 262144 /* 256 KiB */) =
    if len(bytes) <= chunkSize
        then cid_for_raw(bytes)                    # CIDv1 raw + SHA-256 文本
        else manifestTree(chunkSize, len(bytes), leafCids).rootCid
```

- `len(bytes) <= chunkSize` 的输入返回 `CIDv1/raw/SHA-256` 文本；`source/read` 的 `cid` 必须调用同一函数（`crates/conex-provider-fs`、`crates/conex-provider-http-catalog` 已改为 `conex_proto::cid::content_cid`）。
- 多块内容返回 `ChunkManifest` 根 CID。同一份字节在任一实现、任一语言下必须产生同一根；同一份内容**不得**同时存在 `raw_cid` 与 `manifest_cid` 两种可比较地址（设计 §5.3）。
- 根 CID **绑定** `formatVersion`、`chunkSize`、`contentLength` 与全部叶子 CID 序列：改变其中任何一项都必须得到不同的根。

## 2. 块大小与边界

- 默认 `chunkSize = 262144`（256 KiB）；末块可更短但不得为零字节（空内容走 `cid_for_raw(b"")`）。
- `len(bytes) == chunkSize` 属于单块内容，返回 raw CID；只有 `len(bytes) > chunkSize` 才产生 manifest，因此 manifest 的叶子数 ≥ 2。
- 第 `i` 块的逻辑长度固定推导为 `min(chunkSize, contentLength - i * chunkSize)`；`chunk_length` 字段必须等于该值。
- 单层 `ManifestEntries` 最多 **1024** 项（`MANIFEST_FANOUT`）；叶子数超过 1024 时按 §3.3 分层，直至根层只有一项。

## 3. 规范化 manifest 字节（canonical manifest）

### 3.1 唯一形式

多块内容的规范化字节 = `ChunkManifest` 的 **protobuf wire 编码**，由类型源生成的两侧实现产出：

- Rust：`prost::Message::encode_to_vec`（`ChunkManifest`）。
- TS：生成的 `ChunkManifest.encode(...).finish()`（ts-proto，`@bufbuild/protobuf/wire`）。
- **不**经过 ProtoJSON，**不**使用 canonical JSON；JSON 路线在 P1 内不存在，若将来需要必须提升 `format_version` 并重新冻结向量。

### 3.2 编码规则（必须有唯一的字节结果）

1. **字段**：`format_version`（=1）、`chunk_size`（uint32）、`content_length`（base-10 字符串）、`root`（必填且非空）。`root` 为 `ManifestEntries`，同一层内要么只有 `leaves`，要么只有 `child_manifest_cids`。
2. **顺序**：`leaves` 按 `chunkIndex` 升序；`child_manifest_cids` 按其覆盖的叶子区间升序。二者都是 `repeated`，按字段编号升序写出，不做排序以外的重排。
3. **数值**：`chunk_size` 为 uint32；`content_length` 与 `chunk_length` 为十进制字符串（避免 JS Number 丢精度），不得有空值或前导零。
4. **默认值省略**：只有 proto3 的默认值字段被省略；本消息的全部字段在 v1 中都必须显式存在（`format_version=1`、`chunk_size>0`、`content_length` 非空、`root` 非空），因此不存在省略歧义。
5. **禁止**：map、浮点、负数、未知字段、`packed` 之外的 repeated 编码；出现即视为格式错误。字段编号一旦发布不得改动，新增字段必须提升 `format_version`（见 `chunking.proto` 注释）。

> 设计 §5.3 指出 protobuf 的「deterministic serialization」不保证跨版本规范化。本契约因此把编码规则限定在上表的窄结构（字符串 + uint32 + repeated message），并由 §4 的黄金字节向量在 Rust、TS 两侧逐字节验证；任何一侧序列化行为变化都会使门禁失败，而不是静默改变根 CID。

### 3.3 分层规则

- 叶子层：每 ≤1024 个叶子构成一个 manifest（叶层节点），`leaves` 按 `chunkIndex` 升序，`chunk_length` 按 §2 推导。
- 上层：把下一层的 manifest CID 每 ≤1024 个分组构成一个父 manifest，只填 `child_manifest_cids`。
- 直到某层只剩 1 个 manifest，它就是根。每一层的 `chunk_size` 与 `content_length` 都写同一份内容的总值。
- 所有 manifest 节点都用 `cid_for_raw(nodeBytes)` 寻址（CIDv1 raw/SHA-256），因此 manifest 对象与数据块走同一条存储与检索路径。

## 4. 已知答案向量边界

`conformance/vectors/p1/chunking.json` 由 `conformance/tools/gen_manifest_goldens.py` 生成（`python3 conformance/tools/gen_manifest_goldens.py [--check]`）。每个用例给出 `payload` 规格、`chunkSize`、`sizeBytes`、`expectedRootKind`、`rootCid`；manifest 用例另有 `rootManifestBytesLength`，根 manifest ≤ 4 KiB 时给出 `rootManifestBytesHex`，分层用例给出 `leafManifestCids`，叶子 ≤4 的用例给出 `leafCids`。

| 用例 | 期望 |
|---|---|
| `empty` | `contentCid(b"") == cid_for_raw(b"")` |
| `exactly_one_chunk_262kib` | 长度恰等于 `chunkSize` → raw CID（单块边界） |
| `one_byte_over_one_chunk` | 多 1 字节即走 manifest；末块长度为 1 |
| `two_full_chunks` / `three_chunks_short_last` | 根 CID 与黄金字节逐字节一致；`chunk_length` 分别等于 `chunkSize` 与末块实长 |
| `single_level_full_fanout` | 恰好 1024 个叶子 → 单层 manifest（根 manifest 字节长度固定） |
| `layered_above_fanout` | 1025 个叶子 → 两层：叶层 2 个 manifest + 根 manifest（根只含 `child_manifest_cids`） |
| `rejects` | 非 raw codec、非 sha2-256、不可解析 CID 拒绝；`content_length <= chunk_size` 不允许 manifest；叶子数与 `ceil(content_length/chunk_size)` 不符拒绝；`chunk_size == 0` 拒绝 |

## 5. BlobRef 与访问授权

- `BlobRef.cid` 是字节完整性地址；`access` 是定位信息（`providerId / plane / spaceId? / resourceId`），**不是**授权凭据。
- 接收侧用资源元数据或授权句柄证明资源与 CID 的关联（设计 §5.3）；`blob/get` / `blob/have` / `blob/pin` 都经过资源级策略。
- 跨空间相同 CID **不**继承权限。

## 6. 与 source/read CID 的一致性

- `source/read` 返回的 `cid` 由 `content_cid(bytes, CHUNK_SIZE)` 计算，与 `blob/*` 的根使用同一函数。
- 当前两个 provider 的单文档上限（`MAX_DOC_BYTES = 256 KiB`）等于一个 chunk，因此可读文档的地址恒为 raw CID；`crates/conex-provider-fs/tests/read.rs` 用上限边界用例锁定这一点，上限若超过一个 chunk，该用例会失败并强制重新评审地址语义。
- `source/read` 不得为同一份内容同时声明 `raw_cid` 与 `manifest_cid` 两种可比较地址；必须是 §1 的 `ContentAddress` oneof。
