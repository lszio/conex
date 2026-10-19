import { expect, test } from "bun:test";
import type { ServerWebSocket } from "bun";
import { ConexWsClient } from "../src/ws-client";
import type { WebSocketLike } from "../src/ws-client";
import type { CapabilityContract, ModuleReference } from "../src/capability";
import { CapabilitySession, CONSUMER_METHODS } from "../src/capability";
import type { ModuleProvider } from "../src/capability";
import { ConexError } from "../src/errors";
import { mintUlid } from "../src/client";
import { AtomicState } from "../src/state";
import type { Sample, StateObserver } from "../src/state";
import { decodeS1Frame, stateEndInput, validateFrame } from "../src/s1-wire";
import { ErrorCode } from "../src/generated/conex/common";

function barrier() {
  let release!: () => void;
  const wait = new Promise<void>(resolve => { release = resolve; });
  return { wait, release };
}
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("expected object");
  return value as Record<string, unknown>;
}
const contract: CapabilityContract = {
  contractId: "test.mutable",
  methods: {
    add: {
      execution: "non_replayable",
      input(value) { const argument = object(value); if (!Number.isInteger(argument.delta)) throw new Error("invalid delta"); return argument; },
      output(value) { const result = object(value); if (!Number.isInteger(result.count)) throw new Error("invalid count"); return result; },
    },
    read: {
      execution: "read_only", input: object,
      output(value) { const result = object(value); if (!Number.isInteger(result.count)) throw new Error("invalid count"); return result; },
    },
  },
  states: { value: { input: object, value(value) { if (!Number.isInteger(value)) throw new Error("invalid state"); return value; } } },
};
const limits = { maxFrameBytes: 262144, maxInflight: 8, maxQueuedBytes: 1048576, timeoutMs: 3000, maxSubscriptions: 8, maxModules: 8 };

/** A real WS peer owns mutable domain state, not a forwarding echo. */
async function peer() {
  const reference: ModuleReference = { spaceId: "test", providerEndpointId: "mutable", registrationId: mintUlid(), integrationId: "test", contractId: contract.contractId, methods: ["add", "read"], states: ["value"] };
  const domain = new AtomicState<number>({ value: 0, revision: "9007199254740993", sampledAtMs: "0", quality: "current" });
  let count = 0;
  let additions = 0;
  let connections = 0;
  let socket!: ServerWebSocket<undefined>;
  let hold = false;
  const committed = barrier();
  const resume = barrier();
  const subscribed = new Map<string, { seq: bigint; dispose(): void; context: unknown }>();
  const methods = ["module/list", "module/invoke", "module/cancel", "state/subscribe", "state/unsubscribe"];
  const bindingId = mintUlid();
  const linkIdentity = { linkId: mintUlid(), peerId: "host", tenantId: "test" };
  let helloProvides: unknown;
  let readyProvides: unknown;
  const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    fetch(request, server) { if (server.upgrade(request)) return; return new Response("upgrade required", { status: 400 }); },
    websocket: {
      open(ws) { socket = ws; connections++; },
      async message(ws, data) {
        const frame = JSON.parse(String(data)) as { id: string; method: string; params: { input: Record<string, unknown>; context: unknown } };
        const reply = (result: unknown) => ws.send(JSON.stringify({ jsonrpc: "2.0", id: frame.id, result }));
        if (frame.method === "conex/hello") {
          expect(frame.params.context).toEqual({ plane: "broker" });
          expect(frame.params.input.plane).toBe("PLANE_BROKER");
          helloProvides = frame.params.input.provides ?? [];
          reply({ bindingId, expiresInMs: 0, profileId: "conex-jsonrpc2-wss", plane: "PLANE_BROKER", provides: methods, limits });
          return;
        }
        if (frame.method === "conex/ready") {
          expect(frame.params.input.negotiationId).toBe(bindingId);
          expect(frame.params.input.plane).toBe("PLANE_BROKER");
          readyProvides = frame.params.input.provides ?? [];
          expect(readyProvides).toEqual(helloProvides);
          reply({ negotiationId: bindingId, profileId: "conex-jsonrpc2-wss", plane: "PLANE_BROKER", provides: methods, limits, linkIdentity });
          return;
        }
        const input = frame.params.input;
        if (frame.method === "module/list") { reply({ modules: [reference] }); return; }
        if (frame.method === "module/cancel") { reply({ requested: true }); return; }
        if (frame.method === "state/unsubscribe") {
          const subscriptionId = String(input.subscriptionId);
          const subscription = subscribed.get(subscriptionId);
          subscription?.dispose();
          reply({ removed: subscribed.delete(subscriptionId) });
          return;
        }
        if (frame.method === "state/subscribe") {
          const subscriptionId = String(input.subscriptionId);
          const opening = domain.open({}, {
            update(sample) {
              const subscription = subscribed.get(subscriptionId)!;
              ws.send(JSON.stringify({ jsonrpc: "2.0", method: "state/update", params: { context: subscription.context, input: { subscriptionId, seq: (++subscription.seq).toString(), sample } } }));
            },
            end() {},
          }, new AbortController().signal);
          subscribed.set(subscriptionId, { seq: 0n, dispose: opening.dispose, context: frame.params.context });
          reply({ subscriptionId, registrationId: reference.registrationId, snapshot: opening.snapshot });
          return;
        }
        if (frame.method === "module/invoke") {
          if (input.method === "add") {
            count += Number(object(input.argument).delta);
            additions++;
            domain.publish({ value: count, revision: (BigInt(domain.read().revision) + 1n).toString(), sampledAtMs: "1", quality: "current" });
            committed.release();
            if (hold) await resume.wait;
          }
          reply({ count });
        }
      },
    },
  });
  // Bun supports constructor headers; browser DOM declarations expose only protocols.
  const ServiceSocket = globalThis.WebSocket as unknown as new (url: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
  const client = new ConexWsClient({ origin: `http://127.0.0.1:${server.port}`, reconnect: true, tokenProvider: async () => "test-token", createWebSocket: (url, headers) => new ServiceSocket(url, { headers }), capabilities: { contracts: [contract] } });
  await client.connect();
  return {
    client, reference, domain, committed, resume,
    bindingId, linkIdentity,
    get helloProvides() { return helloProvides; },
    get readyProvides() { return readyProvides; },
    hold() { hold = true; },
    get additions() { return additions; }, get connections() { return connections; },
    disconnect() { socket.close(); },
    notification(method: string, input: unknown) { socket.send(JSON.stringify({ jsonrpc: "2.0", method, params: { context: { plane: "broker", spaceId: reference.spaceId, providerEndpointId: reference.providerEndpointId, registrationId: reference.registrationId }, input } })); },
    close() { resume.release(); client.close(); for (const subscription of subscribed.values()) subscription.dispose(); server.stop(true); },
  };
}

test("S1 handshake uses generated envelopes and confirms its own asymmetric provides", async () => {
  const run = await peer();
  try {
    expect(run.helloProvides).toEqual([]);
    expect(run.readyProvides).toEqual(run.helloProvides);
    expect(run.client.negotiation).toMatchObject({
      negotiationId: run.bindingId,
      plane: "PLANE_BROKER",
      limits,
      linkIdentity: run.linkIdentity,
    });
    expect(await run.client.invoke(run.reference, "read", {}).result).toEqual({ count: 0 });
  } finally { run.close(); }
});

test("cancel acceptance never overwrites a committed success", async () => {
  const run = await peer();
  try {
    run.hold();
    const call = run.client.invoke<{ delta: number }, { count: number }>(run.reference, "add", { delta: 3 });
    await run.committed.wait;
    expect(await call.cancel()).toEqual({ requested: true });
    run.resume.release();
    expect(await call.result).toEqual({ count: 3 });
    expect(await call.cancel()).toEqual({ requested: false });
    expect(run.additions).toBe(1);
  } finally { run.close(); }
});

test("disconnect yields unknown for a committed write and reconnect never replays it", async () => {
  const run = await peer();
  try {
    run.hold();
    const call = run.client.invoke(run.reference, "add", { delta: 4 });
    const failure = expect(call.result).rejects.toMatchObject({ code: -32012, execution: "unknown", retry: "never" });
    await run.committed.wait;
    const reconnected = new Promise<void>(resolve => {
      const off = run.client.onEvent(event => { if (event.phase === "ready" && run.connections === 2) { off(); resolve(); } });
    });
    run.disconnect();
    await failure;
    await reconnected;
    expect(await run.client.invoke(run.reference, "read", {}).result).toEqual({ count: 4 });
    expect(run.additions).toBe(1);
  } finally { run.close(); }
});

test("uint64 revisions remain exact; repeated seq terminates once and late frames cannot deliver", async () => {
  const run = await peer();
  try {
    const updates: Sample<number>[] = [];
    const subscription = run.client.subscribe(run.reference, "value", {}, (sample: Sample<number>) => updates.push(sample));
    expect((await subscription.snapshot).revision).toBe("9007199254740993");
    await run.client.invoke(run.reference, "add", { delta: 1 }).result;
    expect(updates.map(sample => sample.revision)).toEqual(["9007199254740994"]);
    run.notification("state/update", { subscriptionId: subscription.subscriptionId, seq: "1", sample: { value: 2, revision: "9007199254740995", sampledAtMs: "2", quality: "current" } });
    expect(await subscription.ended).toMatchObject({ code: -32017 });
    const end = await subscription.ended;
    run.notification("state/end", stateEndInput(subscription.subscriptionId, { code: ErrorCode.ERROR_CODE_UNAVAILABLE, message: "gone", execution: "completed", retry: "safe" }));
    expect(await subscription.unsubscribe()).toEqual({ removed: false });
    await run.client.invoke(run.reference, "add", { delta: 1 }).result;
    expect(await subscription.ended).toBe(end);
    expect(updates.map(sample => sample.value)).toEqual([1]);
  } finally { run.close(); }
});

test("producer notifications omit authenticated identity without losing unsubscribe ownership", async () => {
  const module = { spaceId: "test", integrationId: "test", providerEndpointId: "mutable", contractId: contract.contractId };
  const registrationId = mintUlid();
  const subscriptionId = mintUlid();
  const openingId = mintUlid();
  const opened = barrier();
  const sent: string[] = [];
  let observer!: StateObserver<unknown>;
  const context = { plane: "broker", spaceId: module.spaceId, providerEndpointId: module.providerEndpointId, registrationId, principalId: "consumer", tenantId: "tenant" };
  const provider: ModuleProvider = {
    module,
    handlers: { add: () => ({ count: 0 }), read: () => ({ count: 0 }) },
    states: {
      value(_argument, sink, verified) {
        expect(verified).toEqual(context);
        observer = sink;
        return { snapshot: { value: 0, revision: "0", sampledAtMs: "0", quality: "current" }, dispose() {} };
      },
    },
    authorize: verified => verified.principalId === context.principalId && verified.tenantId === context.tenantId,
  };
  const session = new CapabilitySession({ contracts: [contract], providers: [provider] }, mintUlid, text => {
    sent.push(text);
    const frame = decodeS1Frame(text);
    if (frame.method === "module/register") {
      session.receive(JSON.stringify({ jsonrpc: "2.0", id: frame.id, result: { ...module, registrationId } }));
    }
    if (frame.id === openingId) opened.release();
  }, () => { throw new Error("unexpected link close"); });
  session.ready([...CONSUMER_METHODS, "module/register", "module/unregister"], limits);
  try {
    await session.publish(module);
    session.receive(JSON.stringify({
      jsonrpc: "2.0", id: openingId, method: "state/subscribe",
      params: { context, timeoutBudgetMs: 3000, input: { subscriptionId, state: "value", argument: {} } },
    }));
    await opened.wait;
    const unsubscribeId = mintUlid();
    session.receive(JSON.stringify({
      jsonrpc: "2.0", id: unsubscribeId, method: "state/unsubscribe",
      params: { context: { ...context, principalId: "other" }, timeoutBudgetMs: 3000, input: { subscriptionId } },
    }));
    expect(sent.map(decodeS1Frame).find(frame => frame.id === unsubscribeId)?.result).toEqual({ removed: false });
    observer.update({ value: 1, revision: "1", sampledAtMs: "1", quality: "current" });
    observer.end(new ConexError("source unavailable", { code: -32005 }));
    const notifications = sent.map(decodeS1Frame).filter(frame => frame.kind === "notification");
    expect(notifications.map(frame => frame.method)).toEqual(["state/update", "state/end"]);
    for (const frame of notifications) {
      expect(() => validateFrame(frame, "client")).not.toThrow();
      expect(frame.params?.context).toEqual({
        plane: context.plane, spaceId: context.spaceId,
        providerEndpointId: context.providerEndpointId, registrationId,
      });
    }
  } finally { session.disconnected(); }
});
