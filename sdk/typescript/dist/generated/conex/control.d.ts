import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { Limits, Plane } from "./common";
export declare const protobufPackage = "conex";
export interface HelloRequest {
    profileId?: string | undefined;
    plane?: Plane | undefined;
    provides?: string[] | undefined;
    requires?: string[] | undefined;
}
export interface RejectedCapability {
    method?: string | undefined;
    direction?: string | undefined;
    reason?: string | undefined;
}
export interface HelloResponse {
    bindingId?: string | undefined;
    expiresInMs?: number | undefined;
    profileId?: string | undefined;
    plane?: Plane | undefined;
    provides?: string[] | undefined;
    rejectedCapabilities?: RejectedCapability[] | undefined;
    limits?: Limits | undefined;
}
/**
 * Identity and capability summary for a negotiated WS link. These fields are
 * deliberately separate from HTTP binding state.
 */
export interface LinkIdentity {
    linkId?: string | undefined;
    peerId?: string | undefined;
    tenantId?: string | undefined;
}
export interface NegotiatedCapabilities {
    provides?: string[] | undefined;
    requires?: string[] | undefined;
    rejectedCapabilities?: RejectedCapability[] | undefined;
}
/** WSS ready exchange uses the same generated control types as hello. */
export interface ReadyRequest {
    negotiationId?: string | undefined;
    profileId?: string | undefined;
    plane?: Plane | undefined;
    provides?: string[] | undefined;
    limits?: Limits | undefined;
}
export interface ReadyResponse {
    negotiationId?: string | undefined;
    profileId?: string | undefined;
    plane?: Plane | undefined;
    provides?: string[] | undefined;
    limits?: Limits | undefined;
    linkIdentity?: LinkIdentity | undefined;
}
export declare const HelloRequest: MessageFns<HelloRequest>;
export declare const RejectedCapability: MessageFns<RejectedCapability>;
export declare const HelloResponse: MessageFns<HelloResponse>;
export declare const LinkIdentity: MessageFns<LinkIdentity>;
export declare const NegotiatedCapabilities: MessageFns<NegotiatedCapabilities>;
export declare const ReadyRequest: MessageFns<ReadyRequest>;
export declare const ReadyResponse: MessageFns<ReadyResponse>;
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
