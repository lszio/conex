// S2 plugin runtime acceptance (docs/contracts/s2-plugins.md §8, plan §4 U01):
// S2-A01 dual instances, A02 local DI with zero broker frames, A04 missing
// dependency rejection, A05 cycle rejection, A06 stop release, A08 broken
// links do not revive. Runs against the in-process S1 broker fixture
// (fixtures/s2-host.ts) so every module start/publish/stop crosses a real
// WebSocket connection with real wire frames.
import { expect, test } from "bun:test";
import { ConexWsClient } from "../src/ws-client";
import type { WebSocketLike } from "../src/ws-client";
import type { CapabilityContract, ModuleReference } from "../src/capability";
import { AtomicState } from "../src/state";
import type { Sample } from "../src/state";
import { Instance, PluginCompositeError, definePlugin } from "../src/plugin";
import type { InstanceLink, ModuleDefinition, Plugin } from "../src/plugin";
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
    expect(a.state).toBe("active");
    expect(b.state).toBe("active");
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
  } finally { host.stop(); }
});

test("S2-A02 derived module consumes its local dependency in-process with zero broker frames", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(counterPlugin(), serviceLink(host.port));
    await instance.start({ endpoint: "local" });
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
  } finally { host.stop(); }
});

test("an explicit remote binding invokes and subscribes to another instance's module", async () => {
  const host = s2Host();
  try {
    const source = new Instance(counterPlugin(), serviceLink(host.port));
    await source.start({ endpoint: "src" });
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
    const client = await consumer(host.port);
    expect(await client.invoke(mirrorInstance.module("mirror")!, "read", {}).result).toEqual({ count: 0 });
    expect(mirrorSnapshot).toMatchObject({ value: { count: 0 } });
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
    expect(endpointsOf(host, "register")).toEqual([]);
    // The half-open link was released, not left dangling.
    expect(await until(() => host.events.some(event => event.kind === "disconnect") || undefined, "link close")).toBe(true);
  } finally { host.stop(); }
});

test("a failed publish unwinds already-published modules", async () => {
  const host = s2Host();
  try {
    // Both modules share (spaceId, providerEndpointId): the second publish is
    // refused locally by the S1 session's already-published guard.
    const registration = { spaceId: "test", providerEndpointId: "collision", integrationId: "integration" };
    const instance = new Instance(definePlugin({
      id: "collision",
      modules: [failingModule("first", { start: () => undefined, registration }), failingModule("second", { depsLocal: ["first"], start: () => undefined, registration })],
    }), serviceLink(host.port));
    await expect(instance.start({})).rejects.toMatchObject({ code: -32014 });
    expect(instance.state).toBe("failed");
    expect(endpointsOf(host, "unregister")).toEqual(["collision"]);
    expect(host.references()).toHaveLength(0);
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
    const doomedCounter = doomed.module("counter")!;
    const doomedDerived = doomed.module("derived")!;
    const survivorCounter = survivor.module("counter")!;
    const client = await consumer(host.port);
    const subscription = client.subscribe<Record<string, never>, CounterValue>(doomedDerived, "value", {}, () => undefined);
    await subscription.snapshot;
    await doomed.stop();
    expect(doomed.state).toBe("stopped");
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
  } finally { host.stop(); }
});

test("stop aborts an in-flight provider handler and waits for it to settle", async () => {
  const host = s2Host();
  try {
    let handlerSettled = false;
    // Real timer on purpose: the handler settles strictly after stop aborts
    // it, so a stop() that did not wait for handler tasks would resolve first
    // and leave handlerSettled false at the assertion below.
    const instance = new Instance(definePlugin<{ endpoint: string }>({
      id: "blocked",
      graph: config => ({
        modules: [{
          name: "counter",
          contract: counterContract,
          registration: { spaceId: "test", providerEndpointId: config.endpoint, integrationId: "integration" },
          handlers: {
            add: (_raw, _context, signal) => {
              const { promise, resolve } = Promise.withResolvers<{ count: number }>();
              signal.addEventListener("abort", () => setTimeout(() => { handlerSettled = true; resolve({ count: 1 }); }, 20), { once: true });
              return promise;
            },
            read: () => ({ count: 0 }),
          },
          states: { value: (_argument, observer, _context, signal) => new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" }).open({}, observer, signal) },
          start: () => undefined,
        }],
      }),
    }), serviceLink(host.port));
    await instance.start({ endpoint: "blocked" });
    const client = await consumer(host.port);
    const call = client.invoke(instance.module("counter")!, "add", { delta: 1 });
    await instance.stop();
    expect(handlerSettled).toBe(true);
    expect(instance.state).toBe("stopped");
    // The consumer's own invoke settles later (its reply cannot reach a closed
    // link); handled, not awaited — the runtime release is what we assert.
    void call.result.catch(() => undefined);
  } finally { host.stop(); }
});

test("S2-A08 a broken link does not revive; a new instance gets a fresh generation and old references go stale", async () => {
  const host = s2Host();
  try {
    const instance = new Instance(counterPlugin(), serviceLink(host.port));
    await instance.start({ endpoint: "counter-a" });
    const staleRef = instance.module("counter")!;
    host.closeRegistration(staleRef.registrationId);
    expect(await until(() => instance.state === "broken" ? instance.state : undefined, "broken state")).toBe("broken");
    expect(host.references()).toHaveLength(0);
    // Real wait on purpose: this spans the client's own RECONNECT_DELAY_MS
    // (100ms) reconnect window, which only exists on the platform clock.
    await new Promise(resolve => setTimeout(resolve, 150));
    expect(instance.state).toBe("broken");
    expect(endpointsOf(host, "register")).toEqual(["counter-a", "counter-a.derived"]);
    // A rebuilt instance is a new generation: fresh registration, old refs stale.
    const rebuilt = new Instance(counterPlugin(), serviceLink(host.port));
    await rebuilt.start({ endpoint: "counter-a" });
    const freshRef = rebuilt.module("counter")!;
    expect(freshRef.registrationId).not.toBe(staleRef.registrationId);
    const client = await consumer(host.port);
    await expect(client.invoke(staleRef, "read", {}).result).rejects.toMatchObject({ code: -32013 });
    expect(await client.invoke(freshRef, "read", {}).result).toEqual({ count: 0 });
    await rebuilt.stop();
    expect(rebuilt.state).toBe("stopped");
  } finally { host.stop(); }
});
