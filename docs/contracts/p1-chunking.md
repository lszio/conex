# P1 分块与 manifest 契约（conex/1 broker 分块）

状态：**P1-01 冻结**（2026-09-15）。
类型源：`schema/conex/v1/chunking.proto`（参见 `chunking.proto::ChunkEntry / ManifestEntries / ChunkManifest / ContentAddress / BlobRef / BlobAccess`）。
权威严格解码点：`MethodContract.prepare / validate_output` 与 `crates/conex-proto/src/cid.rs::cid_for_raw / parse_cid`。
向量：`conformance/vectors/p1/chunking.json`，Rust（`crates/conex-proto/tests/chunking.rs`）与 TS（`sdk/typescript/tests/chunking.test.ts`）两侧分类一致。

## 1. 统一内容寻址函数（设计 §5.3，P0 固定，P1 沿用）

```text
contentCid(bytes, chunkSize = 262144 /* 256 KiB */) =
    if len(bytes) <= chunkSize
        then cid_for_raw(bytes)             # CIDv1 raw + SHA-256 文本
        else contentCid(manifestBytesFor(contentCid, bytes, chunkSize))
```

- `len(bytes) <= chunkSize` 的输入返回 `CIDv1/raw/SHA-256` 文本，与 P0 `source/read` 的 `cid` 字段使用同一函数。
- 多块内容返回 `ChunkManifest` 根 CID；同一份字节必须产生同一根 CID，与序列化路径无关（protobuf wire 与 canonical JSON 等价）。

## 2. 块大小与边界

- 默认 `chunkSize = 262144`（256 KiB）；末块可更短但不得为零字节（空内容走 `cid_for_raw(b"")`）。
- 单层 `ManifestEntries` 最多 1024 项；超过时分层（`child_manifest_cids`），再次超过继续分层直至叶子层。
- 分层深度仅受实现栈限制，但同一份内容的根 CID 必须稳定：构造算法见 §3。

## 3. 规范化 manifest 字节（canonical manifest）

manifest 的字节输入 `manifestBytesFor` 必须满足以下三个不变式，否则同一份内容会产生不同根 CID：

1. **顺序**：同级 `leaves` 按 `chunkIndex` 升序拼接；同级 `child_manifest_cids` 按索引升序拼接。
2. **字段顺序**：序列化时固定 `formatVersion → chunkSize → contentLength → root`（proto wire 字段编号天然有序；canonical JSON 走字段名字典序）。
3. **数值编码**：`chunk_size` 为 uint32；`content_length` 与每个 `chunk_length` 为 base-10 字符串，**禁止** protobuf 普通序列化当规范化内容（设计 §5.3）。

具体做法：

- **protobuf 路径**：使用 prost/pbjson 生成的 Rust/TS 类型；调用方在写入前必须按字段顺序构造 `ChunkManifest` 并直接调用 `prost::Message::encode_to_vec` / TS 端 `@bufbuild/protobuf` 同名方法，**不**经过 ProtoJSON。ProtoJSON 输出不视为规范字节。
- **JSON 路径**：调用方先把所有 `*_length` 字段序列化为十进制字符串，整体经 JSON canonicalisation（RFC 8785 子集：字典序字段名、无空白、无转义差异、`number` 仅出现在显式声明的数字字段）后投到 `cid_for_raw`。当前 P1 范围 JSON 路径不返回 `manifest_cid`，只读 `raw_cid`；如未来需要 JSON manifest 等价，**必须**经评审冻结新版本。

> §3 的具体实现选择属于「**已冻结**」：本节明确「protobuf wire 才是 manifest 规范化字节」；JSON canonicalisation 路线属于「**占位待定**」，在 P1-02 之前的协议评审冻结。

## 4. 已知答案向量边界

`conformance/vectors/p1/chunking.json` 覆盖：

| 用例 | 期望 |
|---|---|
| 单块（≤ 256 KiB） | `contentCid(bytes) == cid_for_raw(bytes)` |
| 256 KiB 整块 + 1 字节 | 走 manifest；根 CID 与两个叶子块的拼接顺序无关 |
| 多块（2、3、5 块） | 同一输入多次计算根 CID 必须相同；改变叶子顺序必须产生不同根 |
| 末块恰好 256 KiB | 仍视为「多块」，但 manifest 仅含一项（等价规则不变） |
| 分层（> 1024 叶子） | 根 CID 在 protobuf wire 与同一输入的独立 Rust/TS 实现一致 |
| 空输入 | `contentCid(b"") == cid_for_raw(b"")`（P0 CID 向量已覆盖） |
| 错误输入 | 非 raw/SHA-256 的子 CID 拒绝（bad_blob）；非十进制 `content_length` 拒绝 |

## 5. BlobRef 与访问授权

- `BlobRef.cid` 是字节完整性地址；`access` 是定位信息（`providerId / plane / spaceId? / resourceId`），**不是**授权凭据。
- 接收侧用资源元数据或授权句柄证明资源与 CID 的关联（设计 §5.3）；`blob/get` / `blob/have` / `blob/pin` 都经过资源级策略。
- 跨空间相同 CID **不**继承权限。

## 6. 与 source/read CID 的一致性

- `source/read` 返回的 `cid` 与 `contentCid(bytes)` 在同一 chunkSize 下输出同一字符串。
- `source/read` 不得为同一份内容同时声明 `raw_cid` 与 `manifest_cid` 两种可比较地址；必须是 §1 的 `ContentAddress` oneof。