#!/usr/bin/env bun
/**
 * Real TS counter node for the S1 two-process sample
 * (docs/plans/2026-10-17-conex-s1.md T07; docs/contracts/s1-capabilities.md).
 *
 * Provides `conex.counter` (counter/read, counter/add, state counter/value)
 * at CONEX_ENDPOINT and consumes the peer at CONEX_PEER_ENDPOINT over a
 * single connection. Emits structured one-line JSON events on stdout for
 * read/add/snapshot/update/termination; never prints the token.
 *
 * Env:
 *   CONEX_URL         ws://127.0.0.1:PORT/wss
 *   CONEX_TOKEN       bearer matching the host [[tokens]] digest
 *   CONEX_SPACE       space id (dev)
 *   CONEX_ENDPOINT    own provider endpoint id (counter-ts)
 *   CONEX_PEER_ENDPOINT  peer endpoint id to consume (counter-rust)
 */
import { ConexWsClient } from "../../sdk/typescript/src/ws-client";
import type { WebSocketLike } from "../../sdk/typescript/src/ws-client";
import type { CapabilityContract, ModuleProvider, ModuleReference } from "../../sdk/typescript/src/capability";
import { AtomicState } from "../../sdk/typescript/src/state";
import type { Sample } from "../../sdk/typescript/src/state";
import { ConexError } from "../../sdk/typescript/src/errors";
import { ErrorCode } from "../../sdk/typescript/src/generated/conex/common";

const CONTRACT_ID = "conex.counter";
const INTEGRATION_ID = "integration";
const I32_MIN = -2147483648;
const I32_MAX = 2147483647;

interface CounterResult { count: number; revision: string }

function emit(event: string, fields: Record<string, unknown> = {}): void {
  console.log(JSON.stringify({ event, node: ENDPOINT, ...fields }));
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
const PEER_ENDPOINT = requireEnv("CONEX_PEER_ENDPOINT");

// ---- domain contract checks ----------------------------------------------

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("expected object");
  return value as Record<string, unknown>;
}
function int32(value: unknown, what: string): number {
  if (!Number.isInteger(value)) throw new Error(`${what} must be int32`);
  return value as number;
}
function uint64String(value: unknown, what: string): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value)) throw new Error(`${what} must be a canonical uint64 string`);
  return value;
}
function counterResult(value: unknown): CounterResult {
  const result = object(value);
  return { count: int32(result.count, "count"), revision: uint64String(result.revision, "revision") };
}

const contract: CapabilityContract = {
  contractId: CONTRACT_ID,
  methods: {
    "counter/read": {
      execution: "read_only",
      input: object,
      output: counterResult,
    },
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
  states: {
    "counter/value": {
      input: object,
      value(value) { return { count: int32(object(value).count, "count") }; },
    },
  },
};

// ---- domain: mutation + snapshot + observer edge in one synchronous turn ----

const domain = new AtomicState<{ count: number }>({
  value: { count: 0 },
  revision: "0",
  sampledAtMs: String(Date.now()),
  quality: "current",
});

const provider: ModuleProvider = {
  module: { spaceId: SPACE, integrationId: INTEGRATION_ID, providerEndpointId: ENDPOINT, contractId: CONTRACT_ID },
  handlers: {
    "counter/read": async () => {
      const current = domain.read();
      return { count: current.value.count, revision: current.revision };
    },
    "counter/add": async (argument) => {
      const delta = (object(argument).delta as number);
      const current = domain.read();
      const next = current.value.count + delta;
      if (next < I32_MIN || next > I32_MAX) {
        // Reject before the mutation: count and revision stay unchanged.
        throw new ConexError("int32 overflow", { code: ErrorCode.ERROR_CODE_BAD_REQUEST, execution: "not_started", retry: "never" });
      }
      const revision = (BigInt(current.revision) + 1n).toString();
      // publish() validates and broadcasts to every observer synchronously;
      // a blocked/slow subscriber is ended by the SDK, never rolls back commit.
      domain.publish({ value: { count: next }, revision, sampledAtMs: String(Date.now()), quality: "current" });
      return { count: next, revision };
    },
  },
  states: {
    "counter/value": (argument, observer, _context, signal) => domain.open(argument, observer, signal),
  },
  // Consumption is statically approved in host.toml; the module itself is the
  // local exposure ceiling for the two approved service principals.
  authorize: () => true,
};

// ---- client --------------------------------------------------------------

const url = new URL(CONEX_URL);
// Bun's WebSocket accepts an options object with headers; DOM declarations only
// expose protocols, hence the adapter cast.
type ServiceSocketCtor = new (target: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
const ServiceSocket = globalThis.WebSocket as unknown as ServiceSocketCtor;

const client = new ConexWsClient({
  origin: url.origin,
  capabilities: { contracts: [contract], providers: [provider] },
  tokenProvider: async () => CONEX_TOKEN,
  createWebSocket: (target, headers) => new ServiceSocket(target, { headers }),
});

function sleep(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

async function findPeer(timeoutMs: number): Promise<ModuleReference> {
  const deadline = Date.now() + timeoutMs;
  let last: unknown = null;
  while (Date.now() < deadline) {
    try {
      const modules = await client.listModules(SPACE);
      const peer = modules.find(m => m.providerEndpointId === PEER_ENDPOINT && m.contractId === CONTRACT_ID);
      if (peer) return peer;
    } catch (error) { last = error; }
    await sleep(500);
  }
  throw last instanceof Error ? last : new Error(`peer ${PEER_ENDPOINT} not registered within ${timeoutMs}ms`);
}

// ---- main ----------------------------------------------------------------

async function main(): Promise<void> {
  const shutdown = Promise.withResolvers<void>();
  process.once("SIGINT", shutdown.resolve);
  process.once("SIGTERM", shutdown.resolve);
  emit("start", { space: SPACE, endpoint: ENDPOINT, peer: PEER_ENDPOINT });
  await client.connect();
  emit("ready");

  const own = await client.publishModule(provider.module);
  emit("publish", { registrationId: own.registrationId, providerEndpointId: own.providerEndpointId });
  const peer = await findPeer(10_000);
  emit("discover", { target: peer.providerEndpointId, registrationId: peer.registrationId });

  // Subscribe before mutating so snapshot + updates are both observed.
  const queued: Sample<{ count: number }>[] = [];
  const waiters: Array<(sample: Sample<{ count: number }>) => void> = [];
  const subscription = client.subscribe<Record<string, never>, { count: number }>(peer, "counter/value", {}, (sample) => {
    const current = sample.value.count;
    emit("update", { target: PEER_ENDPOINT, revision: sample.revision, count: current });
    const waiter = waiters.shift();
    if (waiter) waiter(sample); else queued.push(sample);
  });

  const snapshot = await subscription.snapshot;
  emit("snapshot", { target: PEER_ENDPOINT, revision: snapshot.revision, count: snapshot.value.count });

  const awaitUpdate = (timeoutMs: number): Promise<Sample<{ count: number }>> => {
    const ready = queued.shift();
    if (ready) return Promise.resolve(ready);
    const { promise, resolve, reject } = Promise.withResolvers<Sample<{ count: number }>>();
    const timer = setTimeout(() => reject(new Error("update timeout")), timeoutMs);
    waiters.push(sample => { clearTimeout(timer); resolve(sample); });
    return promise;
  };

  const invoke = async (method: string, argument: unknown): Promise<CounterResult> =>
    await client.invoke<unknown, CounterResult>(peer, method, argument).result;

  const read0 = await invoke("counter/read", {});
  emit("read", { target: PEER_ENDPOINT, count: read0.count, revision: read0.revision });

  const added = await invoke("counter/add", { delta: 3 });
  emit("add", { target: PEER_ENDPOINT, delta: 3, count: added.count, revision: added.revision });
  const update1 = await awaitUpdate(5_000);
  emit("update-seen", { target: PEER_ENDPOINT, revision: update1.revision, count: update1.value.count });

  const read1 = await invoke("counter/read", {});
  emit("read", { target: PEER_ENDPOINT, count: read1.count, revision: read1.revision });

  // delta = 0 still advances the revision and publishes a new sample.
  const zero = await invoke("counter/add", { delta: 0 });
  emit("add", { target: PEER_ENDPOINT, delta: 0, count: zero.count, revision: zero.revision });
  const update2 = await awaitUpdate(5_000);
  emit("update-seen", { target: PEER_ENDPOINT, revision: update2.revision, count: update2.value.count });

  // int32 overflow must be rejected before any mutation: revision unchanged,
  // and no state update is published.
  try {
    await invoke("counter/add", { delta: I32_MAX });
    throw new Error("overflow add unexpectedly succeeded");
  } catch (error) {
    if (error instanceof Error && error.message === "overflow add unexpectedly succeeded") throw error;
    const fields = error instanceof ConexError
      ? { code: error.code, execution: error.execution, retry: error.retry, message: error.message }
      : { message: String(error) };
    emit("overflow-rejected", fields);
  }

  const read2 = await invoke("counter/read", {});
  emit("read", { target: PEER_ENDPOINT, count: read2.count, revision: read2.revision });

  const removed = await subscription.unsubscribe();
  emit("termination", { target: PEER_ENDPOINT, removed: removed.removed });
  emit("done");
  // Consumer completion does not withdraw this node's published provider.
  await shutdown.promise;
  client.close();
  process.off("SIGINT", shutdown.resolve);
  process.off("SIGTERM", shutdown.resolve);
}

main().catch((error) => {
  emit("error", { message: error instanceof Error ? error.message : String(error) });
  process.exit(1);
});
