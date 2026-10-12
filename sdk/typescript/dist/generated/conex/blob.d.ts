import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { BlobAccess, ContentAddress } from "./chunking";
export declare const protobufPackage = "conex";
/** 持久性等级（设计 §5.5）；后端必须声明支持等级，不支持就拒绝，不能降级。 */
export declare enum PersistenceLevel {
    PERSISTENCE_LEVEL_UNSPECIFIED = 0,
    PERSISTENCE_LEVEL_LOCAL = 1,
    UNRECOGNIZED = -1
}
export declare function persistenceLevelFromJSON(object: any): PersistenceLevel;
export declare function persistenceLevelToJSON(object: PersistenceLevel): string;
/** 上传创建（不承载数据）。 */
export interface BlobPutRequest {
    /**
     * 绑定 principal/resourceId/format_version/上限；服务端回填 content_cid
     * 校验用的格式版本。
     */
    formatVersion?: number | undefined;
    /** 客户端声明的总字节；服务端在 commit 时按 contentCid 重新计算。 */
    declaredSizeBytes?: string | undefined;
    declaredChunkSize?: string | undefined;
    /** 客户端期望的内容 CID（manifest 根或 raw CID）；用于 commit 期一致性检查。 */
    expectedRoot?: ContentAddress | undefined;
    /** 资源定位；与 blob/get、blob/pin 的 access 字段一致。 */
    access?: BlobAccess | undefined;
    /** staging 租约请求；默认 1 小时，权限内可续期。 */
    requestedLeaseMs?: string | undefined;
}
export interface BlobPutResponse {
    uploadId?: string | undefined;
    /** 服务端按 formatVersion 选择实际 chunkSize，强制一致。 */
    chunkSize?: number | undefined;
    leaseMs?: string | undefined;
    /** 上传初始窗口字节；超过则服务端回 bad_blob 并关闭上传。 */
    maxBlobBytes?: string | undefined;
    /** 服务端要求或允许 inline 化的最大块；大于此值只能走 chunk 通道。 */
    inlineThresholdBytes?: number | undefined;
    /** 已持有块（如果资源已存在，列出 cid）；客户端可跳过这些块的提交。 */
    alreadyHaveChunkCids?: string[] | undefined;
}
/**
 * blob/chunk：单块提交。顺序由 chunk_index 保证；服务端在收到所有块之前不
 * 持久化 manifest；坏块返回 bad_blob 并结束当前上传。
 */
export interface BlobChunkRequest {
    uploadId?: string | undefined;
    chunkIndex?: number | undefined;
    chunkCid?: string | undefined;
    /** 实际块字节；服务端再算一次 SHA-256 并与 chunk_cid 校验。 */
    chunkBytes?: Uint8Array | undefined;
}
export interface BlobChunkResponse {
    chunkIndex?: number | undefined;
    receivedBytes?: string | undefined;
    remainingBytes?: string | undefined;
}
/**
 * blob/commit：原子校验（根 CID、所有可达块、长度、授权）后建立持久引用。
 * GC 不能删除 commit 正在引用的块；committed 仅在持久性达成后返回。
 */
export interface BlobCommitRequest {
    uploadId?: string | undefined;
    /** 客户端最终声明的根 CID（raw 或 manifest 根）。 */
    declaredRoot?: ContentAddress | undefined;
    /** 可选 blob/pin 期：pin 阻止 GC，直到到期或显式 unpin。 */
    pinUntilMs?: string | undefined;
    /** 请求的持久性等级；后端不能降级后返回成功。 */
    persistence?: string | undefined;
}
export interface BlobCommitResponse {
    resourceRootCid?: string | undefined;
    committedBytes?: string | undefined;
    persistence?: string | undefined;
    /** 提交的接收凭证 ID；不是密码学承诺。 */
    receiptId?: string | undefined;
}
/** blob/pin：返回绑定 principal/space/root 的 pinId；只能释放有权管理的 pin。 */
export interface BlobPinRequest {
    root?: ContentAddress | undefined;
    pinUntilMs?: string | undefined;
    spaceId?: string | undefined;
}
export interface BlobPinResponse {
    pinId?: string | undefined;
    expiresAtMs?: string | undefined;
}
export interface BlobUnpinRequest {
    pinId?: string | undefined;
}
export interface BlobUnpinResponse {
}
/** blob/have：询问某 CID 是否被持久引用；用于流式续传。 */
export interface BlobHaveRequest {
    chunkCids?: string[] | undefined;
}
export interface BlobHaveResponse {
    /** 与请求同序；true 表示已持有，false 表示需要 (重新)上传。 */
    present?: boolean[] | undefined;
}
/**
 * blob/get 目标二选一（M0 冻结）：互斥结构避免同一个 offset 同时被解释成
 * 块内偏移与文件偏移。
 * - committed：读取已提交内容中被授权根覆盖的块，整块返回。
 * - remote：按端点、资源、revision、字节范围读取远端内容（M3 接通运行路径）。
 */
export interface CommittedTarget {
    /** 已提交块 CID；必须属于调用方获授权的根。 */
    chunkCid?: string | undefined;
}
export interface RemoteTarget {
    endpointId?: string | undefined;
    resourceId?: string | undefined;
    /** 提供则与当前版本不符时返回 stale_revision；不拼接两个版本。 */
    revision?: string | undefined;
    /** 字节偏移（十进制字符串 u64）。 */
    offset?: string | undefined;
    /** 字节长度（十进制字符串 u64，>= 1）；offset+length 溢出拒绝。 */
    length?: string | undefined;
}
export interface BlobGetRequest {
    committed?: CommittedTarget | undefined;
    remote?: RemoteTarget | undefined;
}
export interface BlobGetResponse {
    chunkBytes?: Uint8Array | undefined;
    /** committed 路径返回该块 CID；remote 路径缺省（任意 range 不冒充完整块 CID）。 */
    chunkCid?: string | undefined;
    /** remote 路径返回实际服务的 revision。 */
    revision?: string | undefined;
    /** remote 路径返回是否到达文件尾；读取结束必须回传实际长度与 EOF。 */
    eof?: boolean | undefined;
}
/** blob/cancel：主动结束 staging；不会留下 committed 资源。 */
export interface BlobCancelRequest {
    uploadId?: string | undefined;
    reason?: string | undefined;
}
export interface BlobCancelResponse {
}
export declare const BlobPutRequest: MessageFns<BlobPutRequest>;
export declare const BlobPutResponse: MessageFns<BlobPutResponse>;
export declare const BlobChunkRequest: MessageFns<BlobChunkRequest>;
export declare const BlobChunkResponse: MessageFns<BlobChunkResponse>;
export declare const BlobCommitRequest: MessageFns<BlobCommitRequest>;
export declare const BlobCommitResponse: MessageFns<BlobCommitResponse>;
export declare const BlobPinRequest: MessageFns<BlobPinRequest>;
export declare const BlobPinResponse: MessageFns<BlobPinResponse>;
export declare const BlobUnpinRequest: MessageFns<BlobUnpinRequest>;
export declare const BlobUnpinResponse: MessageFns<BlobUnpinResponse>;
export declare const BlobHaveRequest: MessageFns<BlobHaveRequest>;
export declare const BlobHaveResponse: MessageFns<BlobHaveResponse>;
export declare const CommittedTarget: MessageFns<CommittedTarget>;
export declare const RemoteTarget: MessageFns<RemoteTarget>;
export declare const BlobGetRequest: MessageFns<BlobGetRequest>;
export declare const BlobGetResponse: MessageFns<BlobGetResponse>;
export declare const BlobCancelRequest: MessageFns<BlobCancelRequest>;
export declare const BlobCancelResponse: MessageFns<BlobCancelResponse>;
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
