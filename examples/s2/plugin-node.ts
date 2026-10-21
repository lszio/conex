#!/usr/bin/env bun
/**
 * TS S2 plugin instance (instance B) for the dual-instance plugin sample
 * (docs/contracts/s2-plugins.md §5–§7; plan 2026-10-19-conex-s2.md §6 U03).
 *
 * Same plugin design as crates/conex-sdk/examples/plugin.rs: module `counter`
 * (S1 counter semantics: int32 overflow rejected before mutation, delta=0
 * still advances the revision) plus module `derived-counter` (same contract
 * republished under its own 4-tuple: an ×2 view of its upstream with its own
 * revision sequence, upstream snapshot captured before anything is published).
 *
 * This process runs the derived-counter in REMOTE mode: its upstream is the
 * peer instance's `counter` ModuleRef, discovered via list_modules and fixed
 * as an explicit remote binding before instance start. Every derived read/add
 * crosses the hub; the local-mode branch (deps.local) exists for symmetry and
 * is exercised by the S2 unit tests.
 *
 * Emits structured one-line JSON events; never prints the token.
 *
 * Env:
 *   CONEX_URL            ws://127.0.0.1:PORT/wss
 *   CONEX_TOKEN          bearer matching the host [[tokens]] digest (s2-ts)
 *   CONEX_SPACE          space id (dev)
 *   CONEX_ENDPOINT       own endpoint prefix (s2-ts)
 *   CONEX_PEER_ENDPOINT  peer endpoint prefix (s2-rust)
 */
import { ConexWsClient } from "../../sdk/typescript/src/ws-client";
import type { WebSocketLike } from "../../sdk/typescript/src/ws-client";
import type { CapabilityContract, ModuleReference } from "../../sdk/typescript/src/capability";
import { AtomicState } from "../../sdk/typescript/src/state";
import type { Sample } from "../../sdk/typescript/src/state";
import { ConexError } from "../../sdk/typescript/src/errors";
import { ErrorCode } from "../../sdk/typescript/src/generated/conex/common";
import { Instance, definePlugin } from "../../sdk/typescript/src/plugin";
import type { ModuleDefinition, Plugin } from "../../sdk/typescript/src/plugin";

const CONTRACT_ID = "conex.counter";
const INTEGRATION_ID = "integration";
const I32_MIN = -2147483648;
const I32_MAX = 2147483647;

interface CounterValue { count: number }
interface CounterResult { count: number; revision: string }

/** Service handle the counter module exposes to same-instance dependents. */
interface CounterHandle {
  read(): Sample<CounterValue>;
  add(delta: number): CounterResult;
  observe(update: (sample: Sample<CounterValue>) => void): () => void;
}

interface InstanceGraphConfig {
  endpointPrefix: string;
  upstream: { mode: "local" } | { mode: "remote"; reference: ModuleReference };
}

// ---- env -----------------------------------------------------------------

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    console.error(`missing ${name}`);
    process.exit(2);
  }
  return value;
}

const CONEX_URL = requireEnv("CONEX_URL");
const CONEX_TOKEN = requireEnv("CONEX_TOKEN");
const SPACE = requireEnv("CONEX_SPACE");
const ENDPOINT = requireEnv("CONEX_ENDPOINT");
const PEER = requireEnv("CONEX_PEER_ENDPOINT");

function emit(event: string, fields: Record<string, unknown> = {}): void {
  console.log(JSON.stringify({ event, node: ENDPOINT, ...fields }));
}

// ---- domain contract checks ----------------------------------------------

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("expected object");
  return value as Record<string, unknown>;
}
function int32(value: unknown, what: string): number {
  if (typeof value !== "number" || !Number.isInteger(value)) throw new Error(`${what} must be int32`);
  return value;
}
function uint64String(value: unknown, what: string): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value)) throw new Error(`${what} must be a canonical uint64 string`);
  return value;
}
function counterResult(value: unknown): CounterResult {
  const result = object(value);
  return { count: int32(result.count, "count"), revision: uint64String(result.revision, "revision") };
}
function countValue(value: unknown): CounterValue {
  return { count: int32(object(value).count, "count") };
}

const contract: CapabilityContract = {
  contractId: CONTRACT_ID,
  methods: {
    "counter/read": { execution: "read_only", input: object, output: counterResult },
    "counter/add": {
      execution: "non_replayable",
      input(value) {
        const argument = object(value);
        const delta = int32(argument.delta, "delta");
        if (delta < I32_MIN || delta > I32_MAX) throw new Error("delta out of int32 range");
        return { delta };
      },
      output: counterResult,
    },
  },
  states: { "counter/value": { input: object, value: countValue } },
};

// ---- plugin class: counter + derived-counter ------------------------------

/** ×2 view with the derived's own int32 ceiling; never silently wraps. */
function doubled(count: number): number {
  const value = count * 2;
  if (value < I32_MIN || value > I32_MAX) {
    throw new ConexError("derived ×2 view overflows int32", { code: ErrorCode.ERROR_CODE_BAD_REQUEST, execution: "not_started", retry: "never" });
  }
  return value;
}

function counterPlugin(): Plugin<InstanceGraphConfig> {
  return definePlugin<InstanceGraphConfig>({
    id: "counter-plugin",
    graph(config) {
      const registration = (suffix: string) => ({
        spaceId: SPACE,
        providerEndpointId: `${config.endpointPrefix}${suffix}`,
        integrationId: INTEGRATION_ID,
      });
      const remote = config.upstream.mode === "remote";
      // Narrowed here so the binding reference needs no cast below.
      const upstreamBinding = config.upstream.mode === "remote" ? { upstream: config.upstream.reference } : undefined;
      // Per-graph domains: two instances of this plugin class never share state.
      const counter = new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: String(Date.now()), quality: "current" });
      const derived = new AtomicState<CounterValue>({ value: { count: 0 }, revision: "0", sampledAtMs: String(Date.now()), quality: "current" });
      let lastUpstreamRevision = -1n;
      let forwardAdd: ((delta: number) => Promise<CounterResult>) | undefined;

      const commitCounter = (count: number): CounterResult => {
        const revision = (BigInt(counter.read().revision) + 1n).toString();
        counter.publish({ value: { count }, revision, sampledAtMs: String(Date.now()), quality: "current" });
        return { count, revision };
      };
      const addCounter = (delta: number): CounterResult => {
        const next = counter.read().value.count + delta;
        if (next < I32_MIN || next > I32_MAX) {
          // Reject before the mutation: count and revision stay unchanged.
          throw new ConexError("int32 overflow", { code: ErrorCode.ERROR_CODE_BAD_REQUEST, execution: "not_started", retry: "never" });
        }
        return commitCounter(next);
      };
      const advanceDerived = (count: number): string => {
        const revision = (BigInt(derived.read().revision) + 1n).toString();
        derived.publish({ value: { count }, revision, sampledAtMs: String(Date.now()), quality: "current" });
        return revision;
      };
      const onUpstream = (upstreamRevision: bigint, upstreamCount: number): void => {
        if (upstreamRevision <= lastUpstreamRevision) return;
        try {
          lastUpstreamRevision = upstreamRevision;
          advanceDerived(doubled(upstreamCount));
        } catch (error) {
          emit("error", { message: error instanceof Error ? error.message : String(error) });
        }
      };

      const readView = (state: AtomicState<CounterValue>): CounterResult => {
        const sample = state.read();
        return { count: sample.value.count, revision: sample.revision.toString() };
      };

      const modules: ModuleDefinition[] = [
        {
          name: "counter",
          contract,
          registration: registration("-counter"),
          handlers: {
            "counter/read": () => readView(counter),
            "counter/add": raw => addCounter(int32(object(raw).delta, "delta")),
          },
          states: { "counter/value": (_argument, observer, _context, signal) => counter.open({}, observer, signal) },
          start: (): CounterHandle => ({
            read: () => counter.read(),
            add: addCounter,
            observe(update) {
              const controller = new AbortController();
              counter.open({}, { update, end: () => undefined }, controller.signal);
              return () => controller.abort();
            },
          }),
        },
        {
          name: "derived-counter",
          contract,
          registration: registration("-derived-counter"),
          depsLocal: remote ? [] : ["counter"],
          depsRemote: remote ? ["upstream"] : [],
          handlers: {
            "counter/read": () => readView(derived),
            "counter/add": async raw => {
              // add is forwarded to the upstream counter; the ×2 view follows
              // via the upstream observation. Local mode is a process-local
              // call (zero frames); remote mode goes through the binding.
              const delta = int32(object(raw).delta, "delta");
              if (!forwardAdd) throw new ConexError("derived upstream not started", { code: ErrorCode.ERROR_CODE_INTERNAL, execution: "not_started", retry: "never" });
              return forwardAdd(delta);
            },
          },
          states: { "counter/value": (_argument, observer, _context, signal) => derived.open({}, observer, signal) },
          start: async ctx => {
            // 先快照后发布: the upstream snapshot is captured and committed
            // before this module serves any state subscription; the derived
            // revision sequence starts independently at 1.
            if (remote) {
              const upstream = ctx.deps.remote("upstream");
              const initial = counterResult(await upstream.invoke<unknown, unknown>("counter/read", {}).result);
              emit("bind-snapshot", { target: upstream.reference.providerEndpointId, count: initial.count, revision: initial.revision });
              advanceDerived(doubled(initial.count));
              lastUpstreamRevision = BigInt(initial.revision);
              upstream.subscribe<unknown, CounterValue>("counter/value", {}, sample => onUpstream(BigInt(sample.revision), sample.value.count));
              forwardAdd = delta => upstream.invoke<unknown, CounterResult>("counter/add", { delta }).result;
              return;
            }
            const upstream = ctx.deps.local<CounterHandle>("counter");
            const initial = upstream.read();
            emit("bind-snapshot", { target: `${config.endpointPrefix}-counter`, count: initial.value.count, revision: initial.revision.toString(), mode: "local" });
            advanceDerived(doubled(initial.value.count));
            lastUpstreamRevision = BigInt(initial.revision);
            upstream.observe(sample => onUpstream(BigInt(sample.revision), sample.value.count));
            forwardAdd = delta => Promise.resolve(upstream.add(delta));
          },
        },
      ];
      return { modules, remotes: upstreamBinding };
    },
  });
}

// ---- client ---------------------------------------------------------------

type ServiceSocketCtor = new (target: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
// Bun's WebSocket accepts a headers options object that the DOM type omits;
// the SDK contract (WebSocketLike) is what we actually implement.
const ServiceSocket = globalThis.WebSocket as unknown as ServiceSocketCtor;

const link = {
  origin: new URL(CONEX_URL).origin,
  tokenProvider: async () => CONEX_TOKEN,
  createWebSocket: (target: string, headers: Readonly<Record<string, string>>) => new ServiceSocket(target, { headers }),
};

const consumer = new ConexWsClient({ ...link, capabilities: { contracts: [contract] } });

function sleep(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

async function discover(endpoint: string, timeoutMs = 10_000): Promise<ModuleReference> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const modules = await consumer.listModules(SPACE);
    const found = modules.find(module => module.providerEndpointId === endpoint && module.contractId === CONTRACT_ID);
    if (found) return found;
    await sleep(200);
  }
  throw new Error(`endpoint ${endpoint} not registered within ${timeoutMs}ms`);
}

async function invokeOn<T>(target: ModuleReference, method: string, argument: unknown): Promise<T> {
  return consumer.invoke<unknown, T>(target, method, argument).result;
}

/** Polls a counter read until `count >= min`; the cross-process phase gate. */
async function untilCount(target: ModuleReference, min: number, what: string): Promise<CounterResult> {
  const deadline = Date.now() + 15_000;
  let last: CounterResult | undefined;
  while (Date.now() < deadline) {
    try {
      last = counterResult(await invokeOn<unknown>(target, "counter/read", {}));
      if (last.count >= min) return last;
    } catch { last = undefined; }
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${what}: last ${JSON.stringify(last)}`);
}

/** Update feed for one subscription: emits every sample, awaits thresholds. */
function feed(target: string) {
  const queued: Sample<CounterValue>[] = [];
  const takers: Array<(sample: Sample<CounterValue>) => void> = [];
  return {
    on(sample: Sample<CounterValue>): void {
      emit("update", { target, revision: sample.revision.toString(), count: sample.value.count });
      const taker = takers.shift();
      if (taker) taker(sample);
      else queued.push(sample);
    },
    async awaitCount(start: Sample<CounterValue>, min: number): Promise<Sample<CounterValue>> {
      let current = start;
      const deadline = Date.now() + 15_000;
      while (current.value.count < min) {
        if (Date.now() >= deadline) throw new Error(`timed out waiting for ${target} count >= ${min}`);
        current = await new Promise<Sample<CounterValue>>((resolve, reject) => {
          const ready = queued.shift();
          if (ready) { resolve(ready); return; }
          const timer = setTimeout(() => reject(new Error(`timed out waiting for ${target} count >= ${min}`)), deadline - Date.now());
          takers.push(sample => { clearTimeout(timer); resolve(sample); });
        });
      }
      return current;
    },
  };
}

// ---- main -----------------------------------------------------------------

async function main(): Promise<void> {
  const shutdown = Promise.withResolvers<void>();
  process.once("SIGINT", shutdown.resolve);
  process.once("SIGTERM", shutdown.resolve);
  emit("start", { space: SPACE, endpointPrefix: ENDPOINT, peerPrefix: PEER, derivedMode: "remote" });
  await consumer.connect();
  emit("ready");

  // Discover the peer instance's counter and fix the remote binding BEFORE
  // the instance starts (composition validates the binding list up front).
  const upstream = await discover(`${PEER}-counter`);
  emit("discover", { target: upstream.providerEndpointId, registrationId: upstream.registrationId });

  const instance = new Instance(counterPlugin(), link);
  await instance.start({ endpointPrefix: ENDPOINT, upstream: { mode: "remote", reference: upstream } });
  emit("publish", { providerEndpointId: `${ENDPOINT}-counter`, registrationId: instance.module("counter")!.registrationId });
  emit("publish", { providerEndpointId: `${ENDPOINT}-derived-counter`, registrationId: instance.module("derived-counter")!.registrationId });

  // ---- A03: cross-instance binding through this instance's derived-counter --
  const derivedRef = instance.module("derived-counter")!;
  const derivedFeed = feed(derivedRef.providerEndpointId);
  const derivedSub = consumer.subscribe<Record<string, never>, CounterValue>(derivedRef, "counter/value", {}, sample => derivedFeed.on(sample));
  const derivedSnapshot = await derivedSub.snapshot;
  emit("snapshot", { target: derivedRef.providerEndpointId, revision: derivedSnapshot.revision.toString(), count: derivedSnapshot.value.count });

  await untilCount(upstream, 5, "rust instance add(+5)");
  const mirrored = await derivedFeed.awaitCount(derivedSnapshot, 10);
  emit("bind-view", { target: derivedRef.providerEndpointId, count: mirrored.value.count, revision: mirrored.revision.toString() });

  const forwarded = counterResult(await invokeOn<unknown>(derivedRef, "counter/add", { delta: 3 }));
  emit("add", { target: derivedRef.providerEndpointId, delta: 3, forwardedCount: forwarded.count, forwardedRevision: forwarded.revision });
  const upstreamAfter = counterResult(await invokeOn<unknown>(upstream, "counter/read", {}));
  emit("read", { target: upstream.providerEndpointId, count: upstreamAfter.count, revision: upstreamAfter.revision });
  await derivedFeed.awaitCount(mirrored, 16);
  const derivedNow = counterResult(await invokeOn<unknown>(derivedRef, "counter/read", {}));
  emit("read", { target: derivedRef.providerEndpointId, count: derivedNow.count, revision: derivedNow.revision });
  emit("phase-done", { phase: "A03-remote-binding" });

  // ---- A07: TS consumes the Rust instance's published derived module --------
  const peerDerived = await discover(`${PEER}-derived-counter`);
  await untilCount(upstream, 12, "rust derived-consumer add(+4)");
  const peerFeed = feed(peerDerived.providerEndpointId);
  const peerSub = consumer.subscribe<Record<string, never>, CounterValue>(peerDerived, "counter/value", {}, sample => peerFeed.on(sample));
  const peerSnapshot = await peerSub.snapshot;
  emit("snapshot", { target: peerDerived.providerEndpointId, revision: peerSnapshot.revision.toString(), count: peerSnapshot.value.count });

  const added = counterResult(await invokeOn<unknown>(upstream, "counter/add", { delta: 2 }));
  emit("add", { target: upstream.providerEndpointId, delta: 2, count: added.count, revision: added.revision });
  const seen = await peerFeed.awaitCount(peerSnapshot, 28);
  emit("update-seen", { target: peerDerived.providerEndpointId, revision: seen.revision.toString(), count: seen.value.count });
  const finalView = counterResult(await invokeOn<unknown>(peerDerived, "counter/read", {}));
  emit("read", { target: peerDerived.providerEndpointId, count: finalView.count, revision: finalView.revision });
  emit("phase-done", { phase: "A07-ts-consumes-rust-derived" });

  emit("idle");
  await shutdown.promise;

  emit("stopping");
  await instance.stop();
  emit("stopped", { state: instance.state });
  consumer.close();
}

main().catch(error => {
  emit("error", { message: error instanceof Error ? error.message : String(error) });
  process.exit(1);
});
