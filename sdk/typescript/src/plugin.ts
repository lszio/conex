//! S2 plugin runtime: plugin classes, instances, local DI and explicit remote
//! bindings. Behavior authority: docs/contracts/s2-plugins.md. One instance is
//! exactly one S1 connection; the center is unchanged.

import { ConexError } from "./errors";
import type {
  CapabilityContract, Invocation, InvokeOptions, ModuleProvider, ModuleReference, VerifiedContext,
} from "./capability";
import type { Sample, StateSubscription } from "./state";
import { ConexWsClient } from "./ws-client";
import type { ConexWsClientOptions } from "./ws-client";

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

export type InstanceState =
  | "installed" | "starting" | "active"
  | "stopping" | "stopped" | "failed" | "broken";

/** Link options for the instance's single S1 connection; reconnection is never ours. */
export type InstanceLink = Omit<ConexWsClientOptions, "capabilities" | "reconnect">;

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
 * One plugin instantiation: one connection, several modules, one generation.
 * A failed or broken instance never restarts; the upper layer creates a new
 * instance (and therefore a new generation with fresh ModuleRefs).
 */
export class Instance<I = unknown> {
  #state: InstanceState = "installed";
  #client?: ConexWsClient;
  readonly #references = new Map<string, ModuleReference>();
  readonly #handles = new Map<string, unknown>();
  readonly #remotes = new Map<string, ModuleReference>();
  readonly #handlerTasks = new Set<Promise<unknown>>();
  #order: readonly ModuleDefinition[] = [];
  #startPromise?: Promise<void>;
  #inflight?: Promise<void>;
  #stopRequested = false;

  constructor(private readonly plugin: Plugin<I>, private readonly link: InstanceLink) {}

  get state(): InstanceState { return this.#state; }

  /** The live reference a started module publishes under; undefined before start. */
  module(name: string): ModuleReference | undefined { return this.#references.get(name); }

  start(config: I): Promise<void> {
    if (!this.#startPromise) this.#startPromise = this.#run(config);
    return this.#startPromise;
  }

  /** Idempotent and concurrency safe: every caller gets the same terminal settle. */
  stop(): Promise<void> {
    this.#stopRequested = true;
    if (!this.#inflight) {
      this.#inflight = this.#state === "starting"
        ? this.#startPromise!.catch(() => undefined)
        : this.#stopNow();
    }
    return this.#inflight;
  }

  async #run(config: I): Promise<void> {
    if (this.#state !== "installed") {
      throw new ConexError(`instance is ${this.#state}; create a new instance instead`, { code: -32011, execution: "completed", retry: "never" });
    }
    this.#state = "starting";
    try {
      // Composition phase: pure declaration checks before any network frame.
      const graph = this.plugin.graph(config);
      this.#order = compose(graph);
      for (const [name, reference] of Object.entries(graph.remotes ?? {})) this.#remotes.set(name, reference);
      const client = new ConexWsClient({
        ...this.link,
        reconnect: false,
        capabilities: { contracts: dedupeContracts(this.#order), providers: this.#order.map(module => this.#provider(module)) },
      });
      this.#client = client;
      client.onEvent(event => { if (event.phase === "close" && this.#state === "active") void this.#broken(); });
      await client.connect();
      this.#checkpoint();
      const deps = this.#deps();
      for (const module of this.#order) {
        this.#checkpoint();
        try {
          this.#handles.set(module.name, await module.start({ deps }));
        } catch (cause) {
          throw new PluginCompositeError({ kind: "start_failed", module: module.name, cause });
        }
      }
      for (const module of this.#order) {
        this.#checkpoint();
        const reference = await client.publishModule({ ...module.registration, contractId: module.contract.contractId });
        this.#references.set(module.name, reference);
      }
      this.#state = "active";
    } catch (error) {
      await this.#teardown(true);
      const stopped = error instanceof StopRequested;
      this.#state = stopped ? "stopped" : "failed";
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
      authorize: module.authorize ?? (() => true),
    };
  }

  #tracked<T>(pending: T | Promise<T>): Promise<T> {
    const task = Promise.resolve(pending);
    this.#handlerTasks.add(task);
    void task.catch(() => undefined).finally(() => this.#handlerTasks.delete(task));
    return task;
  }

  async #stopNow(): Promise<void> {
    if (this.#state === "active") {
      this.#state = "stopping";
      await this.#teardown(true);
      this.#state = "stopped";
    } else if (this.#state === "installed") {
      this.#state = "stopped";
    }
    // failed/broken/stopped are already terminal; stop stays idempotent.
  }

  /** Transport died on its own: same release path, no revival, no reconnect. */
  async #broken(): Promise<void> {
    await this.#teardown(false);
    this.#state = "broken";
  }

  /**
   * Reverse-topology unpublish -> end subscriptions/consumers -> abort
   * handlers and wait -> close the link. With `frames` false the transport is
   * already gone: registrations and subscriptions died with the link and the
   * S1 generation cleanup is the backstop, so only local release remains.
   */
  async #teardown(frames: boolean): Promise<void> {
    const client = this.#client;
    if (client && frames && client.state === "ready") {
      for (const reference of [...this.#references.values()].reverse()) {
        try { await client.unpublishModule(reference); } catch { /* Release continues; generation cleanup is the backstop. */ }
      }
    }
    this.#references.clear();
    this.#handles.clear();
    // client.close() ends every consumer and aborts every remaining handler.
    client?.close();
    // Handlers observed their abort signals; give them their last turn.
    await Promise.all([...this.#handlerTasks]);
  }
}
