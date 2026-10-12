import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
export declare const protobufPackage = "conex";
/** 单方向流的元数据；在 stream/open 响应中返回，session/resume 也回填一份。 */
export interface StreamSpec {
    streamId?: string | undefined;
    /** "data" 或 "control"；控制流（ack/flow/cancel/ping）独立有界。 */
    kind?: string | undefined;
    /** 接收窗口字节（设计 §6.2）；窗口在流建立时固定，不能用后续 flow 扩展。 */
    windowBytes?: string | undefined;
    /** 初始 maxFrameBytes。 */
    maxFrameBytes?: number | undefined;
}
/**
 * 数据帧（profile 决定字节形态，本消息是其语义字段）。注意：零字节 message
 * 禁止；ACK 必须是连续 seq 的最大已确认值；累计字节是 sender 实际发送字节。
 */
export interface StreamFrame {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    epoch?: string | undefined;
    streamId?: string | undefined;
    seq?: string | undefined;
    /** 单帧字节；由 Profile 编码（JSON/protobuf）后投到 wire。 */
    message?: Uint8Array | undefined;
}
/** stream/ack：累计已连续接收的最大 seq；不是窗口外字节通知。 */
export interface StreamAckRequest {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    epoch?: string | undefined;
    streamId?: string | undefined;
    lastReceivedSeq?: string | undefined;
}
export interface StreamAckResponse {
    streamId?: string | undefined;
    /**
     * 服务端按 (lastReceivedSeq+1) 起重新广告的窗口（<=协商窗口）；
     * 接收端不能基于此把窗口扩展到协商上限之外。
     */
    availableWindowBytes?: string | undefined;
}
/**
 * stream/flow：发送端报告累计已消费字节（receiver 视角），请求新窗口。
 * 重复或旧的 flow（consumedBytes <= 之前接受的最大值）忽略，不加信用。
 */
export interface StreamFlowRequest {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    epoch?: string | undefined;
    streamId?: string | undefined;
    consumedBytes?: string | undefined;
    /** 发送端请求新窗口字节数；不得超过协商上限且不得用于扩展。 */
    requestedWindowBytes?: string | undefined;
}
export interface StreamFlowResponse {
    newWindowBytes?: string | undefined;
    /** 当 receiver 不能接受任何数据信用（控制仍然可发）。 */
    zeroWindow?: boolean | undefined;
}
/**
 * stream/reset：清空一段范围；接收端必须把未确认字节明确标为需重放，不能
 * 静默当作完整历史。
 */
export interface StreamResetRequest {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    epoch?: string | undefined;
    streamId?: string | undefined;
    afterSeq?: string | undefined;
    reason?: string | undefined;
    resumeHandle?: string | undefined;
}
export interface StreamResetResponse {
    streamId?: string | undefined;
    acceptedAfterSeq?: string | undefined;
}
/** 慢消费者信号：control queue 满或 credit 耗尽 30 秒后由接收方发出。 */
export interface SlowConsumerSignal {
    streamId?: string | undefined;
    reason?: string | undefined;
    sinceMs?: string | undefined;
}
export declare const StreamSpec: MessageFns<StreamSpec>;
export declare const StreamFrame: MessageFns<StreamFrame>;
export declare const StreamAckRequest: MessageFns<StreamAckRequest>;
export declare const StreamAckResponse: MessageFns<StreamAckResponse>;
export declare const StreamFlowRequest: MessageFns<StreamFlowRequest>;
export declare const StreamFlowResponse: MessageFns<StreamFlowResponse>;
export declare const StreamResetRequest: MessageFns<StreamResetRequest>;
export declare const StreamResetResponse: MessageFns<StreamResetResponse>;
export declare const SlowConsumerSignal: MessageFns<SlowConsumerSignal>;
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
