import type { Limits } from "./generated/conex/common";
import type { LinkIdentity } from "./generated/conex/control";
import type { ClientHelloResult, ClientListResponse, ClientProfile, ClientProfileResponse, ClientStatusResponse, ConnectionListResponse } from "./generated/conex/dashboard";
import type { EndpointListRequest, EndpointListResult } from "./generated/conex/endpoint";
import type { SourceListRequest, SourceListResponse, SourceReadRequest, SourceReadResponse, SourceSearchRequest, SourceSearchResponse } from "./generated/conex/source";
type WebSocketHandler = (event: {
    data: unknown;
}) => void;
export interface WebSocketLike {
    onopen: (() => void) | null;
    onmessage: WebSocketHandler | null;
    onerror: (() => void) | null;
    onclose: (() => void) | null;
    send(data: string): void;
    close(): void;
}
export type WebSocketConstructor = new (url: string, protocols?: string | string[]) => WebSocketLike;
export type WsFetch = (url: string | URL, init?: RequestInit) => Promise<Response>;
export type ConexWsState = "idle" | "authenticating" | "connecting" | "negotiating" | "ready" | "reconnecting" | "closed" | "failed";
export interface ConexWsClientOptions {
    origin: string;
    csrfToken?: string;
    ticketPath?: string;
    wsPath?: string;
    reconnect?: boolean;
    timeoutMs?: number;
    WebSocket?: WebSocketConstructor;
    fetch?: WsFetch;
}
export interface ConexWsEvent {
    phase: ConexWsState | "ticket" | "hello" | "ready" | "request" | "response" | "close" | "error";
    at: number;
    requestId?: string;
    method?: string;
    summary?: string;
}
export interface ConexWsNegotiation {
    negotiationId: string;
    profileId: string;
    plane: string;
    provides: string[];
    requires: string[];
    limits: Limits;
    linkIdentity?: LinkIdentity;
}
export type ConexWsEventListener = (event: ConexWsEvent) => void;
/**
 * A greeting or a question the host pushed to this client.
 *
 * `ask` is the whole distinction: false is a greeting this client has already
 * acknowledged, true is a question whose answer only the page can supply.
 */
export interface ConexHelloPush {
    replyId: string;
    from: string;
    fromName: string;
    text: string;
    ask: boolean;
    /** The sender's callback arguments. Empty unless `ask` is true. */
    payload: Record<string, unknown>;
}
export declare class ConexWsClient {
    private readonly origin;
    private readonly csrfToken?;
    private readonly ticketPath;
    private readonly wsPath;
    private readonly reconnectEnabled;
    private readonly timeoutMs;
    private readonly fetchImpl;
    private readonly WebSocketImpl;
    private currentState;
    private socket?;
    private ticket?;
    private connectPromise?;
    private reconnectTimer?;
    private negotiationResult?;
    private linkIdField?;
    private readonly listeners;
    private readonly helloListeners;
    private readonly pending;
    /** Calls issued while the link was down, replayed in order on reconnect. */
    private readonly queued;
    private closed;
    private handshake?;
    constructor(options: ConexWsClientOptions);
    get negotiation(): ConexWsNegotiation | undefined;
    get state(): ConexWsState;
    /**
     * The link this client's ticket opened, once the ticket has been issued.
     *
     * It is the tab's own identity: every browser tab on one cookie holds a
     * different one, and the same-origin HTTP routes need it to answer "who am
     * I" without guessing from the cookie. Undefined before the first ticket.
     */
    get linkId(): string | undefined;
    onEvent(listener: ConexWsEventListener): () => void;
    connect(): Promise<void>;
    listEndpoints(input?: EndpointListRequest): Promise<EndpointListResult>;
    listConnections(input?: Record<string, never>): Promise<ConnectionListResponse>;
    /** Online clients that chose to be visible, in the caller's own group. */
    listClients(input?: Record<string, never>): Promise<ClientListResponse>;
    /**
     * Host-wide counters plus one row per group: how many clients and groups are
     * on this host, and the latency actually measured in each group.
     *
     * The latency fields are 0 (as strings) until a hello in that group has
     * completed. Treat 0 as "not measured" rather than as a fast connection.
     */
    listClientStatus(input?: Record<string, never>): Promise<ClientStatusResponse>;
    /** Declare this visitor's own name, group and visibility. */
    setProfile(profile: ClientProfile): Promise<ClientProfileResponse>;
    /**
     * Greet another client; resolves once that client answers.
     *
     * A `payload` makes it a question rather than a greeting: the target is not
     * acknowledged automatically, and its `answer` comes back in the result. A
     * bare hello resolves as soon as the target's SDK acknowledges, which is what
     * a latency sample wants; a payload resolves when a person or a handler has
     * actually answered, which is what a question wants.
     */
    sendHello(targetLinkId: string, text?: string, payload?: Record<string, unknown>): Promise<ClientHelloResult>;
    /**
     * Send a JSON-RPC notification, which carries no reply. Used to answer a
     * pushed `conex/client-hello`; a request id would make the host wait for a
     * response that never comes.
     */
    notify(method: string, params: unknown): boolean;
    list(endpointId: string, input: SourceListRequest): Promise<SourceListResponse>;
    read(endpointId: string, input: SourceReadRequest): Promise<SourceReadResponse>;
    search(endpointId: string, input: SourceSearchRequest): Promise<SourceSearchResponse>;
    close(): void;
    private establish;
    private getTicket;
    private negotiate;
    private routeHandshake;
    /**
     * Answer a pushed `conex/client-hello`. Returns false for any other
     * notification so the caller can keep its normal routing.
     *
     * A greeting (`ask` absent) is answered right here, before any listener
     * runs, so the sender's latency sample measures the socket rather than the
     * page. A question (`ask: true`) is *not*: the sender is waiting for a real
     * answer, and acknowledging it automatically would hand back a measurement
     * nobody took. The listener answers it, or nobody does and the sender sees
     * the timeout.
     */
    private handleServerNotification;
    /**
     * Answer a question the host pushed, with the fields the sender asked about.
     *
     * Returns false when the hello needs no answer — it was a bare greeting, or
     * it carries no `replyId` — so a caller can fire it from a shared handler
     * without checking first. A non-object answer is refused here rather than
     * pushed: the host drops it, and failing loudly beats a silent timeout on
     * the other side.
     */
    answerHello(replyId: string, answer: Record<string, unknown>, reply?: string): boolean;
    /** Greetings pushed by other clients. Returns an unsubscribe function. */
    onHello(listener: (hello: ConexHelloPush) => void): () => void;
    private routeBusiness;
    private call;
    private handleTransportClose;
    /**
     * Move in-flight calls back onto the queue for the next connection.
     *
     * Their timers keep running, so a call that never gets an answer still
     * times out on its own budget instead of hanging forever.
     */
    private requeuePending;
    /** Send whatever survived a reconnect, oldest first. */
    private flushQueued;
    private scheduleReconnect;
    private rejectPending;
    private setState;
    private emit;
    private websocketUrl;
}
export {};
