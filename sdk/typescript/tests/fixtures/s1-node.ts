#!/usr/bin/env bun
// Test-only process controller for `cargo xtask e2e --suite s1-capabilities`
// (docs/plans/2026-10-17-conex-s1.md T08). All domain traffic goes through
// the real SDK/socket: connect, publish `conex.counter` via the real TS SDK
// (ConexWsClient + AtomicState + ModuleProvider with generated typed
// input/output), then serve JSONL commands on stdin.
//
// Env:
//   CONEX_URL        ws URL incl. path, e.g. ws://127.0.0.1:8788/wss
//   CONEX_TOKEN      bearer token matching host [[tokens]] digest
//   CONEX_SPACE      space id (dev)
//   CONEX_ENDPOINT   own provider endpoint id (counter-ts)
//   CONEX_INTEGRATION optional; default "integration"
//
// STDOUT is JSONL events only; diagnostics go to STDERR. Startup line:
//   {"event":"ready","reference":<ModuleReference>}
// Provider operations emit:
//   {"event":"barrier","point",requestId?,count,revision}
//   {"event":"call_started","callId","requestId","sdkRequestId"}
//   {"event":"snapshot","subscriptionKey","snapshot","subscriptionId"}
//   {"event":"update","subscriptionKey","seq"?,"sample"}
//   {"event":"end","subscriptionKey","error":{code,message,execution,retry}}
//   {"event":"fault_applied","kind","requestId"?,"subscriptionId"?}
//
// Command protocol: one JSON line per request on stdin, response on stdout.
//   -> {"id":string,"op":string,...}
//   <- {"id", "ok":true, "value":...} | {"id", "ok":false,
//      "error":{code,message,execution,retry}}
// The request loop stays live: nothing is awaited so cancel/release/inspect
// keep working while other invokes/handlers are blocked on barriers.
//
// Ops:
//   list      {spaceId?}                         -> ModuleReference[]
//   invoke    {callId,target,method,argument,timeoutMs?,wireRequestId?}
//                                                -> terminal result; emits
//                                                   call_started after the
//                                                   control is registered so
//                                                   cancel can race it
//   cancel    {callId}                           -> {requested}; never
//                                                   settles the invoke itself
//   subscribe {subscriptionKey,target,state:"counter/value",argument:{},
//              timeoutMs?}                       -> {snapshot,subscriptionId};
//                                                   emits update/end events
//   unsubscribe {subscriptionKey}                -> {removed}
//   arm       {point:"before_commit"|"after_commit"|"snapshot"}
//                                                -> {armed}; gates the next
//                                                   relevant provider op
//   release   {point}                            -> {released}
//   mutate    {delta,revision?}                  -> {count,revision}; same
//                                                   atomic commit path as the
//                                                   provider add handler;
//                                                   delta=0 commits and bumps
//                                                   revision; int32 overflow
//                                                   rejects before mutation
//   inspect                                     -> {count,revision,executions,
//                                                   observers}; executions
//                                                   counts committed adds only
//   fault     {kind:"seq_gap"|"revision_rollback"|"malformed_output"|
//              "bad_quality",subscriptionId?}    -> {armed}
//   close                                        -> {closed}; SDK link closed,
//                                                   local inspect still works
//   shutdown                                     -> {closed}; close + exit 0
//
// Faults are applied by this test adapter to the NEXT matching OUTGOING frame
// AFTER the real SDK generated and validated it; the SDK code is untouched:
//   seq_gap            state/update seq += 1 (host expects strict 1..n)
//   revision_rollback  state/update sample.revision := "0"
//   bad_quality        state/update sample.quality := "invalid-quality"
//   malformed_output   provider invoke result count := "invalid-int32"
// `subscriptionId` restricts state-faults to one subscription. Optional
// `invoke.wireRequestId` remaps ONLY the outgoing request id (host sees the
// remapped ULID; responses are remapped back) for cross-source identical-id
// checks. Cancel carries the remapped id too. The fixture never fabricates a
// successful result the host did not produce — faults only corrupt bytes.
import { createInterface } from "node:readline";
import { ConexWsClient, type WebSocketLike } from "../../src/ws-client";
import type { CapabilityContract, Invocation, ModuleProvider, ModuleReference } from "../../src/capability";
import { AtomicState, type StateSubscription } from "../../src/state";
import { ConexError } from "../../src/errors";
import { isUlid, validateDomainSchema } from "../../src/s1-wire";
import type { CounterAddInput, CounterReadInput, CounterResult, CounterStateValue } from "../../src/generated/conex/counter";

function emit(value: unknown): void { console.log(JSON.stringify(value)); }
function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`missing ${name}`);
  return value;
}
function parse<T>(schema: string, value: unknown): T {
  validateDomainSchema(schema, value);
  return value as T;
}
function failure(error: unknown) {
  const value = error instanceof ConexError ? error : new ConexError(error instanceof Error ? error.message : String(error));
  return { code: value.code ?? -32603, message: value.message, execution: value.execution ?? "not_started", retry: value.retry ?? "never" };
}
function cancelled(): ConexError {
  return new ConexError("handler stopped before commit", { code: -32011, execution: "completed", retry: "never" });
}

const contract: CapabilityContract = {
  contractId: "conex.counter",
  methods: {
    "counter/read": {
      execution: "read_only",
      input: value => parse<CounterReadInput>("conex.CounterReadInput", value),
      output: value => parse<CounterResult>("conex.CounterResult", value),
    },
    "counter/add": {
      execution: "non_replayable",
      input: value => parse<Required<CounterAddInput>>("conex.CounterAddInput", value),
      output: value => parse<CounterResult>("conex.CounterResult", value),
    },
  },
  states: {
    "counter/value": {
      input: value => parse<CounterReadInput>("conex.CounterReadInput", value),
      value: value => parse<CounterStateValue>("conex.CounterStateValue", value),
    },
  },
};

const domain = new AtomicState<Required<CounterStateValue>>({ value: { count: 0 }, revision: "0", sampledAtMs: "0", quality: "current" });
let executions = 0;
const observers = new Set<symbol>();
type Point = "before_commit" | "after_commit" | "snapshot";
interface Gate { reached: boolean; promise: Promise<void>; release(): void }
const gates = new Map<Point, Gate>();
function point(value: unknown): Point {
  if (value !== "before_commit" && value !== "after_commit" && value !== "snapshot") throw new Error("unknown barrier point");
  return value;
}
function arm(name: Point): void {
  if (gates.has(name)) throw new Error("barrier already armed");
  let release!: () => void;
  const promise = new Promise<void>(resolve => { release = resolve; });
  gates.set(name, { reached: false, promise, release });
}
async function barrier(name: Point, requestId: string | undefined, signal?: AbortSignal): Promise<void> {
  const gate = gates.get(name);
  if (!gate || gate.reached) return;
  gate.reached = true;
  const current = domain.read();
  emit({ event: "barrier", point: name, requestId, count: current.value.count, revision: current.revision });
  let stop: (() => void) | undefined;
  try {
    if (!signal) { await gate.promise; return; }
    if (signal.aborted) throw cancelled();
    await Promise.race([gate.promise, new Promise<never>((_resolve, reject) => {
      stop = () => reject(cancelled());
      signal.addEventListener("abort", stop, { once: true });
    })]);
  } finally {
    if (stop) signal?.removeEventListener("abort", stop);
    if (gates.get(name) === gate) gates.delete(name);
  }
}
function mutate(delta: number, revision?: string): Required<CounterResult> {
  parse<CounterAddInput>("conex.CounterAddInput", { delta });
  const current = domain.read();
  const count = current.value.count + delta;
  if (count < -2147483648 || count > 2147483647) throw new ConexError("int32 overflow", { code: -32602, execution: "not_started", retry: "never" });
  const nextRevision = revision ?? (BigInt(current.revision) + 1n).toString();
  parse<CounterResult>("conex.CounterResult", { count, revision: nextRevision });
  domain.publish({ value: { count }, revision: nextRevision, sampledAtMs: String(Date.now()), quality: "current" });
  executions += 1;
  return { count, revision: nextRevision };
}

// Only this test adapter can corrupt bytes: SDK parsing/validation is unchanged.
type FaultKind = "seq_gap" | "revision_rollback" | "malformed_output" | "bad_quality";
interface Fault { kind: FaultKind; subscriptionId?: string }
let fault: Fault | undefined;
let nextWireRequestId: string | undefined;
let dispatchRequestId: string | undefined;
const sdkToWire = new Map<string, string>();
const wireToSdk = new Map<string, string>();
const providerInvokes = new Set<string>();
const receivedSeq = new Map<string, string>();
type ServiceSocketCtor = new (target: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
const ServiceSocket = globalThis.WebSocket as unknown as ServiceSocketCtor;
class FaultSocket implements WebSocketLike {
  onopen: WebSocketLike["onopen"] = null;
  onmessage: WebSocketLike["onmessage"] = null;
  onerror: WebSocketLike["onerror"] = null;
  onclose: WebSocketLike["onclose"] = null;
  private readonly socket: WebSocketLike;
  constructor(target: string, headers: Readonly<Record<string, string>>) {
    this.socket = new ServiceSocket(target, { headers });
    this.socket.onopen = () => this.onopen?.();
    this.socket.onerror = () => this.onerror?.();
    this.socket.onclose = () => this.onclose?.();
    this.socket.onmessage = event => {
      if (typeof event.data !== "string") { this.onmessage?.(event); return; }
      const frame = JSON.parse(event.data);
      if (frame.method === "module/invoke") providerInvokes.add(frame.id);
      if (frame.method === "state/update") receivedSeq.set(frame.params.input.subscriptionId, frame.params.input.seq);
      if (!frame.method && wireToSdk.has(frame.id)) {
        const wireId = frame.id;
        frame.id = wireToSdk.get(wireId);
        wireToSdk.delete(wireId);
        sdkToWire.delete(frame.id);
      }
      dispatchRequestId = frame.method ? frame.id : undefined;
      try { this.onmessage?.({ data: JSON.stringify(frame) }); }
      finally { dispatchRequestId = undefined; }
    };
  }
  get bufferedAmount(): number { return this.socket.bufferedAmount ?? 0; }
  close(): void { this.socket.close(); }
  send(text: string): void {
    const frame = JSON.parse(text);
    if (frame.method === "module/invoke" && nextWireRequestId) {
      sdkToWire.set(frame.id, nextWireRequestId);
      wireToSdk.set(nextWireRequestId, frame.id);
      frame.id = nextWireRequestId;
      nextWireRequestId = undefined;
    }
    if (frame.method === "module/cancel") {
      const input = frame.params.input;
      input.requestId = sdkToWire.get(input.requestId) ?? input.requestId;
    }
    const stateMatch = frame.method === "state/update" && (!fault?.subscriptionId || frame.params.input.subscriptionId === fault.subscriptionId);
    const outputMatch = !frame.method && frame.result !== undefined && providerInvokes.has(frame.id);
    let applied: FaultKind | undefined;
    if (fault?.kind === "malformed_output" && outputMatch) {
      frame.result = { ...frame.result, count: "invalid-int32" };
      applied = fault.kind;
    } else if (stateMatch && fault && fault.kind !== "malformed_output") {
      const input = frame.params.input;
      if (fault.kind === "seq_gap") input.seq = (BigInt(input.seq) + 1n).toString();
      if (fault.kind === "revision_rollback") input.sample.revision = "0";
      if (fault.kind === "bad_quality") input.sample.quality = "invalid-quality";
      applied = fault.kind;
    }
    if (!frame.method) providerInvokes.delete(frame.id);
    this.socket.send(JSON.stringify(frame));
    if (applied) {
      fault = undefined;
      emit({ event: "fault_applied", kind: applied, requestId: frame.id, subscriptionId: frame.params?.input?.subscriptionId });
    }
  }
}

const url = new URL(requireEnv("CONEX_URL"));
const token = requireEnv("CONEX_TOKEN");
const spaceId = requireEnv("CONEX_SPACE");
const provider: ModuleProvider = {
  module: {
    spaceId, providerEndpointId: requireEnv("CONEX_ENDPOINT"),
    integrationId: process.env.CONEX_INTEGRATION ?? "integration", contractId: contract.contractId,
  },
  authorize: () => true,
  handlers: {
    "counter/read": (): CounterResult => {
      const current = domain.read();
      return { count: current.value.count, revision: current.revision };
    },
    "counter/add": async (argument, _context, signal): Promise<CounterResult> => {
      const requestId = dispatchRequestId;
      await barrier("before_commit", requestId, signal);
      if (signal.aborted) throw cancelled();
      const result = mutate((argument as Required<CounterAddInput>).delta);
      // The operation is committed. Cancellation cannot turn this into rollback.
      await barrier("after_commit", requestId);
      return result;
    },
  },
  states: {
    "counter/value": async (argument, observer, _context, signal) => {
      const requestId = dispatchRequestId;
      const key = Symbol();
      const opening = domain.open(argument, observer, signal);
      observers.add(key);
      const dispose = () => {
        observers.delete(key);
        opening.dispose();
        signal.removeEventListener("abort", dispose);
      };
      signal.addEventListener("abort", dispose, { once: true });
      try {
        await barrier("snapshot", requestId, signal);
        return { snapshot: opening.snapshot, dispose };
      } catch (error) { dispose(); throw error; }
    },
  },
};
const client = new ConexWsClient({
  origin: url.origin, wsPath: url.pathname, reconnect: false,
  tokenProvider: async () => token,
  createWebSocket: (target, headers) => new FaultSocket(target, headers),
  capabilities: { contracts: [contract], providers: [provider] },
});
const calls = new Map<string, Invocation<CounterResult>>();
const subscriptions = new Map<string, StateSubscription<CounterStateValue>>();
function close(): void {
  client.close();
  for (const gate of gates.values()) gate.release();
  gates.clear();
}

interface Command {
  id: string; op: string; callId?: string; target?: ModuleReference; method?: string; argument?: unknown;
  timeoutMs?: number; subscriptionKey?: string; state?: string; point?: Point; delta?: number;
  spaceId?: string; kind?: FaultKind; subscriptionId?: string; wireRequestId?: string; revision?: string;
}
function required(value: string | undefined, name: string): string {
  if (!value) throw new Error(`missing ${name}`);
  return value;
}
async function run(command: Command): Promise<unknown> {
  switch (command.op) {
    case "list": return client.listModules(command.spaceId ?? spaceId);
    case "invoke": {
      const callId = required(command.callId, "callId");
      if (calls.has(callId)) throw new Error("duplicate callId");
      if (!command.target) throw new Error("missing target");
      if (command.wireRequestId && (!isUlid(command.wireRequestId) || wireToSdk.has(command.wireRequestId))) throw new Error("invalid or active wireRequestId");
      nextWireRequestId = command.wireRequestId;
      let call: Invocation<CounterResult>;
      try { call = client.invoke(command.target, required(command.method, "method"), command.argument, { timeoutMs: command.timeoutMs }); }
      finally { nextWireRequestId = undefined; }
      calls.set(callId, call);
      emit({ event: "call_started", callId, requestId: command.wireRequestId ?? call.requestId, sdkRequestId: call.requestId });
      return call.result;
    }
    case "cancel": {
      const call = calls.get(required(command.callId, "callId"));
      if (!call) throw new Error("unknown callId");
      return call.cancel();
    }
    case "subscribe": {
      const subscriptionKey = required(command.subscriptionKey, "subscriptionKey");
      if (subscriptions.has(subscriptionKey)) throw new Error("duplicate subscriptionKey");
      if (!command.target) throw new Error("missing target");
      const subscription = client.subscribe<unknown, CounterStateValue>(command.target, command.state ?? "counter/value", command.argument ?? {}, sample => {
        emit({ event: "update", subscriptionKey, seq: receivedSeq.get(subscription.subscriptionId), sample });
      }, { timeoutMs: command.timeoutMs });
      subscriptions.set(subscriptionKey, subscription);
      void subscription.ended.then(error => {
        subscriptions.delete(subscriptionKey);
        receivedSeq.delete(subscription.subscriptionId);
        emit({ event: "end", subscriptionKey, error: failure(error) });
      });
      const snapshot = await subscription.snapshot;
      emit({ event: "snapshot", subscriptionKey, snapshot, subscriptionId: subscription.subscriptionId });
      return { snapshot, subscriptionId: subscription.subscriptionId };
    }
    case "unsubscribe": {
      const key = required(command.subscriptionKey, "subscriptionKey");
      return subscriptions.get(key)?.unsubscribe() ?? { removed: false };
    }
    case "arm": arm(point(command.point)); return { armed: true };
    case "release": {
      const name = point(command.point);
      const gate = gates.get(name);
      if (!gate) throw new Error("barrier is not armed");
      gates.delete(name);
      gate.release();
      return { released: true };
    }
    case "mutate": return mutate(command.delta!, command.revision);
    case "inspect": {
      const current = domain.read();
      return { count: current.value.count, revision: current.revision, executions, observers: observers.size };
    }
    case "fault": {
      if (!["seq_gap", "revision_rollback", "malformed_output", "bad_quality"].includes(command.kind ?? "")) throw new Error("unknown fault kind");
      if (fault) throw new Error("fault already armed");
      fault = { kind: command.kind!, subscriptionId: command.subscriptionId };
      return { armed: true };
    }
    case "close": close(); return { closed: true };
    case "shutdown": close(); return { closed: true };
    default: throw new Error("unknown operation");
  }
}

async function main(): Promise<void> {
  process.once("SIGINT", () => { close(); process.exit(0); });
  process.once("SIGTERM", () => { close(); process.exit(0); });
  await client.connect();
  const reference = await client.publishModule(provider.module);
  emit({ event: "ready", reference });
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    if (!line.trim()) continue;
    let command: Command;
    try {
      command = JSON.parse(line);
      if (!command || typeof command.id !== "string" || typeof command.op !== "string") throw new Error("expected command id and op");
    } catch (error) { emit({ id: null, ok: false, error: failure(error) }); continue; }
    // Do not await business operations: cancel/release/inspect must stay live.
    void run(command).then(value => {
      emit({ id: command.id, ok: true, value });
      if (command.op === "shutdown") { lines.close(); process.exit(0); }
    }, error => emit({ id: command.id, ok: false, error: failure(error) }));
  }
  close();
}
void main().catch(error => { close(); console.error(error instanceof Error ? error.message : String(error)); process.exit(1); });
