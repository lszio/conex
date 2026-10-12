import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { Error, Plane } from "./common";
export declare const protobufPackage = "conex";
/**
 * 逻辑消息变体（设计 §5.2）。信封用通用 Value 承载 params/result；
 * 具体方法与 typed input/output 由方法注册绑定并在 prepare 处严格解码。
 */
export interface Message {
    request?: Request | undefined;
    success?: Success | undefined;
    failure?: Failure | undefined;
    notification?: Notification | undefined;
    /**
     * M3：有界数据帧响应（protobuf 二进制 Profile 专用），避免把原始字节
     * 转成 base64 字符串。JSON Profile 下载 JSON 时 chunk 退化为 base64。
     */
    dataChunk?: DataChunk | undefined;
}
/** blob/get(remote) 的字节响应：一帧 ≤ max_frame_bytes 的原始文件切片。 */
export interface DataChunk {
    requestId?: string | undefined;
    chunk?: Uint8Array | undefined;
    /**
     * 实际服务的文件 revision；与请求携带的 revision 不符时服务端直接返回
     * stale_revision 错误，不会出现混合版本数据。
     */
    revision?: string | undefined;
    eof?: boolean | undefined;
}
export interface Request {
    requestId?: string | undefined;
    method?: string | undefined;
    params?: CallParams | undefined;
}
export interface CallParams {
    context?: RequestContext | undefined;
    timeoutBudgetMs?: number | undefined;
    input?: any | undefined;
}
export interface RequestContext {
    providerEndpointId?: string | undefined;
    plane?: Plane | undefined;
    bindingId?: string | undefined;
    /**
     * Authenticated caller identity forwarded by the host (plan M2); the
     * agent uses it for snapshot binding and local claim checks. Empty on
     * frames that carry no user identity.
     */
    principalId?: string | undefined;
    tenantId?: string | undefined;
}
export interface Success {
    requestId?: string | undefined;
    result?: any | undefined;
}
export interface Failure {
    requestId?: string | undefined;
    error?: Error | undefined;
}
export interface Notification {
    method?: string | undefined;
    params?: CallParams | undefined;
}
export declare const Message: MessageFns<Message>;
export declare const DataChunk: MessageFns<DataChunk>;
export declare const Request: MessageFns<Request>;
export declare const CallParams: MessageFns<CallParams>;
export declare const RequestContext: MessageFns<RequestContext>;
export declare const Success: MessageFns<Success>;
export declare const Failure: MessageFns<Failure>;
export declare const Notification: MessageFns<Notification>;
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
