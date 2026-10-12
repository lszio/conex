import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
export declare const protobufPackage = "conex";
/** Configured attachment kind. A configured endpoint is not necessarily online. */
export declare enum AttachmentKind {
    ATTACHMENT_KIND_UNSPECIFIED = 0,
    ATTACHMENT_KIND_EMBEDDED = 1,
    ATTACHMENT_KIND_OUTBOUND_HTTP = 2,
    ATTACHMENT_KIND_REVERSE_AGENT = 3,
    UNRECOGNIZED = -1
}
export declare function attachmentKindFromJSON(object: any): AttachmentKind;
export declare function attachmentKindToJSON(object: AttachmentKind): string;
/** Runtime connection state projected into the endpoint catalog. */
export declare enum ConnectionState {
    CONNECTION_STATE_UNSPECIFIED = 0,
    CONNECTION_STATE_NOT_APPLICABLE = 1,
    CONNECTION_STATE_OFFLINE = 2,
    CONNECTION_STATE_READY = 3,
    UNRECOGNIZED = -1
}
export declare function connectionStateFromJSON(object: any): ConnectionState;
export declare function connectionStateToJSON(object: ConnectionState): string;
export interface AuthorizedScope {
    method?: string | undefined;
    root?: string | undefined;
    subtree?: boolean | undefined;
}
export interface EndpointSummary {
    endpointId?: string | undefined;
    displayName?: string | undefined;
    providerId?: string | undefined;
    attachmentKind?: AttachmentKind | undefined;
    agentId?: string | undefined;
    region?: string | undefined;
    connectionState?: ConnectionState | undefined;
    availableMethods?: string[] | undefined;
    authorizedScopes?: AuthorizedScope[] | undefined;
}
export interface EndpointListRequest {
    limit?: number | undefined;
    afterEndpointId?: string | undefined;
}
export interface EndpointListResult {
    endpoints?: EndpointSummary[] | undefined;
    nextAfterEndpointId?: string | undefined;
}
export declare const AuthorizedScope: MessageFns<AuthorizedScope>;
export declare const EndpointSummary: MessageFns<EndpointSummary>;
export declare const EndpointListRequest: MessageFns<EndpointListRequest>;
export declare const EndpointListResult: MessageFns<EndpointListResult>;
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
