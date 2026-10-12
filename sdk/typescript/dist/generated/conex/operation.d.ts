import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { Error } from "./common";
export declare const protobufPackage = "conex";
/**
 * 四个 execution 类别。
 * "read_only" → retry=safe
 * "idempotent" → retry=safe (重查策略)
 * "deduplicated" → retry=with_operation_id
 * "non_replayable" 已发送 → retry=never & execution=unknown
 */
export declare enum ExecutionClass {
    EXECUTION_CLASS_UNSPECIFIED = 0,
    EXECUTION_CLASS_READ_ONLY = 1,
    EXECUTION_CLASS_IDEMPOTENT = 2,
    EXECUTION_CLASS_DEDUPLICATED = 3,
    EXECUTION_CLASS_NON_REPLAYABLE = 4,
    UNRECOGNIZED = -1
}
export declare function executionClassFromJSON(object: any): ExecutionClass;
export declare function executionClassToJSON(object: ExecutionClass): string;
/** operation 状态机：accepted → running → succeeded | failed | cancelled | unknown */
export declare enum OperationState {
    OPERATION_STATE_UNSPECIFIED = 0,
    OPERATION_STATE_ACCEPTED = 1,
    OPERATION_STATE_RUNNING = 2,
    OPERATION_STATE_SUCCEEDED = 3,
    OPERATION_STATE_FAILED = 4,
    OPERATION_STATE_CANCELLED = 5,
    OPERATION_STATE_UNKNOWN = 6,
    UNRECOGNIZED = -1
}
export declare function operationStateFromJSON(object: any): OperationState;
export declare function operationStateToJSON(object: OperationState): string;
/** 去重键（与 requestId 分开；与上游幂等键一致时才有意义）。 */
export interface DedupKey {
    tenantId?: string | undefined;
    principalId?: string | undefined;
    providerEndpointId?: string | undefined;
    spaceId?: string | undefined;
    resourceId?: string | undefined;
    method?: string | undefined;
    /** operationId；ULID；client 不能再造 operationId 来绕开 unknown。 */
    operationId?: string | undefined;
}
/** operation/get 与 cancel 的输入。 */
export interface OperationGetRequest {
    key?: DedupKey | undefined;
}
export interface OperationGetResponse {
    key?: DedupKey | undefined;
    state?: OperationState | undefined;
    /** 保留期限 Unix 毫秒；过期之后状态不再可信。 */
    expiresAtMs?: string | undefined;
    /** 已确定的成功结果（deduplicated 类别；上游幂等支持时才能取回）。 */
    success?: {
        [key: string]: any;
    } | undefined;
    /** 已确定的错误（failed/cancelled）；不可与 success 同时存在。 */
    failure?: Error | undefined;
    /** execution 表示当前已知的副作用状态；unknown 时不能自动重试。 */
    execution?: string | undefined;
    /** 启动时间与最近一次状态变化时间。 */
    acceptedAtMs?: string | undefined;
    settledAtMs?: string | undefined;
}
export interface OperationCancelRequest {
    key?: DedupKey | undefined;
    /** 取消仅影响未开始或已停止的执行器；与已落定的结果并发时，结果优先保留。 */
    reason?: string | undefined;
}
export interface OperationCancelResponse {
    state?: OperationState | undefined;
    /** 若执行器无法确认是否产生副作用，返回 unknown。 */
    outcomeUnknown?: boolean | undefined;
}
/**
 * 持久记录条目（写入持久后端；设计 §7.3）。schemaVersion 用于崩溃恢复时
 * 校验格式演进。
 */
export interface OperationRecord {
    schemaVersion?: number | undefined;
    key?: DedupKey | undefined;
    state?: OperationState | undefined;
    executionClass?: ExecutionClass | undefined;
    /** 参数摘要（与原文摘要比对，防止同键异参被错误合并）；默认 SHA-256 文本 CID。 */
    paramDigest?: string | undefined;
    /** 接受时间 / 最后一次状态变化 / 过期时间。 */
    acceptedAtMs?: string | undefined;
    settledAtMs?: string | undefined;
    expiresAtMs?: string | undefined;
}
export declare const DedupKey: MessageFns<DedupKey>;
export declare const OperationGetRequest: MessageFns<OperationGetRequest>;
export declare const OperationGetResponse: MessageFns<OperationGetResponse>;
export declare const OperationCancelRequest: MessageFns<OperationCancelRequest>;
export declare const OperationCancelResponse: MessageFns<OperationCancelResponse>;
export declare const OperationRecord: MessageFns<OperationRecord>;
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
