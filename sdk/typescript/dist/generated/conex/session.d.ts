import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { Plane } from "./common";
export declare const protobufPackage = "conex";
/** Session 绑定基线（设计 §7.1）。 */
export interface SessionBinding {
    principalId?: string | undefined;
    tenantId?: string | undefined;
    providerEndpointId?: string | undefined;
    plane?: Plane | undefined;
    workspacePeerId?: string | undefined;
    humanPeerId?: string | undefined;
}
/** session/open 输入：客户端只声明目标，接收方最终决定 binding。 */
export interface SessionOpenRequest {
    requested?: SessionBinding | undefined;
    /**
     * 客户端可接受的恢复等级；"none" 表示不可恢复，"in_process" 表示窗口内
     * 网络重连，"persistent" 须显式声明才能接受。
     */
    recovery?: string | undefined;
    /** 客户端声明的 capability snapshot；最终由握手 hello/ready 共同决定。 */
    provides?: string[] | undefined;
    requires?: string[] | undefined;
}
export interface SessionOpenResponse {
    sessionId?: string | undefined;
    binding?: SessionBinding | undefined;
    /** 实际授予的恢复等级与 epoch 起始值。 */
    recovery?: string | undefined;
    attachmentEpoch?: string | undefined;
    /** 参与绑定的租约与失联后的最迟恢复期限（毫秒）。 */
    leaseMs?: string | undefined;
    /** 能力快照（设计 §4.3）；rejected 给出被丢弃的能力与原因。 */
    provides?: string[] | undefined;
    rejectedCapabilities?: string[] | undefined;
}
/** session/resume 必须在新的已认证 Link 上调用；expected_epoch fencing 旧连接。 */
export interface SessionResumeRequest {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    expectedEpoch?: string | undefined;
    /** 每流的恢复游标；按 seq 已确认的最大值回填。 */
    streams?: StreamCursor[] | undefined;
}
export interface StreamCursor {
    streamId?: string | undefined;
    /** 接收端已连续确认的最大 seq；发送端从 seq+1 起重放未确认部分。 */
    lastReceivedSeq?: string | undefined;
    /** 接收端累计已消费字节；用于恢复期信用锚定（设计 §6.2）。 */
    consumedBytes?: string | undefined;
}
export interface SessionResumeResponse {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    /** 新 epoch（总是 expected_epoch+1）；旧 Link 在该 attachment 上被 fencing。 */
    newEpoch?: string | undefined;
    /** 若窗口不足返回 stream_reset，附可用业务恢复方式（如 blob/uploadId）。 */
    streamsReset?: StreamReset[] | undefined;
    /** 接收端按上次 advertised 信用重置；新窗口不能小于尚未确认的已发送字节。 */
    restoredWindowBytes?: string | undefined;
}
export interface StreamReset {
    streamId?: string | undefined;
    /** 提示接收方为什么不接受；不允许悄悄丢弃未确认字节。 */
    reason?: string | undefined;
    /** 对应业务的续传入口（如 blob uploadId）。 */
    resumeHandle?: string | undefined;
}
/**
 * session/renew：在原 attachment 上延长租约（不能延长其他 attachment 或无
 * 关 Session）。
 */
export interface SessionRenewRequest {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    expectedEpoch?: string | undefined;
    extendMs?: string | undefined;
}
export interface SessionRenewResponse {
    newEpoch?: string | undefined;
    leaseMs?: string | undefined;
}
/** session/close：显式终止；终态不可复用旧 ID 复活。 */
export interface SessionCloseRequest {
    sessionId?: string | undefined;
    attachmentId?: string | undefined;
    expectedEpoch?: string | undefined;
    reason?: string | undefined;
}
export interface SessionCloseResponse {
    sessionId?: string | undefined;
    finalEpoch?: string | undefined;
}
/** 一次性 /tickets（设计 §8.5）：30 秒、原子消费、绑定主体/会话/角色/上限。 */
export interface WebTicket {
    /** ticket 值本身；服务端原子消费后立即失效。 */
    ticket?: string | undefined;
    /** 绑定主体与租户。 */
    principalId?: string | undefined;
    tenantId?: string | undefined;
    /** WS 握手校验 Origin 时使用；其他 Origin 拒绝。 */
    origin?: string | undefined;
    /** 目标 host 与 peer 角色（agent/ui）。 */
    targetHost?: string | undefined;
    peerRole?: string | undefined;
    /** 能力上限与（可选）目标 sessionId。 */
    capabilityCaps?: string[] | undefined;
    sessionId?: string | undefined;
    /** 颁发 Unix 毫秒与到期 Unix 毫秒；server 拒绝过期 ticket。 */
    issuedAtMs?: string | undefined;
    expiresAtMs?: string | undefined;
}
export declare const SessionBinding: MessageFns<SessionBinding>;
export declare const SessionOpenRequest: MessageFns<SessionOpenRequest>;
export declare const SessionOpenResponse: MessageFns<SessionOpenResponse>;
export declare const SessionResumeRequest: MessageFns<SessionResumeRequest>;
export declare const StreamCursor: MessageFns<StreamCursor>;
export declare const SessionResumeResponse: MessageFns<SessionResumeResponse>;
export declare const StreamReset: MessageFns<StreamReset>;
export declare const SessionRenewRequest: MessageFns<SessionRenewRequest>;
export declare const SessionRenewResponse: MessageFns<SessionRenewResponse>;
export declare const SessionCloseRequest: MessageFns<SessionCloseRequest>;
export declare const SessionCloseResponse: MessageFns<SessionCloseResponse>;
export declare const WebTicket: MessageFns<WebTicket>;
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
