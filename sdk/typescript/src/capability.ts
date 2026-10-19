import { ConexError, toConexError } from "./errors";
import { ErrorCode } from "./generated/conex/common";
import type { Limits } from "./generated/conex/common";
import type { ModuleInfo, ModuleRegisterInput, ModuleRegisterResult } from "./generated/conex/capability";
import type { Sample, StateObserver, StateOpening, StateSubscription } from "./state";
import {
  decodeS1Frame, validateFrame, validateResult, validateLimits, isS1Method,
  validateSample, parseStateEndError, stateUpdateInput, stateEndInput,
  METHOD_MODULE_REGISTER, METHOD_MODULE_UNREGISTER, METHOD_MODULE_LIST,
  METHOD_MODULE_INVOKE, METHOD_MODULE_CANCEL, METHOD_STATE_SUBSCRIBE,
  METHOD_STATE_UNSUBSCRIBE, METHOD_STATE_UPDATE, METHOD_STATE_END,
} from "./s1-wire";
import type { S1Context, S1Frame } from "./s1-wire";

export type ExecutionClass = "read_only" | "non_replayable";
export interface MethodContract<I = unknown, O = unknown> {
  execution: ExecutionClass;
  input(value: unknown): I;
  output(value: unknown): O;
}
export interface StateContract<I = unknown, T = unknown> {
  input(value: unknown): I;
  value(value: unknown): T;
}
export interface CapabilityContract {
  contractId: string;
  methods: Record<string, MethodContract>;
  states: Record<string, StateContract>;
}
export type ModuleReference = Required<ModuleInfo>;
export interface VerifiedContext extends S1Context {
  principalId: string; tenantId: string; spaceId: string;
  providerEndpointId: string; registrationId: string;
}
export interface ModuleProvider {
  module: Required<ModuleRegisterInput>;
  handlers: Record<string, (argument: unknown, context: VerifiedContext, signal: AbortSignal) => unknown | Promise<unknown>>;
  states: Record<string, (argument: unknown, observer: StateObserver<unknown>, context: VerifiedContext, signal: AbortSignal) => StateOpening<unknown> | Promise<StateOpening<unknown>>>;
  /** Local exposure ceiling; return false to refuse the verified caller. */
  authorize(context: VerifiedContext, member: string, argument: unknown): boolean;
}
export interface Invocation<T> {
  readonly requestId: string;
  readonly result: Promise<T>;
  cancel(): Promise<{ requested: boolean }>;
}
export interface InvokeOptions { timeoutMs?: number; signal?: AbortSignal }
export interface CapabilityOptions {
  contracts: readonly CapabilityContract[];
  /** Installed local exposure, never supplied by a remote consumer. */
  providers?: readonly ModuleProvider[];
}
export const CONSUMER_METHODS = [METHOD_MODULE_LIST, METHOD_MODULE_INVOKE, METHOD_MODULE_CANCEL, METHOD_STATE_SUBSCRIBE, METHOD_STATE_UNSUBSCRIBE];
export const PROVIDER_METHODS = [METHOD_MODULE_INVOKE, METHOD_MODULE_CANCEL, METHOD_STATE_SUBSCRIBE, METHOD_STATE_UNSUBSCRIBE];
export const HTTP_METHODS = [METHOD_MODULE_LIST, METHOD_MODULE_INVOKE, METHOD_MODULE_CANCEL];

export function checkedLimits(value: unknown): Required<Limits> {
  if (!value || typeof value !== "object") throw new ConexError("missing S1 limits", { code: -32004 });
  const limits = value as Record<string, unknown>;
  validateLimits(limits);
  return limits as Required<Limits>;
}
export function uncertain(execution: ExecutionClass | undefined, message: string, timeout = false): ConexError {
  return new ConexError(message, { code: execution === "read_only" ? (timeout ? -32006 : -32005) : -32012, execution: "unknown", retry: execution === "read_only" ? "safe" : "never" });
}
export function targetContext(reference: ModuleReference): S1Context {
  for (const key of Object.keys(reference)) {
    if (!["spaceId", "providerEndpointId", "registrationId", "integrationId", "contractId", "methods", "states"].includes(key)) throw new ConexError("untrusted module reference field: " + key, { code: -32602, execution: "not_started", retry: "never" });
  }
  return { plane: "broker", spaceId: reference.spaceId, providerEndpointId: reference.providerEndpointId, registrationId: reference.registrationId };
}
interface Deferred<T> { promise: Promise<T>; resolve(value: T): void; reject(error: unknown): void }
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
interface Pending {
  method: string; execution?: ExecutionClass; sent: boolean;
  resolve(value: unknown): void; reject(error: unknown): void;
  timer: ReturnType<typeof setTimeout>;
}
interface ConsumerState {
  reference: ModuleReference; descriptor: StateContract; context: S1Context;
  snapshot: Deferred<Sample<unknown>>;
  ended: Deferred<ConexError>;
  update(sample: Sample<unknown>): void;
  active: boolean; revision: bigint; seq: bigint;
}
interface ProducerState {
  context: VerifiedContext; controller: AbortController; dispose?: () => void;
  opening: boolean; queue: Sample<unknown>[]; bytes: number; revision: bigint; seq: bigint;
  stopOpening?: (error: ConexError) => void;
  descriptor: StateContract; ended: boolean;
}
interface ProviderCall { context: VerifiedContext; controller: AbortController; timer: ReturnType<typeof setTimeout>; settled: boolean }
interface ForwardInput {
  method: string; state: string; argument: unknown; requestId: string; subscriptionId: string;
}

/** Per-link S1 runtime. Reconnection creates no calls, publications or subscriptions. */
export class CapabilitySession {
  readonly requires: string[];
  readonly provides: string[];
  private limits?: Required<Limits>;
  private readonly contracts = new Map<string, CapabilityContract>();
  private readonly registrations = new Map<string, { reference: ModuleReference; provider: ModuleProvider }>();
  private readonly pending = new Map<string, Pending>();
  private readonly consumers = new Map<string, ConsumerState>();
  private readonly producers = new Map<string, ProducerState>();
  private readonly calls = new Map<string, ProviderCall>();
  private openingBytes = 0;
  private readonly publishing = new Set<string>();
  get queuedBytes(): number { return this.openingBytes; }
  constructor(readonly options: CapabilityOptions, private readonly mint: () => string, private readonly send: (text: string, control: boolean) => void, private readonly closeLink: () => void, private readonly bufferedBytes: () => number = () => 0) {
    for (const contract of options.contracts) {
      if (this.contracts.has(contract.contractId)) throw new Error("duplicate contract");
      for (const descriptor of Object.values(contract.methods)) if (!["read_only", "non_replayable"].includes(descriptor.execution)) throw new Error("unsupported execution class");
      this.contracts.set(contract.contractId, contract);
    }
    this.provides = options.providers?.length ? [...PROVIDER_METHODS] : [];
    this.requires = [...CONSUMER_METHODS, ...(this.provides.length ? [METHOD_MODULE_REGISTER, METHOD_MODULE_UNREGISTER] : [])];
  }
  ready(provides: string[], limits: unknown): void {
    if (this.requires.some(method => !provides.includes(method))) throw new ConexError("S1 methods not negotiated", { code: -32004 });
    this.limits = checkedLimits(limits);
  }
  disconnected(): void {
    this.limits = undefined;
    for (const [id, pending] of this.pending) {
      clearTimeout(pending.timer);
      this.pending.delete(id);
      pending.reject(uncertain(pending.execution, "connection lost"));
    }
    for (const id of [...this.consumers.keys()]) this.endConsumer(id, new ConexError("connection lost; retained state is stale", { code: -32005, execution: "completed", retry: "safe" }));
    for (const [id, record] of this.producers) { record.stopOpening = undefined; this.disposeProducer(id); }
    for (const call of this.calls.values()) { clearTimeout(call.timer); call.controller.abort(); }
    this.calls.clear();
    this.registrations.clear();
    this.publishing.clear();
  }
  async publish(module: Required<ModuleRegisterInput>): Promise<ModuleReference> {
    const provider = this.options.providers?.find(provider => Object.keys(module).length === 4 && Object.entries(provider.module).every(([key, value]) => module[key as keyof typeof module] === value));
    if (!provider) throw new ConexError("module exceeds local exposure", { code: -32002 });
    const contract = this.contracts.get(module.contractId);
    if (!contract || Object.keys(contract.methods).some(name => !provider.handlers[name]) || Object.keys(contract.states).some(name => !provider.states[name])) throw new ConexError("contract handlers not installed", { code: -32004 });
    const key = JSON.stringify([module.spaceId, module.providerEndpointId]);
    if (!this.limits || this.registrations.size + this.publishing.size >= this.limits.maxModules) throw new ConexError("module limit", { code: -32008 });
    if (this.publishing.has(key) || [...this.registrations.values()].some(entry => entry.reference.spaceId === module.spaceId && entry.reference.providerEndpointId === module.providerEndpointId)) throw new ConexError("module already published", { code: -32014 });
    this.publishing.add(key);
    try {
      const result = await this.request<Required<ModuleRegisterResult>>(METHOD_MODULE_REGISTER, { plane: "broker" }, module).result;
      for (const field of Object.keys(module) as (keyof typeof module)[]) if (result[field] !== module[field]) throw new ConexError("registration target mismatch", { code: -32017 });
      const reference = { ...result, methods: Object.keys(contract.methods), states: Object.keys(contract.states) };
      this.registrations.set(result.registrationId, { reference, provider });
      return reference;
    } finally { this.publishing.delete(key); }
  }
  async unpublish(reference: ModuleReference): Promise<{ removed: boolean }> {
    targetContext(reference);
    const result = await this.request<{ removed: boolean }>(METHOD_MODULE_UNREGISTER, { plane: "broker" }, { providerEndpointId: reference.providerEndpointId, registrationId: reference.registrationId }).result;
    this.registrations.delete(reference.registrationId);
    for (const [id, state] of this.producers) if (state.context.registrationId === reference.registrationId) this.endProducer(id, new ConexError("module unpublished", { code: -32005 }));
    for (const call of this.calls.values()) if (call.context.registrationId === reference.registrationId) call.controller.abort();
    return result;
  }
  async listModules(spaceId: string): Promise<ModuleReference[]> {
    const result = await this.request<{ modules: ModuleReference[] }>(METHOD_MODULE_LIST, { plane: "broker" }, { spaceId }).result;
    return result.modules;
  }
  invoke<I, O>(reference: ModuleReference, method: string, argument: I, options: InvokeOptions = {}): Invocation<O> {
    const context = targetContext(reference);
    const descriptor = this.contracts.get(reference.contractId)?.methods[method];
    if (!descriptor || !reference.methods.includes(method)) throw new ConexError("method not installed or exposed", { code: -32004, execution: "not_started", retry: "never" });
    const input = descriptor.input(argument);
    const call = this.request<O>(METHOD_MODULE_INVOKE, context, { method, argument: input }, descriptor.execution, options.timeoutMs, value => descriptor.output(value));
    let settled = false;
    const cancel = () => settled ? Promise.resolve({ requested: false }) : this.request<{ requested: boolean }>(METHOD_MODULE_CANCEL, context, { requestId: call.requestId }).result;
    const abort = () => { void cancel().catch(() => undefined); };
    options.signal?.addEventListener("abort", abort, { once: true });
    if (options.signal?.aborted) abort();
    const result = call.result.finally(() => { settled = true; options.signal?.removeEventListener("abort", abort); });
    return { requestId: call.requestId, result, cancel };
  }
  subscribe<I, T>(reference: ModuleReference, state: string, argument: I, update: (sample: Sample<T>) => void, options: InvokeOptions = {}): StateSubscription<T> {
    const context = targetContext(reference);
    const descriptor = this.contracts.get(reference.contractId)?.states[state];
    if (!descriptor || !reference.states.includes(state)) throw new ConexError("state not installed or exposed", { code: -32004 });
    if (!this.limits || this.consumers.size >= this.limits.maxSubscriptions) throw new ConexError("subscription limit", { code: -32008 });
    const argumentValue = descriptor.input(argument);
    const subscriptionId = this.mint();
    const record: ConsumerState = { reference: structuredClone(reference), descriptor, context, snapshot: deferred(), ended: deferred(), update: sample => update(sample as Sample<T>), active: false, seq: 0n, revision: 0n };
    this.consumers.set(subscriptionId, record);
    const unsubscribe = async () => {
      const existed = this.consumers.has(subscriptionId);
      this.endConsumer(subscriptionId, new ConexError("unsubscribed", { code: -32011, execution: "completed", retry: "never" }));
      if (!existed) return { removed: false };
      return this.request<{ removed: boolean }>(METHOD_STATE_UNSUBSCRIBE, context, { subscriptionId }).result;
    };
    const abort = () => { void unsubscribe().catch(() => undefined); };
    options.signal?.addEventListener("abort", abort, { once: true });
    void record.ended.promise.then(() => options.signal?.removeEventListener("abort", abort));
    const call = this.request(METHOD_STATE_SUBSCRIBE, context, { subscriptionId, state, argument: argumentValue }, "read_only", options.timeoutMs, value => {
      if (!this.consumers.has(subscriptionId)) return value;
      const result = value as { subscriptionId: string; registrationId: string; snapshot: Sample<T> };
      if (result.subscriptionId !== subscriptionId || result.registrationId !== reference.registrationId) throw new Error("snapshot origin mismatch");
      descriptor.value(result.snapshot.value);
      record.revision = BigInt(result.snapshot.revision);
      record.active = true;
      record.snapshot.resolve(result.snapshot);
      return value;
    });
    void call.result.catch(error => {
      if (this.consumers.has(subscriptionId)) {
        this.endConsumer(subscriptionId, toConexError(error));
        void this.request(METHOD_STATE_UNSUBSCRIBE, context, { subscriptionId }).result.catch(() => undefined);
      }
    });
    if (options.signal?.aborted) abort();
    return { subscriptionId, snapshot: record.snapshot.promise as Promise<Sample<T>>, ended: record.ended.promise, unsubscribe };
  }
  private request<T>(method: string, context: S1Context, input: unknown, execution: ExecutionClass | undefined = [METHOD_MODULE_LIST, METHOD_MODULE_CANCEL, METHOD_STATE_SUBSCRIBE, METHOD_STATE_UNSUBSCRIBE].includes(method) ? "read_only" : undefined, timeoutMs?: number, output?: (value: unknown) => unknown): { requestId: string; result: Promise<T> } {
    const requestId = this.mint();
    const result = deferred<T>();
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const limits = this.limits;
      if (!limits) throw new ConexError("S1 link not ready", { code: -32005, execution: "not_started", retry: "never" });
      const control = method === METHOD_MODULE_CANCEL || method === METHOD_STATE_UNSUBSCRIBE;
      if (this.pending.size >= limits.maxInflight - (control ? 0 : 1)) throw new ConexError("inflight limit", { code: -32008, execution: "not_started", retry: "never" });
      const budget = timeoutMs ?? limits.timeoutMs;
      if (!Number.isSafeInteger(budget) || budget <= 0 || budget > limits.timeoutMs) throw new ConexError("invalid request budget", { code: -32602 });
      const params = { context, input, timeoutBudgetMs: budget };
      validateFrame({ kind: "request", id: requestId, method, params }, "client");
      timer = setTimeout(() => {
        const pending = this.pending.get(requestId);
        if (!pending) return;
        this.pending.delete(requestId);
        pending.reject(uncertain(execution, "request timed out", true));
      }, budget);
      const pending: Pending = { method, execution, sent: false, timer, resolve: value => {
        try { validateResult(method, value); result.resolve((output ? output(value) : value) as T); }
        catch { result.reject(uncertain(execution, "untrusted result")); }
      }, reject: result.reject };
      this.pending.set(requestId, pending);
      const text = JSON.stringify({ jsonrpc: "2.0", id: requestId, method, params });
      if (new TextEncoder().encode(text).byteLength > limits.maxFrameBytes) throw new ConexError("frame too large", { code: -32007, execution: "not_started", retry: "never" });
      pending.sent = true;
      this.send(text, control);
    } catch (error) {
      const pending = this.pending.get(requestId);
      clearTimeout(timer);
      this.pending.delete(requestId);
      result.reject(pending?.sent ? uncertain(execution, "send outcome unknown") : error);
    }
    return { requestId, result: result.promise };
  }
  /** Returns false only for an unrelated legacy response/notification. */
  receive(text: string): boolean {
    // Legacy application notifications have their own params shape.
    let route: { id?: string; method?: string };
    try { route = JSON.parse(text) as typeof route; }
    catch { this.disconnected(); this.closeLink(); return true; }
    if (route.method ? !isS1Method(route.method) : !this.pending.has(route.id ?? "")) return false;
    if (!this.limits || new TextEncoder().encode(text).byteLength > this.limits.maxFrameBytes) {
      this.disconnected(); this.closeLink(); return true;
    }
    let frame: S1Frame;
    try { frame = decodeS1Frame(text); }
    catch (error) { this.disconnected(); this.closeLink(); return true; }
    if (frame.kind === "success" || frame.kind === "failure") {
      const pending = this.pending.get(frame.id ?? "");
      if (!pending) return false;
      this.pending.delete(frame.id!);
      clearTimeout(pending.timer);
      if (frame.kind === "failure") {
        const error = frame.error as { code: number; message: string; data?: Record<string, unknown> };
        pending.reject(new ConexError(error.message, { ...error.data, code: error.code }));
      } else pending.resolve(frame.result);
      return true;
    }
    if (!frame.method || ![...PROVIDER_METHODS, METHOD_STATE_UPDATE, METHOD_STATE_END].includes(frame.method)) return false;
    try {
      if (!this.limits || new TextEncoder().encode(text).byteLength > this.limits.maxFrameBytes) throw new Error("frame exceeds negotiated limit");
      validateFrame(frame, "host");
      if (frame.kind === "notification") this.notification(frame);
      else void this.forwarded(frame);
    } catch (error) { this.disconnected(); this.closeLink(); }
    return true;
  }
  private notification(frame: S1Frame): void {
    const { context, input } = frame.params!;
    const value = input as { subscriptionId: string; seq: string; sample: Sample<unknown>; error: unknown };
    const record = this.consumers.get(value.subscriptionId);
    if (!record) return;
    try {
      if (context.spaceId !== record.context.spaceId || context.providerEndpointId !== record.context.providerEndpointId || context.registrationId !== record.context.registrationId) throw new Error("state origin mismatch");
      if (frame.method === METHOD_STATE_END) {
        const error = parseStateEndError(value.error);
        this.endConsumer(value.subscriptionId, new ConexError(error.message ?? "state ended", { ...error, code: error.code }));
        return;
      }
      if (!record.active || BigInt(value.seq) !== record.seq + 1n || BigInt(value.sample.revision) <= record.revision) throw new Error("state sequence lost");
      record.descriptor.value(value.sample.value);
      record.seq = BigInt(value.seq);
      record.revision = BigInt(value.sample.revision);
      record.update(value.sample);
    } catch {
      this.endConsumer(value.subscriptionId, new ConexError("state sequence/origin invalid; retained state is stale", { code: -32017, execution: "completed", retry: "safe" }));
      void this.request(METHOD_STATE_UNSUBSCRIBE, record.context, { subscriptionId: value.subscriptionId }).result.catch(() => undefined);
    }
  }
  private endConsumer(id: string, error: ConexError): void {
    const record = this.consumers.get(id);
    if (!record) return;
    this.consumers.delete(id);
    if (!record.active) record.snapshot.reject(error);
    record.ended.resolve(error);
  }
  private verified(context: S1Context): { context: VerifiedContext; provider: ModuleProvider; contract: CapabilityContract } {
    if (!context.principalId || !context.tenantId) throw new ConexError("missing verified identity", { code: -32010 });
    const entry = this.registrations.get(context.registrationId!);
    if (!entry || entry.reference.spaceId !== context.spaceId || entry.reference.providerEndpointId !== context.providerEndpointId) throw new ConexError("stale registration", { code: -32013 });
    return { context: Object.freeze({ ...context }) as VerifiedContext, provider: entry.provider, contract: this.contracts.get(entry.reference.contractId)! };
  }
  private sameContext(a: S1Context, b: S1Context): boolean {
    return ["principalId", "tenantId", "spaceId", "providerEndpointId", "registrationId"].every(key => a[key] === b[key]);
  }
  private async forwarded(frame: S1Frame): Promise<void> {
    const id = frame.id!;
    const method = frame.method!;
    const params = frame.params!;
    const input = params.input as ForwardInput;
    let execution: ExecutionClass | undefined;
    try {
      const control = method === METHOD_MODULE_CANCEL || method === METHOD_STATE_UNSUBSCRIBE;
      if (control) {
        if (this.calls.size >= this.limits!.maxInflight) throw new ConexError("control limit", { code: -32008 });
        if (method === METHOD_MODULE_CANCEL) {
          const call = this.calls.get(input.requestId);
          const requested = !!call && this.sameContext(call.context, params.context);
          if (requested) call!.controller.abort();
          this.reply(id, { requested });
        } else {
          const producer = this.producers.get(input.subscriptionId);
          const removed = !!producer && this.sameContext(producer.context, params.context);
          if (removed) this.disposeProducer(input.subscriptionId);
          this.reply(id, { removed });
        }
        return;
      }
      const { context, provider, contract } = this.verified(params.context);
      if (this.calls.size >= this.limits!.maxInflight - 1) throw new ConexError("provider inflight limit", { code: -32008 });
      if (this.calls.has(id)) throw new ConexError("duplicate forwarded id", { code: -32014 });
      if (!provider.authorize(context, input.method ?? input.state, input.argument)) throw new ConexError("local exposure denied", { code: -32002 });
      if (!Number.isSafeInteger(params.timeoutBudgetMs) || params.timeoutBudgetMs! <= 0 || params.timeoutBudgetMs! > this.limits!.timeoutMs) throw new ConexError("invalid forwarded budget", { code: -32602 });
      if (method === METHOD_STATE_SUBSCRIBE) {
        await this.openProducer(id, input, context, provider, contract, params.timeoutBudgetMs!);
        return;
      }
      const descriptor = contract.methods[input.method];
      if (!descriptor || !provider.handlers[input.method]) throw new ConexError("unknown contract method", { code: -32004 });
      const argument = descriptor.input(input.argument);
      execution = descriptor.execution;
      const controller = new AbortController();
      const call: ProviderCall = { context, controller, settled: false, timer: setTimeout(() => {
        if (this.calls.get(id) !== call || call.settled) return;
        call.settled = true;
        controller.abort();
        this.reply(id, undefined, uncertain(execution, "provider deadline exceeded", true));
      }, params.timeoutBudgetMs) };
      this.calls.set(id, call);
      try {
        const result = await provider.handlers[input.method](argument, context, controller.signal);
        if (this.calls.get(id) !== call || call.settled) return;
        let output: unknown;
        try { output = descriptor.output(result); } catch { throw uncertain(execution, "handler output invalid"); }
        this.reply(id, output);
      } catch (error) {
        if (this.calls.get(id) === call && !call.settled) {
          const failure = error instanceof ConexError
            ? new ConexError(error.message, { ...error, execution: error.execution ?? "completed", retry: error.retry ?? "never" })
            : uncertain(execution, "handler outcome unknown");
          this.reply(id, undefined, failure);
        }
      } finally {
        clearTimeout(call.timer);
        if (this.calls.get(id) === call) this.calls.delete(id);
      }
    } catch (error) { this.reply(id, undefined, toConexError(error)); }
  }
  private async openProducer(id: string, input: ForwardInput, context: VerifiedContext, provider: ModuleProvider, contract: CapabilityContract, budget: number): Promise<void> {
    const subscriptionId = input.subscriptionId as string;
    const descriptor = contract.states[input.state];
    if (!descriptor || !provider.states[input.state]) throw new ConexError("unknown state", { code: -32004 });
    const argument = descriptor.input(input.argument);
    if (this.producers.has(subscriptionId)) throw new ConexError("duplicate subscription", { code: -32014 });
    if (this.producers.size >= this.limits!.maxSubscriptions) throw new ConexError("provider subscription limit", { code: -32008 });
    const controller = new AbortController();
    const record: ProducerState = { context, controller, opening: true, queue: [], bytes: 0, revision: 0n, seq: 0n, descriptor, ended: false };
    this.producers.set(subscriptionId, record);
    let replied = false;
    const timer = setTimeout(() => {
      if (replied) return;
      replied = true;
      this.disposeProducer(subscriptionId);
      this.reply(id, undefined, new ConexError("state opening timeout", { code: -32006, execution: "not_started", retry: "safe" }));
    }, budget);
    // Count opening handlers too, while control requests continue on the receive loop.
    const call: ProviderCall = { context, controller, timer, settled: false };
    this.calls.set(id, call);
    record.stopOpening = error => {
      if (replied) return;
      replied = true;
      clearTimeout(timer);
      this.reply(id, undefined, error);
    };
    try {
      const opening = await provider.states[input.state](argument, {
        update: sample => this.produce(subscriptionId, sample),
        end: error => this.endProducer(subscriptionId, toConexError(error)),
      }, context, controller.signal);
      record.dispose = opening.dispose;
      if (record.ended || this.producers.get(subscriptionId) !== record || replied) { opening.dispose(); return; }
      validateSample("snapshot", opening.snapshot);
      descriptor.value(opening.snapshot.value);
      record.revision = BigInt(opening.snapshot.revision);
      for (const sample of record.queue) {
        if (BigInt(sample.revision) <= record.revision) throw new Error("non-atomic state opening");
        record.revision = BigInt(sample.revision);
      }
      record.revision = BigInt(opening.snapshot.revision);
      this.reply(id, { subscriptionId, registrationId: context.registrationId, snapshot: opening.snapshot });
      replied = true;
      record.opening = false;
      const queue = record.queue;
      record.queue = [];
      this.openingBytes -= record.bytes;
      record.bytes = 0;
      for (const sample of queue) this.produce(subscriptionId, sample);
    } catch (error) {
      record.stopOpening?.(toConexError(error));
      this.disposeProducer(subscriptionId);
    } finally {
      clearTimeout(timer);
      if (this.calls.get(id) === call) this.calls.delete(id);
      if (!replied) this.reply(id, undefined, new ConexError("subscription stopped while opening", { code: -32011, execution: "not_started", retry: "never" }));
    }
  }
  private produce(id: string, sample: Sample<unknown>): void {
    const record = this.producers.get(id);
    if (!record || record.ended) return;
    try {
      validateSample("sample", sample);
      record.descriptor.value(sample.value);
      if (record.opening) {
        const bytes = new TextEncoder().encode(JSON.stringify(sample)).byteLength;
        if (!this.limits || bytes > this.limits.maxFrameBytes || this.openingBytes + this.bufferedBytes() + bytes > this.limits.maxQueuedBytes - this.limits.maxFrameBytes) throw new ConexError("opening state queue full", { code: -32015 });
        record.queue.push(structuredClone(sample));
        record.bytes += bytes;
        this.openingBytes += bytes;
        return;
      }
      if (BigInt(sample.revision) <= record.revision || record.seq === 18446744073709551615n) throw new ConexError("state revision/sequence exhausted", { code: -32017 });
      const input = stateUpdateInput(id, (++record.seq).toString(), sample);
      const { plane, spaceId, providerEndpointId, registrationId } = record.context;
      this.sendFrame({ jsonrpc: "2.0", method: METHOD_STATE_UPDATE, params: { context: { plane, spaceId, providerEndpointId, registrationId }, input } }, false);
      record.revision = BigInt(sample.revision);
    } catch (error) { this.endProducer(id, toConexError(error)); }
  }
  private disposeProducer(id: string): void {
    const record = this.producers.get(id);
    if (!record) return;
    this.producers.delete(id);
    record.ended = true;
    this.openingBytes -= record.bytes;
    record.queue = [];
    record.bytes = 0;
    record.stopOpening?.(new ConexError("subscription stopped while opening", { code: -32011, execution: "not_started", retry: "never" }));
    record.controller.abort();
    try { record.dispose?.(); } catch { /* Cleanup cannot prevent link teardown. */ }
  }
  private endProducer(id: string, error: ConexError): void {
    const record = this.producers.get(id);
    if (!record) return;
    if (record.opening) {
      record.stopOpening?.(error);
      this.disposeProducer(id);
      return;
    }
    this.disposeProducer(id);
    const code = error.code === -32017 ? ErrorCode.ERROR_CODE_SESSION_LOST : error.code === -32002 ? ErrorCode.ERROR_CODE_FORBIDDEN : error.code === -32005 ? ErrorCode.ERROR_CODE_UNAVAILABLE : ErrorCode.ERROR_CODE_SLOW_CONSUMER;
    try {
      const { plane, spaceId, providerEndpointId, registrationId } = record.context;
      this.sendFrame({ jsonrpc: "2.0", method: METHOD_STATE_END, params: { context: { plane, spaceId, providerEndpointId, registrationId }, input: stateEndInput(id, { code, message: error.message, execution: "completed", retry: code === ErrorCode.ERROR_CODE_FORBIDDEN ? "never" : "safe" }) } }, true);
    } catch { this.disconnected(); this.closeLink(); }
  }
  private reply(id: string, result?: unknown, error?: ConexError): void {
    try {
      this.sendFrame(error ? { jsonrpc: "2.0", id, error: { code: error.code ?? -32603, message: error.message, data: { diagnosticId: error.diagnosticId, execution: error.execution ?? "not_started", retry: error.retry ?? "never", details: error.details } } } : { jsonrpc: "2.0", id, result }, true);
    } catch { this.disconnected(); this.closeLink(); }
  }
  private sendFrame(frame: unknown, control: boolean): void {
    if (!this.limits) throw new ConexError("S1 disconnected", { code: -32005 });
    const text = JSON.stringify(frame);
    if (new TextEncoder().encode(text).byteLength > this.limits.maxFrameBytes) throw new ConexError("frame too large", { code: -32007 });
    this.send(text, control);
  }
}
