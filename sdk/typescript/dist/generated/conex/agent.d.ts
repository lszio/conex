import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { RejectedCapability } from "./control";
export declare const protobufPackage = "conex";
/**
 * Registration is authenticated by the transport credential; no secret is
 * carried in this message or returned by the host.
 */
export interface AgentEndpoint {
    endpointId?: string | undefined;
    root?: string | undefined;
    methods?: string[] | undefined;
}
export interface AgentRegisterRequest {
    agentId?: string | undefined;
    tenantId?: string | undefined;
    endpoints?: AgentEndpoint[] | undefined;
    provides?: string[] | undefined;
    hostOrigin?: string | undefined;
}
export interface AgentRegisterResult {
    agentId?: string | undefined;
    linkId?: string | undefined;
    acceptedEndpointIds?: string[] | undefined;
    rejectedCapabilities?: RejectedCapability[] | undefined;
}
export declare const AgentEndpoint: MessageFns<AgentEndpoint>;
export declare const AgentRegisterRequest: MessageFns<AgentRegisterRequest>;
export declare const AgentRegisterResult: MessageFns<AgentRegisterResult>;
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
