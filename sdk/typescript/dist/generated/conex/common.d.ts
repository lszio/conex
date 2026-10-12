import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
export declare const protobufPackage = "conex";
/** 连接平面。P0 只允许 broker；relay 可识别但拒绝使用。 */
export declare enum Plane {
    PLANE_UNSPECIFIED = 0,
    PLANE_BROKER = 1,
    PLANE_RELAY = 2,
    UNRECOGNIZED = -1
}
export declare function planeFromJSON(object: any): Plane;
export declare function planeToJSON(object: Plane): string;
/**
 * JSON-RPC 数值在 conex/1 冻结（设计 §5.2）。数值一旦发布不得改号或复用；
 * -32019..-32099 预留给后续业务码。
 */
export declare enum ErrorCode {
    ERROR_CODE_UNSPECIFIED = 0,
    ERROR_CODE_PARSE_ERROR = -32700,
    ERROR_CODE_BAD_REQUEST = -32602,
    ERROR_CODE_UNKNOWN_METHOD = -32601,
    ERROR_CODE_INTERNAL = -32603,
    ERROR_CODE_UNAUTHORIZED = -32001,
    ERROR_CODE_FORBIDDEN = -32002,
    ERROR_CODE_UNKNOWN_PROVIDER = -32003,
    ERROR_CODE_UNSUPPORTED_CAPABILITY = -32004,
    ERROR_CODE_UNAVAILABLE = -32005,
    ERROR_CODE_TIMEOUT = -32006,
    ERROR_CODE_PAYLOAD_TOO_LARGE = -32007,
    ERROR_CODE_QUOTA_EXCEEDED = -32008,
    ERROR_CODE_PLANE_MISMATCH = -32009,
    ERROR_CODE_PEER_UNTRUSTED = -32010,
    ERROR_CODE_CANCELLED = -32011,
    ERROR_CODE_OUTCOME_UNKNOWN = -32012,
    ERROR_CODE_STALE_REVISION = -32013,
    ERROR_CODE_CONFLICT = -32014,
    ERROR_CODE_SLOW_CONSUMER = -32015,
    ERROR_CODE_BAD_BLOB = -32016,
    ERROR_CODE_SESSION_LOST = -32017,
    ERROR_CODE_RESUME_UNAVAILABLE = -32018,
    UNRECOGNIZED = -1
}
export declare function errorCodeFromJSON(object: any): ErrorCode;
export declare function errorCodeToJSON(object: ErrorCode): string;
/** execution/retry 只接受设计列举值，字符串便于前向兼容。 */
export interface Error {
    code?: ErrorCode | undefined;
    message?: string | undefined;
    diagnosticId?: string | undefined;
    execution?: string | undefined;
    retry?: string | undefined;
    details?: any | undefined;
}
export interface Limits {
    maxFrameBytes?: number | undefined;
    maxInflight?: number | undefined;
    maxQueuedBytes?: number | undefined;
    timeoutMs?: number | undefined;
}
export declare const Error: MessageFns<Error>;
export declare const Limits: MessageFns<Limits>;
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
