// S2 plugin runtime acceptance (docs/contracts/s2-plugins.md §8 amended by
// docs/contracts/private-runtime.md §1 and MVP-P2 §5 P2.4): S2-A01 dual
// instances, A02 local DI with zero broker frames, A04 missing dependency
// rejection, A05 cycle rejection, A06 stop release. Business state,
// publication state and link state are independent: a link drop never destroys
// business (A08), reconnect republishes existing modules with fresh refs
// without reconstruction, remote refs stay pinned, and only explicit stop
// closes owned local effects. Runs against the in-process S1 broker fixture
// (fixtures/s2-host.ts) so every module start/publish/stop crosses a real
// WebSocket connection with real wire frames.
import { expect, test } from "bun:test";
import { ConexWsClient } from "../src/ws-client";
import type { WebSocketLike } from "../src/ws-client";
import type { CapabilityContract, ModuleReference } from "../src/capability";
import { AtomicState } from "../src/state";
import type { Sample } from "../src/state";
import { Instance, PluginCompositeError, definePlugin } from "../src/plugin";
import type { InstanceLink, ModuleDefinition, Plugin, PublicationState } from "../src/plugin";
import { s2Host } from "./fixtures/s2-host";
import type { S2Host } from "./fixtures/s2-host";

type CounterValue = { count: number };
interface CounterHandle {
  read(): Sample<CounterValue>;
  observe(update: (sample: Sample<CounterValue>) => void): () => void;
}

function int32(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < -2147483648 || value > 2147483647) throw new Error(`invalid int32 ${field}`);
  return value;
}
function emptyArgument(value: unknown): Record<string, never> {
  if (value && typeof value === "object" && Object.keys(value).length > 0) throw new Error("unexpected argument");
  return {};
}
function countResult(value: unknown): CounterValue {
  if (!value || typeof value !== "object" || !("count" in value)) throw new Error("missing count");
  return { count: int32(value.count, "count") };
}
function addArgument(value: unknown): { delta: number } {
  if (!value || typeof value !== "object" || !("delta" in value)) throw new Error("missing delta");
  return { delta: int32(value.delta, "delta") };
}

const counterContract: CapabilityContract = {
  contractId: "test.counter",
  methods: {
    add: { execution: "non_replayable", input: addArgument, output: countResult },
    read: { execution: "read_only", input: emptyArgument, output: countResult },
  },
  states: { value: { input: emptyArgument, value: countResult } },
};

/** The module graph S2-A01 instantiates twice; all domain state is per-graph. */
function counterPlugin(): Plugin<{ endpoint: string }> {
  return definePlugin<{ endpoint: string }>({
    id: "counter",
    graph(config) {
      const registration = (suffix: string) => ({ spaceId: "test", providerEndpointId: config.endpoint + suffix, integrationId: "integration" });
      const domain = new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" });
      const derived = new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" });
      const commit = (state: AtomicState<CounterValue>, count: number) =>
        state.publish({ value: { count }, revision: (BigInt(state.read().revision) + 1n).toString(), sampledAtMs: "1", quality: "current" });
      const modules: ModuleDefinition[] = [
        {
          name: "counter",
          contract: counterContract,
          registration: registration(""),
          handlers: {
            add: raw => {
              const count = domain.read().value.count + addArgument(raw).delta;
              commit(domain, count);
              return { count };
            },
            read: () => domain.read().value,
          },
          states: { value: (_argument, observer, _context, signal) => domain.open({}, observer, signal) },
          start: async (): Promise<CounterHandle> => ({
            read: () => domain.read(),
            observe(update) {
              const controller = new AbortController();
              domain.open({}, { update, end: () => undefined }, controller.signal);
              return () => controller.abort();
            },
          }),
        },
        {
          name: "derived",
          contract: counterContract,
          registration: registration(".derived"),
          depsLocal: ["counter"],
          handlers: {
            add: raw => {
              const count = derived.read().value.count + addArgument(raw).delta * 2;
              commit(derived, count);
              return { count };
            },
            read: () => derived.read().value,
          },
          states: { value: (_argument, observer, _context, signal) => derived.open({}, observer, signal) },
          start: async ctx => {
            const upstream = ctx.deps.local<CounterHandle>("counter");
            // Snapshot first: derived publishes only what it has already seen.
            const initial = upstream.read();
            commit(derived, initial.value.count * 2);
            let lastSource = initial.revision;
            upstream.observe(sample => {
              if (sample.revision === lastSource) return;
              lastSource = sample.revision;
              commit(derived, sample.value.count * 2);
            });
          },
        },
      ];
      return { modules };
    },
  });
}

/** A module whose start throws; failure-path tests tweak deps via overrides. */
function failingModule(name: string, overrides: Partial<ModuleDefinition> = {}): ModuleDefinition {
  const domain = new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" });
  return {
    name,
    contract: counterContract,
    registration: { spaceId: "test", providerEndpointId: `fail-${name}`, integrationId: "integration" },
    handlers: {
      add: raw => {
        const count = domain.read().value.count + addArgument(raw).delta;
        domain.publish({ value: { count }, revision: (BigInt(domain.read().revision) + 1n).toString(), sampledAtMs: "1", quality: "current" });
        return { count };
      },
      read: () => domain.read().value,
    },
    states: { value: (_argument, observer, _context, signal) => domain.open({}, observer, signal) },
    start: () => {
      throw new Error("boom");
    },
    ...overrides,
  };
}

const CYCLE_DEPS: Record<string, string[]> = { a: ["c"], b: ["a"], c: ["b"] };

function serviceLink(port: number): InstanceLink {
  const ServiceSocket = globalThis.WebSocket as unknown as new (url: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
  return {
    origin: `http://127.0.0.1:${port}`,
    tokenProvider: async () => "test-token",
    createWebSocket: (url, headers) => new ServiceSocket(url, { headers }),
  };
}

async function consumer(port: number): Promise<ConexWsClient> {
  const client = new ConexWsClient({ ...serviceLink(port), capabilities: { contracts: [counterContract] } });
  await client.connect();
  return client;
}

/**
 * Polls a condition on a peer (fixture broker link) that offers no event
 * surface; bounded so a hang fails instead of stalling the suite.
 */
async function until<T>(probe: () => T | undefined, what: string): Promise<T> {
  for (let deadline = Date.now() + 2000; Date.now() < deadline;) {
    const value = probe();
    if (value !== undefined) return value;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  throw new Error(`timed out waiting for ${what}`);
}

const endpointsOf = (host: S2Host, kind: "register" | "unregister") =>
  host.events.filter(event => event.kind === kind).map(event => event.providerEndpointId);

test("S2-A01 one plugin class, two instances keep independent counters, revisions and generations", async () => {
  const host = s2Host();
  try {
    const plugin = counterPlugin();
    const a = new Instance(plugin, serviceLink(host.port));
    const b = new Instance(plugin, serviceLink(host.port));
    await a.start({ endpoint: "counter-a" });
    await b.start({ endpoint: "counter-b" });
    await until(() => a.publication === "published" && b.publication === "published" ? true : undefined, "both publications");
    expect(a.state).toBe("active");
    expect(b.state).toBe("active");
    expect(a.publication).toBe("published");
    expect(b.publication).toBe("published");
    expect(host.references()).toHaveLength(4);
    const refA = a.module("counter")!;
    const refB = b.module("counter")!;
    expect(refA.registrationId).not.toBe(refB.registrationId);
    const client = await consumer(host.port);
    expect(await client.invoke(refA, "add", { delta: 3 }).result).toEqual({ count: 3 });
    expect(await client.invoke(refB, "read", {}).result).toEqual({ count: 0 });
    const snapshotA = await client.subscribe<Record<string, never>, CounterValue>(refA, "value", {}, () => undefined).snapshot;
    const snapshotB = await client.subscribe<Record<string, never>, CounterValue>(refB, "value", {}, () => undefined).snapshot;
    expect(`${snapshotA.revision}`).toBe("1");
    expect(`${snapshotB.revision}`).toBe("0");
    await a.stop();
    await b.stop();
  } finally { host.stop(); }
});

test("S2-A02 derived module consumes its local dependency in-process with zero broker frames", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(counterPlugin(), serviceLink(host.port));
    await instance.start({ endpoint: "local" });
    await until(() => instance.publication === "published" ? true : undefined, "local publication");
    // Kahn order: the dependency started (and registered) before its dependent.
    expect(endpointsOf(host, "register")).toEqual(["local", "local.derived"]);
    const counterRef = instance.module("counter")!;
    const derivedRef = instance.module("derived")!;
    const client = await consumer(host.port);
    const updates: Sample<CounterValue>[] = [];
    const subscription = client.subscribe<Record<string, never>, CounterValue>(derivedRef, "value", {}, sample => updates.push(sample));
    const snapshot = await subscription.snapshot;
    expect(snapshot.value).toEqual({ count: 0 });
    expect(`${snapshot.revision}`).toBe("1"); // derived keeps its own revision sequence
    await client.invoke(counterRef, "add", { delta: 5 }).result;
    expect(await until(() => updates[0], "derived update")).toMatchObject({ value: { count: 10 } });
    // The local dependency edge never touched the wire.
    expect(host.events.filter(event => event.kind === "subscribe" && event.registrationId === counterRef.registrationId)).toHaveLength(0);
    await instance.stop();
  } finally { host.stop(); }
});

test("an explicit remote binding invokes and subscribes to another instance's module", async () => {
  const host = s2Host();
  try {
    const source = new Instance(counterPlugin(), serviceLink(host.port));
    await source.start({ endpoint: "src" });
    await until(() => source.publication === "published" ? true : undefined, "source publication");
    const peerRef = source.module("counter")!;
    let mirrored = -1;
    const mirrorDomain = new AtomicState<CounterValue>({ value: { count: -1 }, revision: "0", sampledAtMs: "0", quality: "current" });
    let mirrorSnapshot: Sample<CounterValue> | undefined;
    const mirrorInstance = new Instance(definePlugin<{ endpoint: string; peer: ModuleReference }>({
      id: "mirror",
      graph: config => ({
        modules: [{
          name: "mirror",
          contract: counterContract,
          registration: { spaceId: "test", providerEndpointId: config.endpoint, integrationId: "integration" },
          depsRemote: ["peer"],
          handlers: {
            read: () => ({ count: mirrored }),
            add: () => {
              throw new Error("mirror add unsupported");
            },
          },
          states: { value: (_argument, observer, _context, signal) => mirrorDomain.open({}, observer, signal) },
          start: async ctx => {
            const remote = ctx.deps.remote("peer");
            mirrored = (await remote.invoke<Record<string, never>, CounterValue>("read", {}).result).count;
            mirrorSnapshot = await remote.subscribe<Record<string, never>, CounterValue>("value", {}, () => undefined).snapshot;
            mirrorDomain.publish({ value: { count: mirrored }, revision: "1", sampledAtMs: "1", quality: "current" });
          },
        }],
        remotes: { peer: config.peer },
      }),
    }), serviceLink(host.port));
    await mirrorInstance.start({ endpoint: "mirror", peer: peerRef });
    await until(() => mirrorInstance.publication === "published" ? true : undefined, "mirror publication");
    const client = await consumer(host.port);
    expect(await client.invoke(mirrorInstance.module("mirror")!, "read", {}).result).toEqual({ count: 0 });
    expect(mirrorSnapshot).toMatchObject({ value: { count: 0 } });
    await source.stop();
    await mirrorInstance.stop();
  } finally { host.stop(); }
});

test("S2-A04 missing local dependency and missing remote binding refuse with zero connections and zero publishes", async () => {
  const host = s2Host();
  try {
    const ghostLocal = new Instance(definePlugin({
      id: "ghost-local",
      modules: [failingModule("orphan", { depsLocal: ["ghost"], start: () => undefined })],
    }), serviceLink(host.port));
    await expect(ghostLocal.start({})).rejects.toMatchObject({
      name: "PluginCompositeError",
      failure: { kind: "missing_dependency", module: "orphan", dependency: "ghost" },
    });
    const ghostRemote = new Instance(definePlugin({
      id: "ghost-remote",
      modules: [failingModule("wan", { depsRemote: ["peer.counter"], start: () => undefined })],
    }), serviceLink(host.port));
    await expect(ghostRemote.start({})).rejects.toMatchObject({
      failure: { kind: "missing_dependency", module: "wan", dependency: "peer.counter" },
    });
    expect(ghostLocal.state).toBe("failed");
    expect(ghostRemote.state).toBe("failed");
    expect(host.references()).toHaveLength(0);
    expect(endpointsOf(host, "register")).toEqual([]);
  } finally { host.stop(); }
});

test("duplicate module names are rejected at composition", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(definePlugin({
      id: "dupe",
      modules: [failingModule("same"), failingModule("same", { registration: { spaceId: "test", providerEndpointId: "other", integrationId: "integration" } })],
    }), serviceLink(host.port));
    await expect(instance.start({})).rejects.toMatchObject({
      failure: { kind: "duplicate_module", module: "same" },
    });
    expect(instance.state).toBe("failed");
    expect(endpointsOf(host, "register")).toEqual([]);
  } finally { host.stop(); }
});

test("S2-A05 a local dependency cycle is rejected with the cycle path before any network frame", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(definePlugin({
      id: "loop",
      modules: (["a", "b", "c"] as const).map(name => failingModule(name, { depsLocal: [CYCLE_DEPS[name]![0]], start: () => undefined })),
    }), serviceLink(host.port));
    const failure = await instance.start({}).then(() => undefined, (error: unknown) => error);
    if (!(failure instanceof PluginCompositeError) || failure.failure.kind !== "circular_dependency") {
      throw failure ?? new Error("start should have failed");
    }
    const cycle = [...failure.failure.cycle].sort();
    expect(cycle).toEqual(["a", "b", "c"]);
    // Consecutive pairs follow "depends on" edges around the ring.
    for (const [index, name] of failure.failure.cycle.entries()) {
      expect(CYCLE_DEPS[name]).toContain(failure.failure.cycle[(index + 1) % failure.failure.cycle.length]);
    }
    expect(instance.state).toBe("failed");
    expect(endpointsOf(host, "register")).toEqual([]);
  } finally { host.stop(); }
});

test("a module start failure fails the instance, closes the link and publishes nothing", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(definePlugin({
      id: "boom",
      modules: [failingModule("healthy", { start: () => undefined }), failingModule("boom", { depsLocal: ["healthy"] })],
    }), serviceLink(host.port));
    await expect(instance.start({})).rejects.toMatchObject({
      failure: { kind: "start_failed", module: "boom" },
    });
    expect(instance.state).toBe("failed");
    expect(instance.publication).toBe("unpublished");
    expect(instance.module("counter")).toBeUndefined();
    expect(endpointsOf(host, "register")).toEqual([]);
    expect(instance.linkState).toBe("closed");
  } finally { host.stop(); }
});

test("a failed publish round unwinds fresh registrations but never fails business", async () => {
  const host = s2Host();
  try {
    // Both modules share (spaceId, providerEndpointId): the second publish is
    // refused locally by the S1 session's already-published guard.
    const registration = { spaceId: "test", providerEndpointId: "collision", integrationId: "integration" };
    const instance = new Instance(definePlugin({
      id: "collision",
      modules: [failingModule("first", { start: () => undefined, registration }), failingModule("second", { depsLocal: ["first"], start: () => undefined, registration })],
    }), serviceLink(host.port));
    const transitions: PublicationState[] = [];
    instance.onPublication(state => transitions.push(state));
    // Publication is independent of business: start resolves active even
    // though this round can never publish.
    await instance.start({});
    await until(() => instance.publicationError !== undefined ? true : undefined, "publication refusal");
    expect(instance.state).toBe("active");
    expect(instance.publication).toBe("unpublished");
    expect(transitions).toContain("registering");
    expect(instance.publicationError).toMatchObject({ code: -32014 });
    // The failed round unwound its own fresh registration in reverse order.
    await until(() => host.events.some(event => event.kind === "unregister") || undefined, "unwind");
    expect(endpointsOf(host, "unregister")).toEqual(["collision"]);
    expect(host.references()).toHaveLength(0);
    await instance.stop();
    expect(instance.state).toBe("stopped");
    expect(instance.publication).toBe("revoked");
  } finally { host.stop(); }
});

test("S2-A06 stop unpublishes in reverse topology, ends subscriptions, and leaves a sibling instance serving", async () => {
  const host = s2Host();
  try {
    const plugin = counterPlugin();
    const doomed = new Instance(plugin, serviceLink(host.port));
    const survivor = new Instance(plugin, serviceLink(host.port));
    await doomed.start({ endpoint: "doomed" });
    await survivor.start({ endpoint: "survivor" });
    await until(() => doomed.publication === "published" && survivor.publication === "published" ? true : undefined, "both publications");
    const doomedCounter = doomed.module("counter")!;
    const doomedDerived = doomed.module("derived")!;
    const survivorCounter = survivor.module("counter")!;
    const client = await consumer(host.port);
    const subscription = client.subscribe<Record<string, never>, CounterValue>(doomedDerived, "value", {}, () => undefined);
    await subscription.snapshot;
    await doomed.stop();
    expect(doomed.state).toBe("stopped");
    expect(doomed.publication).toBe("revoked");
    // Reverse topology: the dependent module unregistered before its dependency.
    expect(endpointsOf(host, "unregister").filter(endpoint => endpoint!.startsWith("doomed"))).toEqual(["doomed.derived", "doomed"]);
    // The consumer's subscription was terminated, not left dangling.
    expect((await subscription.ended).code).toBeDefined();
    // The sibling instance keeps serving calls on its own link.
    expect(await client.invoke(survivorCounter, "add", { delta: 2 }).result).toEqual({ count: 2 });
    expect(survivor.state).toBe("active");
    expect(host.references().find(reference => reference.registrationId === doomedCounter.registrationId)).toBeUndefined();
    // stop() is idempotent: a second call settles to the same terminal state.
    await doomed.stop();
    expect(doomed.state).toBe("stopped");
    await survivor.stop();
  } finally { host.stop(); }
});

test("stop aborts an in-flight provider handler and waits for it to settle", async () => {
  const host = s2Host();
  try {
    let handlerSettled = false;
    const entered = Promise.withResolvers<void>();
    const aborted = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    const instance = new Instance(definePlugin<{ endpoint: string }>({
      id: "blocked",
      graph: config => ({
        modules: [{
          name: "counter",
          contract: counterContract,
          registration: { spaceId: "test", providerEndpointId: config.endpoint, integrationId: "integration" },
          handlers: {
            add: (_raw, _context, signal) => {
              entered.resolve();
              signal.addEventListener("abort", () => aborted.resolve(), { once: true });
              return release.promise.then(() => { handlerSettled = true; return { count: 1 }; });
            },
            read: () => ({ count: 0 }),
          },
          states: { value: (_argument, observer, _context, signal) => new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" }).open({}, observer, signal) },
          start: () => undefined,
        }],
      }),
    }), serviceLink(host.port));
    await instance.start({ endpoint: "blocked" });
    await until(() => instance.publication === "published" ? true : undefined, "blocked provider publication");
    const client = await consumer(host.port);
    const call = client.invoke(instance.module("counter")!, "add", { delta: 1 });
    void call.result.catch(() => undefined);
    await entered.promise;
    const stopped = instance.stop();
    await aborted.promise;
    expect(handlerSettled).toBe(false);
    release.resolve();
    await stopped;
    expect(handlerSettled).toBe(true);
    expect(instance.state).toBe("stopped");
    // The consumer's own invoke settles later (its reply cannot reach a closed
    // link); handled, not awaited — the runtime release is what we assert.
    void call.result.catch(() => undefined);
  } finally { host.stop(); }
});

test("start resolves active when the hub is absent; local modules serve and publication stays unpublished", async () => {
  const host = s2Host();
  const port = host.port;
  host.stop(); // the hub is gone before the instance ever connects
  const instance = new Instance(counterPlugin(), serviceLink(port));
  await instance.start({ endpoint: "offline" });
  expect(instance.state).toBe("active");
  // Local DI serves in-process with zero hub reachable.
  expect(instance.local<CounterHandle>("counter").read().value).toEqual({ count: 0 });
  expect(instance.publication).toBe("unpublished");
  expect(instance.module("counter")).toBeUndefined();
  expect(instance.linkState).not.toBe("ready");
  await instance.stop();
  expect(instance.state).toBe("stopped");
  expect(instance.publication).toBe("revoked");
});

test("S2-A08 a link drop stops publication only; reconnect republishes fresh refs without reconstruction", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(counterPlugin(), serviceLink(host.port));
    await instance.start({ endpoint: "counter-a" });
    await until(() => instance.publication === "published" ? true : undefined, "initial publication");
    expect(instance.state).toBe("active");
    expect(instance.publication).toBe("published");
    const staleRef = instance.module("counter")!;
    // Business mutates before the drop; a reconstruction would lose this.
    const client = await consumer(host.port);
    expect(await client.invoke(staleRef, "add", { delta: 3 }).result).toEqual({ count: 3 });
    const handle = instance.local<CounterHandle>("counter");
    host.closeRegistration(staleRef.registrationId);
    await until(() => instance.publication === "disconnected" ? instance.publication : undefined, "disconnected publication");
    // Disconnect never destroys business: state, handle identity, domain kept.
    expect(instance.state).toBe("active");
    expect(instance.local<CounterHandle>("counter")).toBe(handle);
    expect(instance.module("counter")).toBeUndefined();
    // The client reconnects on its own and republishes the SAME modules with
    // fresh registrationIds; count 3 surviving proves no reconstruction and
    // no mutation replay.
    await until(() => instance.publication === "published" ? instance.module("counter")! : undefined, "republished ref");
    const freshRef = instance.module("counter")!;
    expect(freshRef.registrationId).not.toBe(staleRef.registrationId);
    expect(await client.invoke(freshRef, "read", {}).result).toEqual({ count: 3 });
    // The old reference stays stale: explicit failure, no transparent rebind.
    await expect(client.invoke(staleRef, "read", {}).result).rejects.toMatchObject({ code: -32013 });
    await instance.stop();
    expect(instance.state).toBe("stopped");
    expect(instance.publication).toBe("revoked");
  } finally { host.stop(); }
});

test("stop runs reverse cleanup hooks exactly once; a disconnect never runs them", async () => {
  const host = s2Host();
  try {
    const stopped: string[] = [];
    const instance = new Instance(definePlugin({
      id: "hooks",
      modules: [
        {
          name: "base",
          contract: counterContract,
          registration: { spaceId: "test", providerEndpointId: "hooks-base", integrationId: "integration" },
          handlers: { read: () => ({ count: 0 }), add: () => ({ count: 0 }) },
          states: { value: (_argument, observer, _context, signal) => new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" }).open({}, observer, signal) },
          start: () => "base-handle",
          stop: (_ctx, handle) => { stopped.push(`base:${String(handle)}`); },
        },
        {
          name: "leaf",
          contract: counterContract,
          registration: { spaceId: "test", providerEndpointId: "hooks-leaf", integrationId: "integration" },
          depsLocal: ["base"],
          handlers: { read: () => ({ count: 0 }), add: () => ({ count: 0 }) },
          states: { value: (_argument, observer, _context, signal) => new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" }).open({}, observer, signal) },
          start: () => "leaf-handle",
          stop: async (_ctx, handle) => {
            // Async cleanup: stop must wait for it, bounded, before settling.
            stopped.push(`leaf:${String(handle)}`);
          },
        },
      ],
    }), serviceLink(host.port));
    await instance.start({});
    await until(() => instance.publication === "published" ? true : undefined, "hooks publication");
    expect(stopped).toEqual([]);
    // A link drop does not run cleanup hooks: business survives it.
    host.closeRegistration(instance.module("leaf")!.registrationId);
    await until(() => instance.publication === "disconnected" ? instance.publication : undefined, "disconnected");
    expect(stopped).toEqual([]);
    // Republish on the reconnected link, then explicit stop runs hooks in
    // reverse topology, exactly once each.
    await until(() => instance.publication === "published" ? instance.publication : undefined, "republished");
    await instance.stop();
    expect(stopped).toEqual(["leaf:leaf-handle", "base:base-handle"]);
    // Idempotent second stop: hooks never re-run.
    await instance.stop();
    expect(stopped).toEqual(["leaf:leaf-handle", "base:base-handle"]);
    expect(instance.state).toBe("stopped");
  } finally { host.stop(); }
});

test("local-only startup does not wait for a stalled real WebSocket handshake", async () => {
  const stalled = Bun.serve({
    port: 0,
    fetch(request, server) { if (server.upgrade(request)) return; return new Response(null, { status: 400 }); },
    websocket: { message() {} },
  });
  const instance = new Instance(counterPlugin(), { ...serviceLink(stalled.port!), timeoutMs: 1000 });
  const pending = instance.start({ endpoint: "stalled" });
  const { promise, resolve } = Promise.withResolvers<boolean>();
  // Real network integration: this guard proves local startup is independent of
  // the platform's pending WebSocket handshake, not a guessed ordering sleep.
  const timer = setTimeout(() => resolve(false), 100);
  try {
    const started = await Promise.race([pending.then(() => true), promise]);
    expect(started).toBe(true);
    expect(instance.local<CounterHandle>("counter").read().value.count).toBe(0);
  } finally {
    clearTimeout(timer);
    await stalled.stop(true);
    await pending;
    await instance.stop();
  }
});

test("cleanup failure is reported to every stop waiter instead of claiming stopped", async () => {
  const host = s2Host();
  const module = failingModule("cleanup", { start: () => ({ count: 7 }), stop: () => { throw new Error("cleanup refused"); } });
  const instance = new Instance(definePlugin({ id: "cleanup", modules: [module] }), serviceLink(host.port));
  try {
    await instance.start({});
    const first = instance.stop();
    expect(instance.stop()).toBe(first);
    await expect(first).rejects.toBeInstanceOf(AggregateError);
    expect(instance.state).toBe("failed");
    await expect(instance.stop()).rejects.toBeInstanceOf(AggregateError);
  } finally { host.stop(); }
});

test("a later stop request can shorten but not extend an effective deadline", async () => {
  const host = s2Host();
  const entered = Promise.withResolvers<void>();
  const never = Promise.withResolvers<void>();
  const module = failingModule("slow-cleanup", {
    start: () => ({ count: 1 }),
    stop: () => { entered.resolve(); return never.promise; },
  });
  const instance = new Instance(definePlugin({ id: "slow-cleanup", modules: [module] }), serviceLink(host.port));
  try {
    await instance.start({});
    const stopped = instance.stop({ deadlineMs: 1000 });
    void stopped.catch(() => undefined);
    await entered.promise;
    expect(instance.stop({ deadlineMs: 10 })).toBe(stopped);
    expect(instance.stop({ deadlineMs: 30000 })).toBe(stopped);
    const { promise, resolve } = Promise.withResolvers<"hung">();
    // Exercise the platform clock deadline while a real hub link is active;
    // the cleanup gates establish ordering without sleeps.
    const timer = setTimeout(() => resolve("hung"), 100);
    const result = await Promise.race([stopped.then(() => "stopped", () => "failed"), promise]);
    clearTimeout(timer);
    expect(result).toBe("failed");
    expect(instance.state).toBe("failed");
  } finally { never.resolve(); host.stop(); }
});

test("stop expiry during start still runs a late module's stop hook exactly once", async () => {
  const host = s2Host();
  const releaseStart = Promise.withResolvers<void>();
  const cleaned: string[] = [];
  const slow = failingModule("slow", {
    start: async () => { await releaseStart.promise; return { count: 1 }; },
    stop: () => { cleaned.push("slow"); },
  });
  const instance = new Instance(definePlugin({ id: "race", modules: [slow] }), serviceLink(host.port));
  try {
    // start() suspends inside slow.start; stop expiry tears down around it.
    const starting = instance.start({});
    const stopping = instance.stop({ deadlineMs: 15 });
    void stopping.catch(() => undefined);
    await stopping.catch(() => undefined); // stopActive finished: registries cleared
    releaseStart.resolve();
    await until(() => cleaned.includes("slow") || undefined, "late module cleanup");
    let startRejection: unknown;
    await starting.catch(error => { startRejection = error; });
    expect(cleaned).toEqual(["slow"]); // hook exactly once despite the cleared registry
    expect(instance.state).toBe("failed");
    expect(String((startRejection as AggregateError | undefined)?.message ?? ""))
      .not.toContain("lost its cleanup context");
  } finally { host.stop(); }
});
