//! Typed HTTP consumer for conex. Business params go in context/input; the
//! bearer token only ever appears in the Authorization header.
import { checkedLimits, HTTP_METHODS, targetContext, uncertain } from "./capability";
import type { CapabilityOptions, Invocation, InvokeOptions, ModuleReference, ExecutionClass } from "./capability";
import type { Limits } from "./generated/conex/common";
import { decodeS1Frame, validateFrame, validateResult } from "./s1-wire";
import type { S1Context } from "./s1-wire";
import { ConexError, toConexError } from "./errors";
import type {
  SourceListRequest,
  SourceListResponse,
  SourceReadRequest,
  SourceReadResponse,
  SourceSearchRequest,
  SourceSearchResponse,
} from "./generated/conex/source";

export type ClientFetch = (url: string | URL, init?: RequestInit) => Promise<Response>;

export interface ClientOptions {
  url: string;
  tokenProvider: () => Promise<string>;
  fetch?: ClientFetch;
  profileId?: string;
  requires?: string[];
  timeoutMs?: number;
  maxResponseBytes?: number;
  capabilities?: Pick<CapabilityOptions, "contracts">;
}

export interface SearchManyTarget {
  endpointId: string;
  input: SourceSearchRequest;
}

export interface SearchManyResult {
  endpointId: string;
  result?: SourceSearchResponse;
  error?: ConexError;
}

const PROFILE_ID = "conex-jsonrpc2-http";
const DEFAULT_TIMEOUT_MS = 8000;
const DEFAULT_MAX_RESPONSE_BYTES = 1024 * 1024;
const ULID_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

export function mintUlid(now: number = Date.now()): string {
  const bytes = new Uint8Array(16);
  let timestamp = BigInt(now);
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = Number(timestamp & 0xffn);
    timestamp >>= 8n;
  }
  const random = new Uint8Array(10);
  globalThis.crypto.getRandomValues(random);
  bytes.set(random, 6);
  let bits = 0n;
  for (const byte of bytes) {
    bits = (bits << 8n) | BigInt(byte);
  }
  let out = "";
  for (let index = 0; index < 26; index += 1) {
    out = ULID_ALPHABET[Number(bits & 31n)] + out;
    bits >>= 5n;
  }
  return out;
}

interface Binding {
  id: string;
  expiresAt: number;
}

export class ConexClient {
  private readonly url: string;
  private readonly tokenProvider: () => Promise<string>;
  private readonly fetchImpl: ClientFetch;
  private readonly profileId: string;
  private readonly requires: string[];
  private readonly timeoutMs: number;
  private readonly maxResponseBytes: number;
  private binding?: Binding;
  private closed = false;
  private readonly abort = new AbortController();
  private readonly capabilities?: Pick<CapabilityOptions, "contracts">;
  private s1Limits?: Required<Limits>;
  private bindingPromise?: Promise<void>;
  private s1Inflight = 0;

  private constructor(options: ClientOptions) {
    this.url = options.url;
    this.tokenProvider = options.tokenProvider;
    this.fetchImpl = options.fetch ?? ((url, init) => globalThis.fetch(url, init));
    this.profileId = options.profileId ?? PROFILE_ID;
    this.capabilities = options.capabilities;
    this.requires = [...new Set([...(options.requires ?? []), ...(options.capabilities ? HTTP_METHODS : [])])];
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.maxResponseBytes = options.maxResponseBytes ?? DEFAULT_MAX_RESPONSE_BYTES;
  }

  static async connect(options: ClientOptions): Promise<ConexClient> {
    return new ConexClient(options);
  }

  list(endpointId: string, input: SourceListRequest): Promise<SourceListResponse> {
    return this.call<SourceListResponse>("source/list", endpointId, input, true);
  }

  read(endpointId: string, input: SourceReadRequest): Promise<SourceReadResponse> {
    return this.call<SourceReadResponse>("source/read", endpointId, input, true);
  }

  search(endpointId: string, input: SourceSearchRequest): Promise<SourceSearchResponse> {
    return this.call<SourceSearchResponse>("source/search", endpointId, input, true);
  }

  /// Independent per-target calls; one failure never discards the others.
  async searchMany(targets: SearchManyTarget[]): Promise<SearchManyResult[]> {
    const results: SearchManyResult[] = [];
    for (const target of targets) {
      try {
        results.push({ endpointId: target.endpointId, result: await this.search(target.endpointId, target.input) });
      } catch (error) {
        results.push({ endpointId: target.endpointId, error: toConexError(error) });
      }
    }
    return results;
  }

  async listModules(spaceId: string): Promise<ModuleReference[]> {
    const result = await this.s1Request<{ modules: ModuleReference[] }>("module/list", { plane: "broker" }, { spaceId }, mintUlid(), "read_only");
    return result.modules;
  }

  invoke<I, O>(reference: ModuleReference, method: string, argument: I, options: InvokeOptions = {}): Invocation<O> {
    const context = targetContext(reference);
    const descriptor = this.capabilities?.contracts.find(contract => contract.contractId === reference.contractId)?.methods[method];
    if (!descriptor || !reference.methods.includes(method)) throw new ConexError("method not installed or exposed", { code: -32004, execution: "not_started", retry: "never" });
    const input = descriptor.input(argument);
    const requestId = mintUlid();
    let settled = false;
    let binding: S1Context | undefined;
    const result = this.s1Request<O>("module/invoke", context, { method, argument: input }, requestId, descriptor.execution, options.timeoutMs, pinned => { binding = pinned; }, value => descriptor.output(value)).finally(() => {
      settled = true;
      options.signal?.removeEventListener("abort", abort);
    });
    const cancel = async (): Promise<{ requested: boolean }> => {
      if (settled) return { requested: false };
      // Binding acquisition happens before dispatch; wait for it without reissuing invoke.
      await this.bindingPromise;
      if (settled || !binding) return { requested: false };
      return this.s1Request("module/cancel", binding, { requestId }, mintUlid(), "read_only", undefined, undefined, undefined, true);
    };
    const abort = () => { void cancel().catch(() => undefined); };
    options.signal?.addEventListener("abort", abort, { once: true });
    if (options.signal?.aborted) abort();
    return { requestId, result, cancel };
  }

  subscribe(): never {
    throw new ConexError("HTTP subscriptions are unsupported", { code: -32004, execution: "not_started", retry: "never" });
  }

  publishModule(): never {
    throw new ConexError("HTTP publication is unsupported", { code: -32004, execution: "not_started", retry: "never" });
  }

  private async s1Request<T>(method: string, context: S1Context, input: unknown, requestId: string, execution: ExecutionClass, timeoutMs?: number, pin?: (context: S1Context) => void, output?: (value: unknown) => unknown, pinned = false): Promise<T> {
    if (!this.capabilities) throw new ConexError("S1 not configured", { code: -32004 });
    const deadline = Date.now() + (timeoutMs ?? this.timeoutMs);
    if (!pinned) await this.ensureBinding(deadline);
    const limits = this.s1Limits;
    if (!limits) throw new ConexError("S1 binding not negotiated", { code: -32004 });
    const budget = Math.min(deadline - Date.now(), limits.timeoutMs);
    if (budget <= 0 || (timeoutMs !== undefined && (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > limits.timeoutMs))) throw new ConexError("invalid/expired request budget", { code: -32006, execution: "not_started", retry: "safe" });
    const control = method === "module/cancel";
    if (this.s1Inflight >= limits.maxInflight - (control ? 0 : 1)) throw new ConexError("HTTP inflight limit", { code: -32008, execution: "not_started", retry: "never" });
    const bound = { ...context, bindingId: pinned ? context.bindingId : this.binding!.id };
    const params = { context: bound, timeoutBudgetMs: budget, input };
    validateFrame({ kind: "request", id: requestId, method, params }, "client");
    const body = { jsonrpc: "2.0", id: requestId, method, params };
    if (new TextEncoder().encode(JSON.stringify(body)).byteLength > limits.maxFrameBytes) throw new ConexError("frame too large", { code: -32007, execution: "not_started", retry: "never" });
    pin?.(bound);
    this.s1Inflight++;
    const controller = new AbortController();
    const stop = () => controller.abort();
    this.abort.signal.addEventListener("abort", stop, { once: true });
    if (this.closed) stop();
    const timer = setTimeout(stop, deadline - Date.now());
    let rejectAbort!: (error: unknown) => void;
    const aborted = new Promise<never>((_resolve, reject) => { rejectAbort = reject; });
    const onAbort = () => rejectAbort(uncertain(execution, "HTTP outcome unknown", Date.now() >= deadline));
    controller.signal.addEventListener("abort", onAbort, { once: true });
    if (controller.signal.aborted) onAbort();
    try {
      const exchange = async () => {
        const response = await this.post(body, deadline, controller.signal);
        const text = await this.readCapped(response);
        if (new TextEncoder().encode(text).byteLength > limits.maxFrameBytes) throw new Error("response exceeds negotiated frame limit");
        const frame = decodeS1Frame(text);
        if (frame.id !== requestId || (frame.kind !== "success" && frame.kind !== "failure")) throw new Error("response correlation mismatch");
        if (frame.kind === "failure") {
          const error = frame.error as { code: number; message: string; data?: Record<string, unknown> };
          throw new ConexError(error.message, { ...error.data, code: error.code, status: response.status });
        }
        validateResult(method, frame.result);
        return (output ? output(frame.result) : frame.result) as T;
      };
      return await Promise.race([exchange(), aborted]);
    } catch (error) {
      if (error instanceof ConexError && error.execution) throw error;
      throw uncertain(execution, "HTTP outcome unknown", Date.now() >= deadline);
    } finally {
      clearTimeout(timer);
      this.abort.signal.removeEventListener("abort", stop);
      controller.signal.removeEventListener("abort", onAbort);
      this.s1Inflight--;
    }
  }

  close(): void {
    this.closed = true;
    this.abort.abort();
  }

  private async call<T>(method: string, endpointId: string, input: unknown, readOnly: boolean): Promise<T> {
    const deadline = Date.now() + this.timeoutMs;
    let attempt = 0;
    for (;;) {
      attempt += 1;
      try {
        await this.ensureBinding(deadline);
        const requestId = mintUlid();
        const response = await this.post(this.envelope(method, endpointId, input, requestId), deadline);
        const json = await this.parse(response);
        if (json && json.error) {
          throw errorFrom(json.error, response.status);
        }
        return json.result as T;
      } catch (error) {
        const conex = toConexError(error);
        const expired = conex.status === 401 || conex.code === -32001;
        if (readOnly && attempt < 2 && expired) {
          this.binding = undefined;
          continue;
        }
        throw conex;
      }
    }
  }

  private async ensureBinding(deadline: number): Promise<void> {
    if (this.bindingPromise) return this.bindingPromise;
    const promise = this.openBinding(deadline);
    this.bindingPromise = promise;
    try { await promise; } finally { if (this.bindingPromise === promise) this.bindingPromise = undefined; }
  }

  private async openBinding(deadline: number): Promise<void> {
    if (this.binding && Date.now() < this.binding.expiresAt - 1000) {
      return;
    }
    const requestId = mintUlid();
    const hello = {
      profileId: this.profileId,
      plane: "broker",
      provides: [],
      requires: this.requires,
    };
    const response = await this.post(this.envelope("conex/hello", "", hello, requestId), deadline);
    const json = await this.parse(response);
    if (json && json.error) {
      throw errorFrom(json.error, response.status);
    }
    const result = json.result as { bindingId?: string; expiresInMs?: number; provides?: string[]; limits?: unknown } | undefined;
    if (!result || typeof result.bindingId !== "string") {
      throw new ConexError("hello response is missing bindingId", { status: response.status });
    }
    if (this.capabilities) {
      if (!result.provides || this.requires.some(method => !result.provides!.includes(method))) throw new ConexError("S1 HTTP methods not negotiated", { code: -32004 });
      this.s1Limits = checkedLimits(result.limits);
    }
    this.binding = { id: result.bindingId, expiresAt: Date.now() + (result.expiresInMs ?? 60000) };
  }

  private envelope(method: string, endpointId: string, input: unknown, requestId: string): unknown {
    const context: Record<string, unknown> = { providerEndpointId: endpointId, plane: "broker" };
    if (this.binding && method !== "conex/hello") {
      context.bindingId = this.binding.id;
    }
    return {
      jsonrpc: "2.0",
      id: requestId,
      method,
      params: { context, timeoutBudgetMs: this.timeoutMs, input },
    };
  }

  private async post(body: unknown, deadline: number, signal?: AbortSignal): Promise<Response> {
    if (this.closed) {
      throw new ConexError("client is closed", { code: -32011 });
    }
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      throw new ConexError("deadline exceeded before request", { code: -32006 });
    }
    const controller = new AbortController();
    const onAbort = () => controller.abort();
    signal?.addEventListener("abort", onAbort, { once: true });
    this.abort.signal.addEventListener("abort", onAbort, { once: true });
    if (this.abort.signal.aborted || signal?.aborted) {
      onAbort();
    }
    const timer = setTimeout(() => controller.abort(), remaining);
    const aborted = new Promise<never>((_resolve, reject) => {
      controller.signal.addEventListener("abort", () => reject(new ConexError("request aborted", { code: -32006 })), { once: true });
      if (controller.signal.aborted) reject(new ConexError("request aborted", { code: -32006 }));
    });
    try {
      const token = await Promise.race([this.tokenProvider(), aborted]);
      controller.signal.throwIfAborted();
      const init: RequestInit = {
        method: "POST",
        headers: { "content-type": "application/json", authorization: "Bearer " + token },
        body: JSON.stringify(body),
        signal: controller.signal,
        redirect: "error",
      };
      return await Promise.race([this.fetchImpl(this.url, init), aborted]);
    } catch (error) {
      if (this.closed) {
        throw new ConexError("client is closed", { code: -32011 });
      }
      throw new ConexError("transport error", { details: error instanceof Error ? error.name : String(error) });
    } finally {
      clearTimeout(timer);
      this.abort.signal.removeEventListener("abort", onAbort);
      signal?.removeEventListener("abort", onAbort);
    }
  }

  private async parse(response: Response): Promise<any> {
    if (response.status === 401) {
      throw new ConexError("unauthorized", { status: 401, code: -32001 });
    }
    const text = await this.readCapped(response);
    if (!text && response.status >= 400) {
      throw new ConexError("http status " + response.status, { status: response.status });
    }
    try {
      return JSON.parse(text);
    } catch {
      throw new ConexError("invalid JSON response", { status: response.status });
    }
  }

  private async readCapped(response: Response): Promise<string> {
    const body = response.body;
    if (!body) {
      const text = await response.text();
      if (new TextEncoder().encode(text).length > this.maxResponseBytes) {
        throw new ConexError("response exceeds maxResponseBytes", { code: -32007 });
      }
      return text;
    }
    const reader = body.getReader();
    const chunks: Uint8Array[] = [];
    let total = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      total += value.byteLength;
      if (total > this.maxResponseBytes) {
        await reader.cancel().catch(() => undefined);
        throw new ConexError("response exceeds maxResponseBytes", { code: -32007 });
      }
      chunks.push(value);
    }
    const merged = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      merged.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return new TextDecoder().decode(merged);
  }
}

function errorFrom(error: any, status: number): ConexError {
  const data = (error && error.data) || {};
  return new ConexError(error?.message ?? "rpc error", {
    code: typeof error?.code === "number" ? error.code : undefined,
    diagnosticId: data.diagnosticId,
    execution: data.execution,
    retry: data.retry,
    details: data.details,
    status,
  });
}
