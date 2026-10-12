import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
export declare const protobufPackage = "conex";
/** 单个分块条目；多块内容用 ManifestEntries 表示。 */
export interface ChunkEntry {
    /** 子 CID，CIDv1 raw + SHA-256，文本默认小写 base32；见 cid.rs::parse_cid。 */
    chunkCid?: string | undefined;
    /** 该块逻辑长度（十进制字符串）；所有叶子合计 = contentLength。 */
    chunkLength?: string | undefined;
}
/** 叶子聚合（指向下一级 manifest 或直接指向 ChunkEntry 的混合列表）。 */
export interface ManifestEntries {
    /** 单层最多 1024 个条目；超过时分层（设计 §5.4）。 */
    leaves?: ChunkEntry[] | undefined;
    /** 中间层 manifest 的子 CID；同级 manifest 之间按条目索引升序拼接。 */
    childManifestCids?: string[] | undefined;
}
/**
 * 分块 manifest。formatVersion 一旦发布不得改号；新增字段走 reserved
 * + 新消息，避免静默破坏 root CID。
 */
export interface ChunkManifest {
    /** 内容格式版本；P1 固定为 1。 */
    formatVersion?: number | undefined;
    /** 叶子层默认 256 KiB；末块更短。除非 formatVersion 提升否则不得改值。 */
    chunkSize?: number | undefined;
    /** 内容总字节数（十进制字符串）。 */
    contentLength?: string | undefined;
    /**
     * 根条目；可以是 leaves 或 child_manifest_cids 中的若干项，但同一份内容
     * 必须产生同一 root CID：实现必须规范化顺序（按条目索引升序拼接后再
     * 算 manifest CID），见 p1-chunking 契约 §3。
     */
    root?: ManifestEntries | undefined;
}
/**
 * contentCid() 输入与输出：分块内容长度 ≤ chunk_size 时返回 raw CID，否则
 * 返回 ChunkManifest 序列化字节的 CID（manifest 序列化使用 deterministic
 * proto wire 或本文件附录 A 的 canonical JSON；二者必须同一根 CID）。
 */
export interface ContentAddress {
    /** 小文件：CIDv1 raw + SHA-256 文本。 */
    rawCid?: string | undefined;
    /** 多块：manifest 根 CID；调用方通过 blob/get 拉取叶子。 */
    manifestCid?: string | undefined;
}
/** BlobRef：带授权定位的内容引用（设计 §5.3）。 */
export interface BlobRef {
    /**
     * 内容 CID；仅当字节已按 CID 格式计算。可变远端文件未计算 CID 时缺省，
     * 禁止用路径或 revision 假充 CID。
     */
    cid?: string | undefined;
    /** 字节数（十进制字符串）。 */
    sizeBytes?: string | undefined;
    /** MIME；缺省为 application/octet-stream。 */
    mime?: string | undefined;
    /** 资源定位信息（不是授权凭据）。 */
    access?: BlobAccess | undefined;
    /** 远端可变文件的读取定位 revision；不透明令牌，仅等值比较。 */
    revision?: string | undefined;
}
/**
 * 资源定位 = endpointId + resourceId；租户来自认证上下文，不信任请求自报。
 * provider_id 不得代替唯一端点定位；endpoint_id 为空表示 Host 本地已提交内容。
 */
export interface BlobAccess {
    endpointId?: string | undefined;
    plane?: string | undefined;
    spaceId?: string | undefined;
    resourceId?: string | undefined;
}
export declare const ChunkEntry: MessageFns<ChunkEntry>;
export declare const ManifestEntries: MessageFns<ManifestEntries>;
export declare const ChunkManifest: MessageFns<ChunkManifest>;
export declare const ContentAddress: MessageFns<ContentAddress>;
export declare const BlobRef: MessageFns<BlobRef>;
export declare const BlobAccess: MessageFns<BlobAccess>;
type Builtin = Date | Function | Uint8Array | string | number | boolean | undefined;
export type DeepPartial<T> = T extends Builtin ? T : T extends globalThis.Array<infer U> ? globalThis.Array<DeepPartial<U>> : T extends ReadonlyArray<infer U> ? ReadonlyArray<DeepPartial<U>> : T extends {} ? {
    [K in keyof T]?: DeepPartial<T[K]>;
} : Partial<T>;
type KeysOfUnion<T> = T extends T ? keyof T : never;
export type Exact<P, I extends P> = P extends Builtin ? P : P & {
    [K in keyof P]: Exact<P[K], I[K]>;
} & {
    [K in Exclude<keyof I, KeysOfUnion<P>>]: never;
};
export interface MessageFns<T> {
    encode(message: T, writer?: BinaryWriter): BinaryWriter;
    decode(input: BinaryReader | Uint8Array, length?: number): T;
    fromJSON(object: any): T;
    toJSON(message: T): unknown;
    create<I extends Exact<DeepPartial<T>, I>>(base?: I): T;
    fromPartial<I extends Exact<DeepPartial<T>, I>>(object: I): T;
}
export {};
