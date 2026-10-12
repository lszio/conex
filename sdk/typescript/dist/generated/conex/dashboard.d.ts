import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
export declare const protobufPackage = "conex";
/**
 * Connection panel messages (L10). Real-time view of every browser link and
 * every reverse-connected agent link on the host. Only ui role callers can
 * inspect the UI link rows; service/agent roles are rejected at the broker.
 */
export interface ConnectionListRequest {
}
export interface UiLinkSummary {
    linkId?: string | undefined;
    principalId?: string | undefined;
    tenantId?: string | undefined;
    connectedAtMs?: string | undefined;
    lastSeenAtMs?: string | undefined;
    ticketsIssued?: string | undefined;
    callsTotal?: string | undefined;
    callsInFlight?: string | undefined;
}
export interface AgentLinkSummary {
    agentId?: string | undefined;
    principalId?: string | undefined;
    tenantId?: string | undefined;
    generation?: string | undefined;
    registeredAtMs?: string | undefined;
    lastHeartbeatAtMs?: string | undefined;
    endpointIds?: string[] | undefined;
}
export interface ConnectionListResponse {
    browserLinks?: UiLinkSummary[] | undefined;
    agentLinks?: AgentLinkSummary[] | undefined;
}
/**
 * Client panel messages. A "client" is a browser visitor: one UI link with a
 * self-declared name, group and visibility. Callers may only see clients that
 * have `visible` set; hidden clients are not listed to anyone, not even to
 * themselves as rows (their own row is still addressable by linkId).
 */
export interface ClientListRequest {
}
export interface ClientProfile {
    /**
     * Self-declared, session-scoped. Never authenticated, never reused as an
     * identity: two clients may pick the same name.
     */
    displayName?: string | undefined;
    /**
     * The join secret of the visitor's group: knowledge of the name is the only
     * requirement to share with that group, so the host never projects it to
     * anyone but its owner (`client/profile` self). Empty in every listing row.
     */
    group?: string | undefined;
    /** When false the client disappears from every other client's list. */
    visible?: boolean | undefined;
}
export interface ClientSummary {
    linkId?: string | undefined;
    principalId?: string | undefined;
    tenantId?: string | undefined;
    profile?: ClientProfile | undefined;
    connectedAtMs?: string | undefined;
    lastSeenAtMs?: string | undefined;
    /** Self-reported by the client's own page; never trusted for access. */
    userAgent?: string | undefined;
    /**
     * Host-derived hash of `profile.group`. The group text is a self-declared
     * label; this hash is the isolation key, so two clients are peers only when
     * their keys match.
     */
    groupKey?: string | undefined;
    /**
     * Last round trip this client was measured on, in milliseconds. 0 means no
     * hello has been exchanged with it yet.
     */
    lastRoundTripMs?: string | undefined;
}
export interface ClientListResponse {
    clients?: ClientSummary[] | undefined;
}
export interface ClientProfileRequest {
    profile?: ClientProfile | undefined;
}
export interface ClientProfileResponse {
    self?: ClientSummary | undefined;
}
/**
 * Sends a greeting to one online client. The host pushes
 * `conex/client-hello` to the target link; the target answers with
 * `conex/client-pong`, which the host reports back to the sender together
 * with the measured round-trip time. An offline or hidden target fails with
 * `unavailable` rather than being silently dropped.
 */
export interface ClientHelloRequest {
    targetLinkId?: string | undefined;
    text?: string | undefined;
    /**
     * Callback arguments: the fields the sender wants answered. An empty
     * payload is a bare greeting — the target's SDK acknowledges it
     * immediately, so a latency sample never waits on a human. A non-empty
     * payload makes it a question: the frame is pushed with `ask`, the
     * auto-acknowledge is skipped, and the round trip is only measured once the
     * target actually answers it. Untrusted and length-capped by the host.
     */
    payload?: {
        [key: string]: any;
    } | undefined;
}
export interface ClientHelloResult {
    targetLinkId?: string | undefined;
    /**
     * Round-trip milliseconds measured by the host, from handing the frame to
     * the target's socket until its answer is read back.
     */
    roundTripMs?: string | undefined;
    reply?: string | undefined;
    /**
     * The target's answer to `payload`, echoed back verbatim. An empty object
     * when the target acknowledged without answering, or answered with nothing
     * the host could parse — never a fabricated value.
     */
    answer?: {
        [key: string]: any;
    } | undefined;
}
/**
 * `client/status`: what the host sees right now. Aggregate counters plus one
 * row per group, so a page can show "N clients, M groups" and the measured
 * latency of every group without polling anything else.
 *
 * Latency is only ever a real measurement: `lastRoundTripMs` and
 * `avgRoundTripMs` are 0 until a hello has completed, and 0 in the JSON plane
 * is rendered as "<1 ms", never as a fabricated number.
 */
export interface ClientStatusRequest {
}
export interface GroupStatus {
    /**
     * Host-derived isolation key; the group text itself is a display label the
     * visitor chose and is not part of the key.
     */
    groupKey?: string | undefined;
    /**
     * Always empty: the group text is the join secret and is never projected
     * into host-wide stats. Kept for wire compatibility.
     */
    label?: string | undefined;
    clientsOnline?: number | undefined;
    lastRoundTripMs?: string | undefined;
    avgRoundTripMs?: string | undefined;
    roundTrips?: number | undefined;
    filesShared?: number | undefined;
    sharedBytes?: string | undefined;
}
export interface ClientStatusResponse {
    clientsOnline?: number | undefined;
    groupsOnline?: number | undefined;
    groups?: GroupStatus[] | undefined;
}
export declare const ConnectionListRequest: MessageFns<ConnectionListRequest>;
export declare const UiLinkSummary: MessageFns<UiLinkSummary>;
export declare const AgentLinkSummary: MessageFns<AgentLinkSummary>;
export declare const ConnectionListResponse: MessageFns<ConnectionListResponse>;
export declare const ClientListRequest: MessageFns<ClientListRequest>;
export declare const ClientProfile: MessageFns<ClientProfile>;
export declare const ClientSummary: MessageFns<ClientSummary>;
export declare const ClientListResponse: MessageFns<ClientListResponse>;
export declare const ClientProfileRequest: MessageFns<ClientProfileRequest>;
export declare const ClientProfileResponse: MessageFns<ClientProfileResponse>;
export declare const ClientHelloRequest: MessageFns<ClientHelloRequest>;
export declare const ClientHelloResult: MessageFns<ClientHelloResult>;
export declare const ClientStatusRequest: MessageFns<ClientStatusRequest>;
export declare const GroupStatus: MessageFns<GroupStatus>;
export declare const ClientStatusResponse: MessageFns<ClientStatusResponse>;
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
