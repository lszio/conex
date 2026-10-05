import { ConexError, toConexError } from "./errors";
import type { Limits } from "./generated/conex/common";
import type { LinkIdentity } from "./generated/conex/control";
import { mintUlid } from "./client";
import type {
  ClientHelloResult,
  ClientListResponse,
  ClientProfile,
  ClientProfileResponse,
  ClientStatusResponse,
  ConnectionListResponse,
} from "./generated/conex/dashboard";
import type {
  EndpointListRequest,
  EndpointListResult,
} from "./generated/conex/endpoint";
import type {
  SourceListRequest,
  SourceListResponse,
  SourceReadRequest,
  SourceReadResponse,
  SourceSearchRequest,
  SourceSearchResponse,
} from "./generated/conex/source";

const PROFILE_ID = "conex-jsonrpc2-wss";
const PLANE = "broker";
const DEFAULT_TIMEOUT_MS = 8000;
const DEFAULT_TICKET_PATH = "/tickets";
const DEFAULT_WS_PATH = "/wss";
const MAX_FRAME_BYTES = 1024 * 1024;
/// Calls that may wait for a reconnect. A page polls on a timer, so a long
/// outage must not turn into an unbounded pile of resends that all arrive at
/// once when the link comes back.
const MAX_QUEUED_CALLS = 32;
const RECONNECT_DELAY_MS = 100;

type WebSocketHandler = (event: { data: unknown }) => void;

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

export type ConexWsState =
  | "idle"
  | "authenticating"
  | "connecting"
  | "negotiating"
  | "ready"
  | "reconnecting"
  | "closed"
  | "failed";

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
type ConexWsEventInput = Omit<ConexWsEvent, "at">;
export type ConexWsEventListener = (event: ConexWsEvent) => void;

interface PendingCall {
  method: string;
  /** The exact frame that was sent, kept so a reconnect can resend it. */
  message: string;
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
  timer: ReturnType<typeof setTimeout>;
}

/** A call that survived a disconnect and is waiting for the next link. */
interface QueuedCall {
  id: string;
  method: string;
  message: string;
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
}

interface Ticket {
  value: string;
  expiresAt: number;
}

interface JsonRpcMessage {
  jsonrpc?: unknown;
  id?: unknown;
  method?: unknown;
  params?: unknown;
  result?: unknown;
  error?: unknown;
}

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

export class ConexWsClient {
  private readonly origin: string;
  private readonly csrfToken?: string;
  private readonly ticketPath: string;
  private readonly wsPath: string;
  private readonly reconnectEnabled: boolean;
  private readonly timeoutMs: number;
  private readonly fetchImpl: WsFetch;
  private readonly WebSocketImpl: WebSocketConstructor;
  private currentState: ConexWsState = "idle";
  private socket?: WebSocketLike;
  private ticket?: Ticket;
  private connectPromise?: Promise<void>;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private negotiationResult?: ConexWsNegotiation;
  /// Set from the ticket response; the getter above is the public shape.
  private linkIdField?: string;
  private readonly listeners = new Set<ConexWsEventListener>();
  private readonly helloListeners = new Set<(hello: ConexHelloPush) => void>();
  private readonly pending = new Map<string, PendingCall>();
  /** Calls issued while the link was down, replayed in order on reconnect. */
  private readonly queued: QueuedCall[] = [];
  private closed = false;
  private handshake?: { step: "hello" | "ready"; helloId: string; negotiationId: string; helloResult?: Record<string, unknown> };

  constructor(options: ConexWsClientOptions) {
    this.origin = options.origin;
    this.csrfToken = options.csrfToken;
    this.ticketPath = options.ticketPath ?? DEFAULT_TICKET_PATH;
    this.wsPath = options.wsPath ?? DEFAULT_WS_PATH;
    this.reconnectEnabled = options.reconnect ?? true;
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.fetchImpl = options.fetch ?? ((url, init) => globalThis.fetch(url, init));
    this.WebSocketImpl = options.WebSocket ?? (globalThis.WebSocket as unknown as WebSocketConstructor);
    if (!this.WebSocketImpl) {
      throw new Error("WebSocket is not available");
    }
  }

  get negotiation(): ConexWsNegotiation | undefined {
    return this.negotiationResult;
  }

  get state(): ConexWsState {
    return this.currentState;
  }

  /**
   * The link this client's ticket opened, once the ticket has been issued.
   *
   * It is the tab's own identity: every browser tab on one cookie holds a
   * different one, and the same-origin HTTP routes need it to answer "who am
   * I" without guessing from the cookie. Undefined before the first ticket.
   */
  get linkId(): string | undefined {
    return this.linkIdField;
  }

  onEvent(listener: ConexWsEventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  connect(): Promise<void> {
    if (this.currentState === "ready") return Promise.resolve();
    if (this.closed || this.currentState === "closed") {
      return Promise.reject(new ConexError("client is closed", { code: -32011 }));
    }
    if (this.connectPromise) return this.connectPromise;
    const promise = this.establish(false).catch((error) => {
      if (!this.closed) this.setState("failed");
      throw toConexError(error);
    });
    const tracked = promise.finally(() => {
      if (this.connectPromise === tracked) this.connectPromise = undefined;
    });
    this.connectPromise = tracked;
    return tracked;
  }

  listEndpoints(input: EndpointListRequest = {}): Promise<EndpointListResult> {
    return this.call<EndpointListResult>("endpoint/list", "", input);
  }

  listConnections(input: Record<string, never> = {}): Promise<ConnectionListResponse> {
    return this.call<ConnectionListResponse>("connection/list", "", input);
  }

  /** Online clients that chose to be visible, in the caller's own group. */
  listClients(input: Record<string, never> = {}): Promise<ClientListResponse> {
    return this.call<ClientListResponse>("client/list", "", input);
  }

  /**
   * Host-wide counters plus one row per group: how many clients and groups are
   * on this host, and the latency actually measured in each group.
   *
   * The latency fields are 0 (as strings) until a hello in that group has
   * completed. Treat 0 as "not measured" rather than as a fast connection.
   */
  listClientStatus(input: Record<string, never> = {}): Promise<ClientStatusResponse> {
    return this.call<ClientStatusResponse>("client/status", "", input);
  }

  /** Declare this visitor's own name, group and visibility. */
  setProfile(profile: ClientProfile): Promise<ClientProfileResponse> {
    return this.call<ClientProfileResponse>("client/profile", "", { profile });
  }

  /**
   * Greet another client; resolves once that client answers.
   *
   * A `payload` makes it a question rather than a greeting: the target is not
   * acknowledged automatically, and its `answer` comes back in the result. A
   * bare hello resolves as soon as the target's SDK acknowledges, which is what
   * a latency sample wants; a payload resolves when a person or a handler has
   * actually answered, which is what a question wants.
   */
  sendHello(
    targetLinkId: string,
    text = "hello",
    payload?: Record<string, unknown>,
  ): Promise<ClientHelloResult> {
    return this.call<ClientHelloResult>("client/hello", "", {
      targetLinkId,
      text,
      ...(payload ? { payload } : {}),
    });
  }

  /**
   * Send a JSON-RPC notification, which carries no reply. Used to answer a
   * pushed `conex/client-hello`; a request id would make the host wait for a
   * response that never comes.
   */
  notify(method: string, params: unknown): boolean {
    if (this.currentState !== "ready" || !this.socket) return false;
    try {
      this.socket.send(JSON.stringify({ jsonrpc: "2.0", method, params }));
      return true;
    } catch {
      return false;
    }
  }

  list(endpointId: string, input: SourceListRequest): Promise<SourceListResponse> {
    return this.call<SourceListResponse>("source/list", endpointId, input);
  }

  read(endpointId: string, input: SourceReadRequest): Promise<SourceReadResponse> {
    return this.call<SourceReadResponse>("source/read", endpointId, input);
  }

  search(endpointId: string, input: SourceSearchRequest): Promise<SourceSearchResponse> {
    return this.call<SourceSearchResponse>("source/search", endpointId, input);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.currentState = "closed";
    this.emit({ phase: "closed", summary: "client closed" });
    this.rejectPending(new ConexError("client is closed", { code: -32011 }));
    // Queued calls are waiting for a link that will never come, so they are
    // failed here too; otherwise their promises would never settle.
    for (const call of this.queued) {
      call.reject(new ConexError("client is closed", { code: -32011 }));
    }
    this.queued.length = 0;
    const socket = this.socket;
    this.socket = undefined;
    try {
      socket?.close();
    } catch {
      // A browser may throw when close races a failed upgrade.
    }
  }
  private async establish(reconnecting: boolean): Promise<void> {
    if (this.closed) throw new ConexError("client is closed", { code: -32011 });
    this.setState(reconnecting ? "reconnecting" : "authenticating");
    const ticket = await this.getTicket(reconnecting);
    if (this.closed) throw new ConexError("client is closed", { code: -32011 });
    this.setState("connecting");
    const socket = new this.WebSocketImpl(this.websocketUrl(ticket));
    this.socket = socket;
    this.setState("negotiating");
    await this.negotiate(socket);
    if (this.closed || this.socket !== socket) throw new ConexError("client is closed", { code: -32011 });
    this.setState("ready");
    this.emit({ phase: "ready", summary: "ready" });
    // The link is back: anything the outage interrupted goes out now, before
    // the page issues its next poll, so the visitor sees one refresh instead
    // of an error followed by a stale list.
    this.flushQueued();
  }

  private getTicket(force = false): Promise<string> {
    const now = Date.now();
    if (!force && this.ticket && this.ticket.expiresAt > now + 1000) {
      return Promise.resolve(this.ticket.value);
    }
    this.emit({ phase: "ticket", summary: "requesting ticket" });
    const headers = new Headers({ "content-type": "application/json" });
    if (this.csrfToken) headers.set("x-csrf-token", this.csrfToken);
    return this.fetchImpl(new URL(this.ticketPath, this.origin), {
      method: "POST",
      credentials: "include",
      headers,
      body: "{}",
      redirect: "error",
    }).then(async (response) => {
      let body: unknown;
      try {
        body = await response.json();
      } catch {
        throw new ConexError("invalid ticket response", { status: response.status });
      }
      const ticketBody = body && typeof body === "object" ? body as Record<string, unknown> : {};
      if (!response.ok) {
        throw errorFrom(ticketBody.error ?? ticketBody, response.status);
      }
      if (typeof ticketBody.ticket !== "string" || !ticketBody.ticket) {
        throw new ConexError("ticket response is missing ticket", { status: response.status });
      }
      if (typeof ticketBody.linkId === "string" && ticketBody.linkId) {
        this.linkIdField = ticketBody.linkId;
      }
      const expiresAt = parseExpiry(ticketBody.expiresAt ?? ticketBody.expiresAtMs);
      this.ticket = { value: ticketBody.ticket, expiresAt };
      return ticketBody.ticket;
    });
  }

  private negotiate(socket: WebSocketLike): Promise<void> {
    const deferred = deferredPromise<void>();
    let settled = false;
    const helloId = mintUlid();
    this.handshake = { step: "hello", helloId, negotiationId: "" };
    const timer = setTimeout(() => fail(new ConexError("websocket handshake timed out", { code: -32006 })), this.timeoutMs);
    const fail = (error: unknown) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (this.socket === socket) this.socket = undefined;
      try {
        socket.close();
      } catch {
        // Ignore close races.
      }
      deferred.reject(toConexError(error));
    };
    const succeed = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      deferred.resolve();
    };
    socket.onopen = () => {
      try {
        socket.send(JSON.stringify({
          jsonrpc: "2.0",
          id: helloId,
          method: "conex/hello",
          params: { profileId: PROFILE_ID, plane: PLANE, provides: [], requires: [] },
        }));
      } catch (error) {
        fail(new ConexError("websocket send failed", { details: String(error) }));
      }
    };
    socket.onmessage = (event) => {
      if (typeof event?.data !== "string") {
        fail(new ConexError("websocket accepts text frames only", { code: -32600 }));
        return;
      }
      if (new TextEncoder().encode(event.data).byteLength > MAX_FRAME_BYTES) {
        fail(new ConexError("websocket frame exceeds 1MiB", { code: -32007 }));
        return;
      }
      let message: JsonRpcMessage;
      try {
        message = JSON.parse(event.data) as JsonRpcMessage;
      } catch {
        fail(new ConexError("invalid websocket JSON", { code: -32700 }));
        return;
      }
      if (this.currentState !== "negotiating") {
        // A server-initiated hello carries no id: it must be answered as a
        // notification, then surfaced so the page can show it.
        if (message.id === undefined && typeof message.method === "string") {
          if (this.handleServerNotification(message)) return;
        }
        this.routeBusiness(message);
        return;
      }
      try {
        this.routeHandshake(socket, message, fail, succeed);
      } catch (error) {
        fail(error);
      }
    };
    socket.onerror = () => {
      if (settled && this.socket === socket) this.handleTransportClose(socket);
      else fail(new ConexError("websocket transport error", { code: -32000 }));
    };
    socket.onclose = () => {
      if (!settled) fail(new ConexError("websocket closed during handshake", { code: -32000 }));
      else if (this.socket === socket) this.handleTransportClose(socket);
    };
    return deferred.promise;
  }

  private routeHandshake(socket: WebSocketLike, message: JsonRpcMessage, _fail: (error: unknown) => void, succeed: () => void): void {
    const handshake = this.handshake;
    if (!handshake || (handshake.step === "hello" && message.id !== handshake.helloId)) {
      throw new ConexError("unexpected websocket handshake response", { code: -32600 });
    }
    if (message.error) throw errorFrom(message.error);
    if (!message.result || typeof message.result !== "object") throw new ConexError("websocket handshake result is missing", { code: -32600 });
    const result = message.result as Record<string, unknown>;
    if (handshake.step === "hello") {
      if (typeof result.negotiationId !== "string" || result.profileId !== PROFILE_ID || result.plane !== PLANE) {
        throw new ConexError("invalid websocket hello result", { code: -32600 });
      }
      if (!Array.isArray(result.provides) || !result.limits) throw new ConexError("invalid websocket hello capabilities", { code: -32600 });
      const readyId = mintUlid();
      handshake.step = "ready";
      handshake.negotiationId = result.negotiationId;
      handshake.helloResult = result;
      socket.send(JSON.stringify({
        jsonrpc: "2.0",
        id: readyId,
        method: "conex/ready",
        params: {
          negotiationId: result.negotiationId,
          profileId: PROFILE_ID,
          plane: PLANE,
          provides: result.provides,
          limits: result.limits,
        },
      }));
      handshake.helloId = readyId;
      this.emit({ phase: "ready", requestId: readyId, method: "conex/ready", summary: "sent" });
      return;
    }
    if (message.id !== handshake.helloId) throw new ConexError("unexpected websocket ready response", { code: -32600 });
    if (result.negotiationId !== handshake.negotiationId || result.profileId !== PROFILE_ID || result.plane !== PLANE) {
      throw new ConexError("invalid websocket ready result", { code: -32600 });
    }
    this.negotiationResult = buildNegotiation(result, handshake.helloResult);
    succeed();
  }

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
  private handleServerNotification(message: JsonRpcMessage): boolean {
    if (message.method !== "conex/client-hello") return false;
    const params = (message.params ?? {}) as {
      replyId?: string;
      from?: string;
      fromName?: string;
      text?: string;
      ask?: boolean;
      payload?: Record<string, unknown>;
    };
    if (params.replyId && params.ask !== true) {
      this.notify("conex/client-pong", { replyId: params.replyId, reply: "pong" });
    }
    this.emit({
      phase: "request",
      method: "conex/client-hello",
      summary: params.text ?? "hello",
    });
    const hello = {
      replyId: params.replyId ?? "",
      from: params.from ?? "",
      // The host resolves the sender's link to its display name, so a client
      // that never chose one is still greeted by something readable.
      fromName: params.fromName ?? "",
      text: params.text ?? "hello",
      ask: params.ask === true,
      payload: params.payload ?? {},
    };
    this.helloListeners.forEach((listener) => listener(hello));
    return true;
  }

  /**
   * Answer a question the host pushed, with the fields the sender asked about.
   *
   * Returns false when the hello needs no answer — it was a bare greeting, or
   * it carries no `replyId` — so a caller can fire it from a shared handler
   * without checking first. A non-object answer is refused here rather than
   * pushed: the host drops it, and failing loudly beats a silent timeout on
   * the other side.
   */
  answerHello(replyId: string, answer: Record<string, unknown>, reply = "answered"): boolean {
    if (!replyId) return false;
    if (typeof answer !== "object" || answer === null || Array.isArray(answer)) {
      throw new ConexError("hello answer must be an object", { code: -32602 });
    }
    return this.notify("conex/client-pong", { replyId, reply, answer });
  }

  /** Greetings pushed by other clients. Returns an unsubscribe function. */
  onHello(
    listener: (hello: ConexHelloPush) => void,
  ): () => void {
    this.helloListeners.add(listener);
    return () => this.helloListeners.delete(listener);
  }

  private routeBusiness(message: JsonRpcMessage): void {
    if (message.jsonrpc !== "2.0" || (typeof message.id !== "string" && typeof message.id !== "number")) return;
    const id = String(message.id);
    const call = this.pending.get(id);
    if (!call) return;
    this.pending.delete(id);
    clearTimeout(call.timer);
    if (message.error) {
      call.reject(errorFrom(message.error));
      this.emit({ phase: "error", requestId: id, method: call.method, summary: "rpc error" });
      return;
    }
    call.resolve(message.result);
    this.emit({ phase: "response", requestId: id, method: call.method, summary: "success" });
  }

  private call<T>(method: string, endpointId: string, input: unknown): Promise<T> {
    if (this.currentState !== "ready" || !this.socket) {
      return Promise.reject(new ConexError("websocket is not ready", { code: -32010 }));
    }
    const id = mintUlid();
    const params: Record<string, unknown> = {
      context: { providerEndpointId: endpointId, plane: PLANE },
      timeoutBudgetMs: this.timeoutMs,
      input,
    };
    const message = JSON.stringify({ jsonrpc: "2.0", id, method, params });
    if (new TextEncoder().encode(message).byteLength > MAX_FRAME_BYTES) {
      return Promise.reject(new ConexError("websocket frame exceeds 1MiB", { code: -32007 }));
    }
    const deferred = deferredPromise<unknown>();
    const timer = setTimeout(() => {
      this.pending.delete(id);
      deferred.reject(new ConexError("websocket request timed out", { code: -32006 }));
    }, this.timeoutMs);
    this.pending.set(id, {
      method,
      message,
      resolve: deferred.resolve,
      reject: deferred.reject,
      timer,
    });
    try {
      this.socket.send(message);
      this.emit({ phase: "request", requestId: id, method, summary: "sent" });
    } catch (error) {
      clearTimeout(timer);
      this.pending.delete(id);
      deferred.reject(toConexError(error));
    }
    return deferred.promise as Promise<T>;
  }
  private handleTransportClose(socket: WebSocketLike): void {
    if (this.socket !== socket || this.closed) return;
    this.socket = undefined;
    this.negotiationResult = undefined;
    // A call that was in flight when the socket dropped is not a failed call.
    // The reconnect re-opens the link, and these are ordinary read calls
    // (list clients, list files) that are valid a moment later, so they are
    // put back on the queue and sent again once the link is ready. Failing
    // them here instead is what made a backgrounded tab look broken: the
    // page's poll loop threw on every tick and the visitor saw an error
    // rather than a slow refresh.
    this.requeuePending();
    this.emit({ phase: "close", summary: "transport closed" });
    if (!this.reconnectEnabled) {
      this.rejectPending(new ConexError("websocket connection closed", { code: -32000 }));
      this.setState("failed");
      return;
    }
    this.setState("reconnecting");
    this.scheduleReconnect();
  }

  /**
   * Move in-flight calls back onto the queue for the next connection.
   *
   * Their timers keep running, so a call that never gets an answer still
   * times out on its own budget instead of hanging forever.
   */
  private requeuePending(): void {
    if (this.queued.length >= MAX_QUEUED_CALLS) {
      this.rejectPending(
        new ConexError("too many calls queued while reconnecting", { code: -32012 }),
      );
      return;
    }
    for (const [id, call] of this.pending) {
      this.queued.push({
        id,
        method: call.method,
        message: call.message,
        resolve: call.resolve,
        reject: call.reject,
      });
      this.pending.delete(id);
    }
  }

  /** Send whatever survived a reconnect, oldest first. */
  private flushQueued(): void {
    while (this.queued.length > 0) {
      const next = this.queued[0];
      if (!this.socket) return;
      this.queued.shift();
      const timer = setTimeout(() => {
        this.pending.delete(next.id);
        next.reject(new ConexError("websocket request timed out", { code: -32006 }));
      }, this.timeoutMs);
      this.pending.set(next.id, { ...next, resolve: next.resolve, reject: next.reject, timer });
      try {
        this.socket.send(next.message);
      } catch (error) {
        clearTimeout(timer);
        this.pending.delete(next.id);
        next.reject(toConexError(error));
      }
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer || this.closed) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.establish(true).catch(() => {
        if (!this.closed) {
          this.setState("reconnecting");
          this.scheduleReconnect();
        }
      });
    }, RECONNECT_DELAY_MS);
  }

  private rejectPending(error: ConexError): void {
    for (const [id, call] of this.pending) {
      clearTimeout(call.timer);
      call.reject(error);
      this.pending.delete(id);
    }
  }

  private setState(state: ConexWsState): void {
    this.currentState = state;
    this.emit({ phase: state });
  }

  private emit(event: ConexWsEventInput): void {
    const safe: ConexWsEvent = { ...event, at: Date.now() };
    for (const listener of this.listeners) {
      try {
        listener(safe);
      } catch {
        // Event observers must not break the transport.
      }
    }
  }

  private websocketUrl(ticket: string): string {
    const url = new URL(this.wsPath, this.origin);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("ticket", ticket);
    return url.toString();
  }
}

function deferredPromise<T>(): { promise: Promise<T>; resolve: (value: T) => void; reject: (reason: unknown) => void } {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function parseExpiry(value: unknown): number {
  if (typeof value === "number") return value < 10_000_000_000 ? value * 1000 : value;
  if (typeof value === "string") {
    const number = Number(value);
    if (Number.isFinite(number)) return number < 10_000_000_000 ? number * 1000 : number;
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return Date.now() + 30_000;
}

function buildNegotiation(result: Record<string, unknown>, helloResult?: Record<string, unknown>): ConexWsNegotiation {
  const provides = stringArray(result.provides, "provides");
  const requires = stringArray(result.requires ?? helloResult?.requires ?? [], "requires");
  const limits = normalizeLimits(result.limits);
  const identity = normalizeIdentity(result.linkIdentity);
  return {
    negotiationId: typeof result.negotiationId === "string" ? result.negotiationId : "",
    profileId: typeof result.profileId === "string" ? result.profileId : "",
    plane: typeof result.plane === "string" ? result.plane : "",
    provides,
    requires,
    limits,
    ...(identity ? { linkIdentity: identity } : {}),
  };
}

function stringArray(value: unknown, field: string): string[] {
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")) {
    throw new ConexError(`invalid websocket ${field}`, { code: -32600 });
  }
  return [...value];
}

function normalizeLimits(value: unknown): Limits {
  if (!value || typeof value !== "object") throw new ConexError("invalid websocket limits", { code: -32600 });
  const record = value as Record<string, unknown>;
  const limits: Limits = {};
  for (const field of ["maxFrameBytes", "maxInflight", "maxQueuedBytes", "timeoutMs"] as const) {
    if (typeof record[field] !== "number" || !Number.isFinite(record[field])) {
      throw new ConexError(`invalid websocket limits.${field}`, { code: -32600 });
    }
    limits[field] = record[field];
  }
  return limits;
}

function normalizeIdentity(value: unknown): LinkIdentity | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  const identity: LinkIdentity = {};
  for (const field of ["linkId", "peerId", "tenantId"] as const) {
    if (typeof record[field] === "string") identity[field] = record[field];
  }
  return identity;
}

function errorFrom(error: unknown, status?: number): ConexError {
  const record = error && typeof error === "object" ? error as Record<string, unknown> : {};
  const data = record.data && typeof record.data === "object" ? record.data as Record<string, unknown> : record;
  const message = typeof record.message === "string"
    ? record.message
    : typeof data.message === "string"
      ? data.message
      : "websocket rpc error";
  return new ConexError(message, {
    code: typeof record.code === "number" ? record.code : typeof data.code === "number" ? data.code : undefined,
    diagnosticId: typeof data.diagnosticId === "string" ? data.diagnosticId : undefined,
    execution: typeof data.execution === "string" ? data.execution : undefined,
    retry: typeof data.retry === "string" ? data.retry : undefined,
    details: data.details,
    status,
  });
}
