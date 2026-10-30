//! S2 plugin runtime: plugin classes, instances, local DI and explicit remote
//! bindings. Behavior authority: docs/contracts/s2-plugins.md as amended by
//! docs/contracts/private-runtime.md §1 and docs/plans/2026-10-29-conex-mvp-p2.md
//! §5 (P2.4): business state, publication state and link state are independent.
//! A lost link never destroys business; only an explicit `stop()` closes owned
//! local effects. Local modules start when the hub is absent; reconnection
//! republishes existing modules with fresh registrationIds without
//! reconstructing them or replaying mutations; remote refs stay pinned.

import { ConexError } from "./errors";
import type {
  CapabilityContract, Invocation, InvokeOptions, ModuleProvider, ModuleReference, VerifiedContext,
} from "./capability";
import type { Sample, StateSubscription } from "./state";
import { ConexWsClient } from "./ws-client";
import type { ConexWsClientOptions, ConexWsState } from "./ws-client";

/**
 * Composition failures are local typed values (TS discriminated union); they
 * never enter wire frames and never reuse wire error codes.
 */
export type PluginFailure =
  | { kind: "duplicate_module"; module: string }
  | { kind: "missing_dependency"; module: string; dependency: string }
  | { kind: "circular_dependency"; cycle: string[] }
  | { kind: "start_failed"; module: string; cause: unknown };

export class PluginCompositeError extends Error {
  readonly kind: PluginFailure["kind"];
  constructor(readonly failure: PluginFailure) {
    super(failureMessage(failure));
    this.name = "PluginCompositeError";
    this.kind = failure.kind;
  }
}

function failureMessage(failure: PluginFailure): string {
  switch (failure.kind) {
    case "duplicate_module": return `duplicate module name "${failure.module}"`;
    case "missing_dependency": return `module "${failure.module}" depends on missing "${failure.dependency}"`;
    case "circular_dependency": return `circular local dependency ${failure.cycle.join(" -> ")}`;
    case "start_failed": return `module "${failure.module}" failed to start`;
  }
}

/** Consumer handle pinned to one ModuleRef: plain S1 invoke/subscribe semantics. */
export interface RemoteBinding {
  readonly reference: ModuleReference;
  invoke<I, O>(method: string, argument: I, options?: InvokeOptions): Invocation<O>;
  subscribe<I, T>(state: string, argument: I, update: (sample: Sample<T>) => void, options?: InvokeOptions): StateSubscription<T>;
}

export interface ModuleDeps {
  /** In-process handle returned by the upstream module's start(); no envelopes. */
  local<H = unknown>(name: string): H;
  /** Consumer bound to the ModuleRef this dependency name was configured with. */
  remote(name: string): RemoteBinding;
}

export interface ModuleContext { readonly deps: ModuleDeps }

export interface ModuleRegistration { spaceId: string; providerEndpointId: string; integrationId: string }

export interface ModuleDefinition<H = unknown> {
  name: string;
  contract: CapabilityContract;
  registration: ModuleRegistration;
  /** Local in-instance module names this module starts after. */
  depsLocal?: readonly string[];
  /** Dependency names that must have a remote binding in the graph. */
  depsRemote?: readonly string[];
  /** Local exposure ceiling; defaults to accepting every verified caller. */
  authorize?(context: VerifiedContext, member: string, argument: unknown): boolean;
  handlers: ModuleProvider["handlers"];
  states: ModuleProvider["states"];
  /** Returns the service handle local dependents receive via deps.local(). */
  start(ctx: ModuleContext): H | Promise<H>;
  /**
   * Explicit reverse cleanup for owned local effects (in-process observers,
   * timers, spawned work). Runs exactly once per started module, in reverse
   * Kahn topology, on `stop()` and on start-failure rollback; never on a link
   * disconnect. Errors and deadline expiry reject stop and leave state failed.
   */
  stop?(ctx: ModuleContext, handle: H): void | Promise<void>;
}

export interface PluginGraph {
  modules: readonly ModuleDefinition[];
  /** Explicit remote bindings: dependency name -> fixed registration reference. */
  remotes?: Readonly<Record<string, ModuleReference>>;
}

export interface Plugin<I = unknown> {
  readonly id: string;
  /** Factory: config -> module graph declaration. Runs before any network I/O. */
  graph(config: I): PluginGraph;
}

export function definePlugin<I = unknown>(plugin:
  | { id: string; graph(config: I): PluginGraph }
  | { id: string; modules: readonly ModuleDefinition[] }
): Plugin<I> {
  if ("graph" in plugin) return { id: plugin.id, graph: config => plugin.graph(config) };
  return { id: plugin.id, graph: () => ({ modules: plugin.modules }) };
}

/**
 * Business lifecycle (private-runtime §1): Installed -> Starting -> Active ->
 * Stopping -> Stopped; a start failure lands in Failed. A link drop never
 * changes this state.
 */
export type InstanceState =
  | "installed" | "starting" | "active"
  | "stopping" | "stopped" | "failed";

/**
 * Publication lifecycle (private-runtime §1), independent of business state:
 * Unpublished -> Registering -> Published (fresh registrationIds) ->
 * Disconnected -> Registering -> Published; explicit stop ends in Revoked. A
 * failed publish round unwinds and returns to Unpublished, retrying on the
 * next ready link.
 */
export type PublicationState =
  | "unpublished" | "registering" | "published" | "disconnected" | "revoked";

/** Instance construction options. */
export interface InstanceOptions {
  /** Stop cleanup budget in ms; default 30_000, caller may shorten but never extend past 30s (private-runtime §1). */
  stopDeadlineMs?: number;
}

/** Per-stop budget; defaults to the instance budget, clamped to (0, 30_000]. */
export interface StopOptions { deadlineMs?: number }

/**
 * Link options for the instance's publication connection. The instance owns
 * reconnection (reconnect is forced on) and builds its own S1 capabilities
 * from the module graph; callers never pass either.
 */
export type InstanceLink = Omit<ConexWsClientOptions, "capabilities" | "reconnect">;

const DEFAULT_STOP_DEADLINE_MS = 30_000;
const MAX_STOP_DEADLINE_MS = 30_000;
const HUB_RETRY_INTERVAL_MS = 250;

function clampDeadline(ms: number | undefined): number {
  if (ms === undefined || !Number.isFinite(ms) || ms <= 0) return DEFAULT_STOP_DEADLINE_MS;
  return Math.min(ms, MAX_STOP_DEADLINE_MS);
}

class StopRequested extends Error {}

/** Rejects before any network I/O: duplicate names, missing deps, cycles. */
function compose(graph: PluginGraph): readonly ModuleDefinition[] {
  const names = new Set<string>();
  for (const module of graph.modules) {
    if (names.has(module.name)) throw new PluginCompositeError({ kind: "duplicate_module", module: module.name });
    names.add(module.name);
  }
  for (const module of graph.modules) {
    for (const dependency of module.depsLocal ?? []) {
      if (!names.has(dependency)) throw new PluginCompositeError({ kind: "missing_dependency", module: module.name, dependency });
    }
    for (const dependency of module.depsRemote ?? []) {
      if (!graph.remotes?.[dependency]) throw new PluginCompositeError({ kind: "missing_dependency", module: module.name, dependency });
    }
  }
  const byName = new Map(graph.modules.map(module => [module.name, module]));
  const indegree = new Map<string, number>();
  const dependents = new Map<string, string[]>();
  for (const module of graph.modules) indegree.set(module.name, 0);
  for (const module of graph.modules) for (const dependency of module.depsLocal ?? []) {
    dependents.set(dependency, [...(dependents.get(dependency) ?? []), module.name]);
    indegree.set(module.name, (indegree.get(module.name) ?? 0) + 1);
  }
  const ready = graph.modules.filter(module => indegree.get(module.name) === 0).map(module => module.name);
  const order: ModuleDefinition[] = [];
  while (ready.length) {
    const name = ready.shift()!;
    order.push(byName.get(name)!);
    for (const dependent of dependents.get(name) ?? []) {
      const left = indegree.get(dependent)! - 1;
      indegree.set(dependent, left);
      if (left === 0) ready.push(dependent);
    }
  }
  if (order.length < graph.modules.length) {
    throw new PluginCompositeError({ kind: "circular_dependency", cycle: findCycle(graph.modules, new Set(order.map(module => module.name))) });
  }
  return order;
}

/** Walks "depends on" edges inside the Kahn remainder until one repeats. */
function findCycle(modules: readonly ModuleDefinition[], settled: Set<string>): string[] {
  const byName = new Map(modules.map(module => [module.name, module]));
  for (const module of modules) {
    if (settled.has(module.name)) continue;
    const path: string[] = [];
    const seen = new Set<string>();
    let current: ModuleDefinition | undefined = module;
    while (current && !seen.has(current.name)) {
      seen.add(current.name);
      path.push(current.name);
      const next: string | undefined = (current.depsLocal ?? []).find(dependency => !settled.has(dependency));
      current = next ? byName.get(next) : undefined;
    }
    if (current) return path.slice(path.indexOf(current.name));
  }
  return [...byName.keys()];
}

function dedupeContracts(modules: readonly ModuleDefinition[]): CapabilityContract[] {
  const contracts = new Map<string, CapabilityContract>();
  for (const module of modules) contracts.set(module.contract.contractId, module.contract);
  return [...contracts.values()];
}

/**
 * One plugin instantiation: several modules, one publication link. Business
 * state, publication state and link state are independent:
 *
 * - `start()` composes (typed failures, zero connections), contacts the hub
 *   once (bounded; a refused/stalled hub never blocks local modules), starts
 *   modules in Kahn order of their local dependencies, then activates the
 *   instance and publishes. A failed publish round never fails business.
 * - A link drop only ends this link's publication and forwarding: `handles`
 *   and owned local effects survive, business stays `active`, and the next
 *   ready link republishes the same modules with fresh registrationIds
 *   (no reconstruction, no mutation replay). Remote refs are pinned at
 *   composition and fail explicitly (-32013) once replaced.
 * - `stop()` is the only path that closes owned business: reverse-topology
 *   unpublish, reverse-order module `stop` hooks, link close, bounded wait for
 *   handler tasks. Idempotent and concurrency safe: every caller receives the
 *   same terminal settle.
 */
export class Instance<I = unknown> {
  #state: InstanceState = "installed";
  #publication: PublicationState = "unpublished";
  #publicationError: unknown;
  #client?: ConexWsClient;
  readonly #references = new Map<string, ModuleReference>();
  readonly #handles = new Map<string, unknown>();
  readonly #startedOrder: { name: string; context: ModuleContext }[] = [];
  readonly #remotes = new Map<string, ModuleReference>();
  readonly #handlerTasks = new Set<Promise<unknown>>();
  #order: readonly ModuleDefinition[] = [];
  #startPromise?: Promise<void>;
  #stopPromise?: Promise<void>;
  #stopRequested = false;
  #terminal = false;
  #publishChain: Promise<void> = Promise.resolve();
  #roundBinding?: string;
  #publishedBinding?: string;
  #stopDeadlineAt?: number;
  readonly #deadlineListeners = new Set<() => void>();
  readonly #cleanupErrors: unknown[] = [];
  #retryTimer?: ReturnType<typeof setInterval>;
  #stopDeadlineMs: number;
  readonly #publicationListeners = new Set<(publication: PublicationState, error?: unknown) => void>();

  constructor(private readonly plugin: Plugin<I>, private readonly link: InstanceLink, options: InstanceOptions = {}) {
    this.#stopDeadlineMs = clampDeadline(options.stopDeadlineMs);
  }

  get state(): InstanceState { return this.#state; }

  /** Current publication state; independent of business state and link state. */
  get publication(): PublicationState { return this.#publication; }

  /** Last publish-round failure (undefined while published or unpublished clean). */
  get publicationError(): unknown { return this.#publicationError; }

  /** The underlying link state; independent of both business and publication. */
  get linkState(): ConexWsState { return this.#client?.state ?? "idle"; }

  /**
   * The live reference a started module currently publishes under, on the
   * active link only. Undefined before the first publish, after a link drop
   * (the old registrationIds died with the link) and after stop. Each
   * reconnect assigns a fresh registrationId; consumers pinning an old one
   * observe the S1 stale error.
   */
  module(name: string): ModuleReference | undefined { return this.#references.get(name); }

  /**
   * Owner access to a started module's in-process service handle. The object
   * identity is stable across link reconnects; only `stop()` releases it.
   */
  local<H>(name: string): H {
    if (this.#state !== "active") {
      throw new ConexError(`instance is ${this.#state}; local handles exist only while active`, { code: -32011, execution: "completed", retry: "never" });
    }
    if (!this.#handles.has(name)) throw new ConexError(`module "${name}" has no local handle`, { code: -32602, execution: "completed", retry: "never" });
    return this.#handles.get(name) as H;
  }

  /** Observe publication transitions; also reports the error of a failed round. */
  onPublication(listener: (publication: PublicationState, error?: unknown) => void): () => void {
    this.#publicationListeners.add(listener);
    return () => this.#publicationListeners.delete(listener);
  }

  start(config: I): Promise<void> {
    if (!this.#startPromise) this.#startPromise = this.#run(config);
    return this.#startPromise;
  }

  /**
   * Idempotent and concurrency safe: every caller receives the same terminal
   * settle. Closes owned business only — reverse unpublish, reverse module
   * stop hooks, link close, bounded handler wait — within the stop deadline.
   */
  stop(options: StopOptions = {}): Promise<void> {
    this.#stopRequested = true;
    const deadlineMs = clampDeadline(options.deadlineMs ?? this.#stopDeadlineMs);
    this.#stopDeadlineAt = Math.min(this.#stopDeadlineAt ?? Infinity, Date.now() + deadlineMs);
    for (const changed of this.#deadlineListeners) changed();
    if (this.#stopPromise) return this.#stopPromise;
    this.#stopPromise = this.#state === "installed"
      ? this.#finishInstalledStop()
      : this.#state === "starting"
        ? this.#bounded(this.#startPromise!.catch(() => undefined), deadlineMs)
            .catch(error => { this.#cleanupErrors.push(error); })
            .then(() => this.#stopActive(deadlineMs))
        : this.#stopActive(deadlineMs);
    return this.#stopPromise;
  }

  async #run(config: I): Promise<void> {
    if (this.#state !== "installed") {
      throw new ConexError(`instance is ${this.#state}; create a new instance instead`, { code: -32011, execution: "completed", retry: "never" });
    }
    this.#state = "starting";
    let client: ConexWsClient | undefined;
    try {
      // Composition phase: pure declaration checks before any network frame.
      const graph = this.plugin.graph(config);
      this.#order = compose(graph);
      for (const [name, reference] of Object.entries(graph.remotes ?? {})) this.#remotes.set(name, reference);
      const made = new ConexWsClient({
        ...this.link,
        reconnect: true,
        capabilities: { contracts: dedupeContracts(this.#order), providers: this.#order.map(module => this.#provider(module)) },
      });
      client = made;
      this.#client = made;
      made.onEvent(event => {
        if (event.phase === "ready") this.#onLinkReady();
        else if (event.phase === "close") this.#onLinkLost();
      });
      // Local-only business never waits for the hub. A module that explicitly
      // requires a pinned remote binding may await this bounded first contact.
      let contact: Promise<boolean> | undefined;
      this.#checkpoint();
      const deps = this.#deps();
      for (const module of this.#order) {
        this.#checkpoint();
        if (module.depsRemote?.length) {
          contact ??= made.connect().then(() => true, () => false);
          if (!await contact) throw new PluginCompositeError({ kind: "start_failed", module: module.name,
            cause: new ConexError("required remote link unavailable", { code: -32005, execution: "not_started", retry: "never" }) });
        }
        const context: ModuleContext = { deps };
        try {
          const handle = await module.start(context);
          this.#handles.set(module.name, handle);
          // The entry carries its own cleanup context so a stop that expired
          // mid-start (and cleared the registries) can never lose it.
          this.#startedOrder.push({ name: module.name, context });
        } catch (cause) {
          throw new PluginCompositeError({ kind: "start_failed", module: module.name, cause });
        }
        this.#checkpoint();
      }
      this.#state = "active";
      if (!contact) void made.connect().catch(() => undefined);
      this.#publishPending();
      if (!this.#terminal) this.#startRetryDriver();
    } catch (error) {
      // Start-failure rollback: reverse module stop hooks, then close the
      // link (aborts wire consumers/handlers), then bounded handler wait.
      await this.#releaseStarted(this.#stopDeadlineMs);
      client?.close();
      try { await this.#waitHandlers(this.#stopDeadlineMs); } catch (cleanup) { this.#cleanupErrors.push(cleanup); }
      this.#handles.clear();
      this.#references.clear();
      this.#startedOrder.length = 0;
      this.#terminal = true;
      this.#stopRetryDriver();
      const stopped = error instanceof StopRequested;
      this.#state = stopped ? "stopped" : "failed";
      if (stopped) this.#setPublication("revoked");
      if (this.#cleanupErrors.length) {
        this.#state = "failed";
        throw new AggregateError([error, ...this.#cleanupErrors], "startup rollback incomplete");
      }
      if (!stopped) throw error;
    }
  }

  #checkpoint(): void {
    if (this.#stopRequested) throw new StopRequested();
  }

  #deps(): ModuleDeps {
    return {
      local: <H>(name: string): H => {
        if (!this.#handles.has(name)) throw new ConexError(`local dependency "${name}" has no handle`, { code: -32602, execution: "completed", retry: "never" });
        return this.#handles.get(name) as H;
      },
      remote: name => {
        const reference = this.#remotes.get(name);
        if (!reference) throw new ConexError(`remote binding "${name}" is not configured`, { code: -32602, execution: "completed", retry: "never" });
        const client = this.#client!;
        return {
          reference,
          invoke: (method, argument, options) => client.invoke(reference, method, argument, options),
          subscribe: (state, argument, update, options) => client.subscribe(reference, state, argument, update, options),
        };
      },
    };
  }

  /** Wraps handlers so stop can wait for them after aborting their signals. */
  #provider(module: ModuleDefinition): ModuleProvider {
    const handlers: ModuleProvider["handlers"] = {};
    for (const [name, handler] of Object.entries(module.handlers)) {
      handlers[name] = (argument, context, signal) => this.#tracked(handler(argument, context, signal));
    }
    const states: ModuleProvider["states"] = {};
    for (const [name, open] of Object.entries(module.states)) {
      states[name] = (argument, observer, context, signal) => this.#tracked(open(argument, observer, context, signal));
    }
    return {
      module: { ...module.registration, contractId: module.contract.contractId },
      handlers,
      states,
      authorize: (context, member, argument) => this.#state === "active"
        && this.#references.get(module.name)?.registrationId === context.registrationId
        && (module.authorize?.(context, member, argument) ?? true),
    };
  }

  #tracked<T>(pending: T | Promise<T>): Promise<T> {
    const task = Promise.resolve(pending);
    this.#handlerTasks.add(task);
    void task.catch(() => undefined).finally(() => this.#handlerTasks.delete(task));
    return task;
  }

  /** A new link is ready: republish the existing modules with fresh refs. */
  #onLinkReady(): void {
    if (this.#terminal || this.#stopRequested) return;
    if (this.#state === "active") this.#publishPending();
  }

  /** The link died: end only this link's forwarding/subscriptions and refs. */
  #onLinkLost(): void {
    if (this.#stopRequested || this.#terminal) return;
    this.#references.clear();
    this.#publishedBinding = undefined;
    if (this.#state === "active") this.#setPublication("disconnected");
  }

  /** Queue one publish round per link (serialized, binding-guarded). */
  #publishPending(): void {
    const client = this.#client;
    const binding = client?.negotiation?.negotiationId;
    if (!client || client.state !== "ready" || !binding || this.#terminal || this.#stopRequested) return;
    if (this.#roundBinding === binding) return;
    this.#roundBinding = binding;
    this.#publishChain = this.#publishChain.then(() => this.#publishRound(binding));
  }

  /**
   * Publish every started module on the given link. Mid-round failure unwinds
   * this round's fresh registrations in reverse; business stays active and the
   * next ready link retries. Never reconstructs modules or replays mutations.
   */
  async #publishRound(binding: string): Promise<void> {
    const client = this.#client;
    if (!client || this.#terminal || this.#stopRequested || this.#state !== "active") return;
    if (client.state !== "ready" || client.negotiation?.negotiationId !== binding || this.#publishedBinding === binding) return;
    this.#setPublication("registering");
    const fresh: ModuleReference[] = [];
    try {
      for (const module of this.#order) {
        if (this.#stopRequested || this.#terminal || client.negotiation?.negotiationId !== binding || client.state !== "ready") throw new StopRequested();
        const reference = await client.publishModule({ ...module.registration, contractId: module.contract.contractId });
        if (this.#stopRequested || this.#terminal || client.negotiation?.negotiationId !== binding || client.state !== "ready") throw new StopRequested();
        this.#references.set(module.name, reference);
        fresh.push(reference);
      }
      this.#publishedBinding = binding;
      this.#publicationError = undefined;
      this.#setPublication("published");
    } catch (error) {
      for (const reference of fresh.reverse()) {
        try { await client.unpublishModule(reference); } catch { /* S1 generation cleanup is the backstop. */ }
      }
      this.#references.clear();
      // The link may still be alive (e.g. the registration was refused);
      // retry happens on the next ready link. If the link died mid-round the
      // close handler already left Disconnected; don't overwrite it.
      if (!this.#stopRequested && !this.#terminal && client.state === "ready" && client.negotiation?.negotiationId === binding) this.#setPublication("unpublished", error);
    }
  }

  /** Reverse-topology stop hooks for started modules; bounded, exactly once. */
  async #releaseStarted(deadlineMs: number): Promise<void> {
    const deadline = Date.now() + deadlineMs;
    const owned = this.#startedOrder.splice(0).reverse().map(entry => ({
      module: this.#order.find(candidate => candidate.name === entry.name),
      context: entry.context,
      handle: this.#handles.get(entry.name),
    }));
    for (const { module, context, handle } of owned) {
      if (!module?.stop) continue;
      try {
        await this.#bounded(Promise.resolve(module.stop(context, handle)), deadline - Date.now());
      } catch (error) { this.#cleanupErrors.push(error); }
    }
  }

  /** Bounded last turn for handlers observed their abort signals. */
  async #waitHandlers(deadlineMs: number): Promise<void> {
    const tasks = [...this.#handlerTasks];
    if (!tasks.length) return;
    await this.#bounded(Promise.allSettled(tasks), deadlineMs);
  }

  /** Link retries for the hub-absent case; reconnection is never the caller's. */
  #startRetryDriver(): void {
    if (this.#retryTimer) return;
    this.#retryTimer = setInterval(() => {
      const client = this.#client;
      if (this.#terminal || !client || client.state === "closed" || client.state === "reconnecting") return;
      if (client.state === "failed" || client.state === "idle") {
        void client.connect().catch(() => undefined);
      }
    }, HUB_RETRY_INTERVAL_MS);
  }

  #stopRetryDriver(): void {
    clearInterval(this.#retryTimer);
    this.#retryTimer = undefined;
  }

  async #finishInstalledStop(): Promise<void> {
    this.#state = "stopped";
    this.#terminal = true;
    this.#setPublication("revoked");
  }

  async #stopActive(deadlineMs: number): Promise<void> {
    if (this.#state === "stopped" || this.#state === "failed") return;
    this.#state = "stopping";
    this.#setPublication("revoked");
    const deadline = Date.now() + deadlineMs;
    // Reverse-topology unpublish of live refs when the link is reachable.
    const client = this.#client;
    if (client && client.state === "ready") {
      for (const reference of [...this.#references.values()].reverse()) {
        try { await this.#bounded(client.unpublishModule(reference), deadline - Date.now()); } catch { /* Release continues. */ }
      }
    }
    this.#references.clear();
    this.#publishedBinding = undefined;
    // Reverse cleanup of owned local effects (module stop hooks).
    await this.#releaseStarted(deadline - Date.now());
    // Close the link: ends every consumer and aborts every remaining handler.
    client?.close();
    // Handlers observed their abort signals; give them their last turn.
    try { await this.#waitHandlers(deadline - Date.now()); } catch (error) { this.#cleanupErrors.push(error); }
    this.#handles.clear();
    this.#startedOrder.length = 0;
    this.#terminal = true;
    this.#stopRetryDriver();
    this.#state = this.#cleanupErrors.length ? "failed" : "stopped";
    if (this.#cleanupErrors.length) throw new AggregateError(this.#cleanupErrors, "module cleanup incomplete");
  }

  #bounded<T>(pending: Promise<T>, ms: number): Promise<T> {
    const cutoff = Date.now() + Math.max(0, ms);
    const { promise, reject } = Promise.withResolvers<T>();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      clearTimeout(timer);
      const remaining = Math.min(cutoff, this.#stopDeadlineAt ?? Infinity) - Date.now();
      const expired = () => reject(new ConexError("cleanup deadline exceeded", { code: -32006, execution: "unknown", retry: "never" }));
      if (remaining <= 0) expired();
      else timer = setTimeout(expired, remaining);
    };
    this.#deadlineListeners.add(schedule);
    schedule();
    return Promise.race([pending, promise]).finally(() => {
      clearTimeout(timer);
      this.#deadlineListeners.delete(schedule);
    });
  }


  #setPublication(publication: PublicationState, error?: unknown): void {
    this.#publication = publication;
    if (error !== undefined) this.#publicationError = error;
    this.#emitPublication();
  }

  #emitPublication(): void {
    for (const listener of [...this.#publicationListeners]) {
      try {
        listener(this.#publication, this.#publicationError);
      } catch {
        // Observers must not break the instance.
      }
    }
  }
}
