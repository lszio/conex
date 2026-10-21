# @conex/sdk

TypeScript and JavaScript consume the same built SDK (`dist/index.js` and declarations); import from `@conex/sdk`, not from source or a second JS implementation. Browser imports contain no Node modules.

```sh
npm install @conex/sdk
```

## S1: provide and consume on one socket

S1 uses JSON WSS with `conex/hello` → `conex/ready`. Enable it explicitly with `capabilities: { contracts, providers? }`. Contracts are **preinstalled** parsers, method execution metadata (`read_only` or `non_replayable`), and state-value parsers; registration never installs schemas or grants permission. Use generated types and the shared generated-schema validator for domain payloads:

S1 currently uses static Host approvals plus mandatory provider-local authorization. It is restricted development functionality: **do not expose a public multi-user service before S4 identity governance**. HTTP is consumer-only; it cannot publish modules or subscribe to state. See the [three-process example](../../examples/s1/README.md), [runbook](../../docs/runbooks/s1-capabilities.md), and [observed verification](../../docs/verification/s1-capabilities.md). The symmetric Rust SDK is `crates/conex-sdk`, independent of `conex-host`.

## S2: plugin runtime (`definePlugin` / `Instance`)

A plugin class composes modules inside your process: one instance is exactly one S1 connection; local dependencies are in-process handles, explicit remote bindings pin a dependency name to a peer module's `ModuleRef`, and derived modules republish under the same contract with their own revision sequence. Missing or circular dependencies are rejected with typed local errors before any network frame; `stop()` is idempotent and releases in reverse topology order. See the [dual-instance example](../../examples/s2/README.md), [runbook](../../docs/runbooks/s2-plugins.md), and [observed verification](../../docs/verification/s2-plugins.md).

```ts
import {
  ConexWsClient, AtomicState, validateDomainSchema,
  type CapabilityContract, type ModuleProvider, type CounterResult,
} from "@conex/sdk";

function parse<T>(schema: string, value: unknown): T {
  validateDomainSchema(schema, value);
  return value as T; // The preinstalled generated schema is the boundary.
}
const contract: CapabilityContract = {
  contractId: "conex.counter",
  methods: {
    "counter/read": {
      execution: "read_only",
      input: value => parse("conex.CounterReadInput", value),
      output: value => parse<CounterResult>("conex.CounterResult", value),
    },
    "counter/add": {
      execution: "non_replayable",
      input: value => parse("conex.CounterAddInput", value),
      output: value => parse<CounterResult>("conex.CounterResult", value),
    },
  },
  states: {
    "counter/value": {
      input: value => parse("conex.CounterReadInput", value),
      value: value => parse("conex.CounterStateValue", value),
    },
  },
};
```

The application supplies real domain handlers; the communication client implements no counter. For example, given an application-owned `counter` whose read/add and state publication share one synchronous mutation boundary:

```ts
const module = {
  spaceId: "dev", integrationId: "demo",
  providerEndpointId: "counter-ts", contractId: "conex.counter",
};
const provider: ModuleProvider = {
  module,
  authorize: (context, member, argument) => localPolicy(context, member, argument),
  handlers: {
    "counter/read": (_argument, context, signal) => counter.read(context, signal),
    "counter/add": (argument, context, signal) => counter.add(argument, context, signal),
  },
  states: {
    "counter/value": (argument, observer, context, signal) =>
      counter.openValue(argument, observer, context, signal),
  },
};

const client = new ConexWsClient({
  origin: "https://your-host", capabilities: { contracts: [contract], providers: [provider] },
  tokenProvider: async () => process.env.CONEX_TOKEN!,
  // Bun/Node adapter: the caller imports its WebSocket implementation.
  createWebSocket: (url, headers) => new ServiceWebSocket(url, { headers }),
});
await client.connect();
const own = await client.publishModule(module);
const peers = await client.listModules("dev");
const peer = peers.find(item => item.providerEndpointId === "counter-rust")!;
const call = client.invoke<{}, CounterResult>(peer, "counter/read", {});
console.log(call.requestId, await call.result);
await client.unpublishModule(own);
client.close();
```

`ServiceWebSocket` is the injected Bun/Node constructor adapter, returning `WebSocketLike`. With DOM-only type declarations, give that adapter the SDK's `WebSocketLike` return type rather than changing browser imports. S1 adapters must expose accurate `bufferedAmount`; queue limits include opening buffers and the socket queue. Bearer credentials go only in the `Authorization` constructor header, **never** the URL. The existing browser mode uses same-origin one-use tickets and `csrfToken` instead; ordinary UI sessions need no module publication and need not enable S1.

Public WS methods:

- `publishModule(module: Required<ModuleRegisterInput>): Promise<ModuleReference>`: only a module installed in `providers`, with every contract handler present.
- `unpublishModule(reference): Promise<{removed:boolean}>`.
- `listModules(spaceId): Promise<ModuleReference[]>`: concrete space/endpoint/registration/integration/contract plus projected methods/states.
- `invoke<I,O>(reference, method, argument, {timeoutMs?,signal?}?): Invocation<O>`.
- `subscribe<I,T>(reference, state, argument, onUpdate, {timeoutMs?,signal?}?): StateSubscription<T>`.

References are fixed registration instances. A stale reference is not silently rebound. Consumer-supplied identity/context fields are rejected. Provider handlers receive only the authenticated Host's verified identity, fixed registration context, and an `AbortSignal`, and must pass the mandatory local `authorize` ceiling. Cancellation handling runs independently of asynchronous business handlers. A CPU-blocking synchronous handler still blocks JavaScript: move blocking application work outside this event loop.

All required methods and all six positive limits must negotiate before S1 works. A provider advertises invoke/cancel/subscribe/unsubscribe handling; no S1 path falls back to hello or file calls. Resource limits reserve an inflight slot and frame-sized queue space for controls; failed state delivery terminates rather than coalescing or dropping updates. Handlers that ignore cancellation retain an inflight slot until they return, preventing unbounded abandoned handlers on a live link.

## Cancellation is not rollback

```ts
const operation = client.invoke<{ delta: number }, CounterResult>(peer, "counter/add", { delta: 3 });
const acceptance = await operation.cancel(); // { requested }; NOT the operation's result
const result = await operation.result;      // success, confirmed failure, or unknown outcome
```

An `AbortSignal` requests cancellation; it does not forge a `cancelled` result. Known success wins over late cancellation. For a possibly sent non-replayable call, lost connection, deadline, send uncertainty or invalid output yields `ConexError` code `-32012`, `execution="unknown"`, `retry="never"`. Read-only transport/deadline failures use unavailable/timeout and `retry="safe"`. Safe is caller advice, not automatic retry. Neither S1 nor legacy WS pending calls replay on reconnect; reconnect restores transport only. Explicitly rediscover, republish and create new subscriptions as appropriate.

## Atomic snapshots, exact revisions, stale state

A `StateProvider<I,T>` receives `(argument, observer, signal)` and returns `{snapshot, dispose}` (or a Promise thereof). `ModuleProvider.states` adds the verified context before `signal`. Snapshot capture and observer installation must occur in **one serialized domain operation**. Do not implement it as an unsynchronized read followed by listen.

`AtomicState<T>` is a small synchronous holder for this boundary:

```ts
const state = new AtomicState({
  revision: "0", value: { count: 0 }, sampledAtMs: String(Date.now()), quality: "current" as const,
});
// In the domain's synchronous commit, after its validation and mutation:
state.publish({ revision: "1", value: { count: 3 }, sampledAtMs: String(Date.now()), quality: "current" });
// Provider state handler: (_argument, observer, _context, signal) => state.open({}, observer, signal)
```

`read()` and published values are cloned. `open()` captures the snapshot and attaches the observer without yielding. If the domain needs async serialization, supply its own provider closure under the same lock used for mutations. Opening updates are bounded and released only after the snapshot response is sent. Dispose observers on abort; the SDK also invokes the returned `dispose` on teardown.

```ts
const stream = client.subscribe<{}, { count: number }>(peer, "counter/value", {}, sample => {
  render(sample.value, sample.quality); // full ordered updates, not patches
});
render((await stream.snapshot).value, "current");
const termination = await stream.ended; // one terminal ConexError
markRetainedValueStale(termination);
// Or stop explicitly; local callbacks stop immediately, before network cleanup:
await stream.unsubscribe(); // { removed }
```

Use the actual snapshot's `quality` in a UI, not the illustrative constant above: `stale` source data stays stale even on a healthy socket. `revision`, `seq`, and `sampledAtMs` remain decimal uint64 strings; comparisons use BigInt, never Number. Snapshot is accepted before any update. Missing, repeated, out-of-order sequence/revision or a mismatched registered origin ends the stream with session_lost. Unsolicited subscription IDs create nothing. `ended` resolves exactly once, including local unsubscribe; no updates follow it. Retained values become stale on termination. No subscription IDs, snapshots or updates resume automatically after reconnect.

## HTTP consumer

```ts
import { ConexClient } from "@conex/sdk";
const http = await ConexClient.connect({
  url: "https://your-host/rpc", tokenProvider: getToken,
  capabilities: { contracts: [contract] },
});
const modules = await http.listModules("dev");
const request = http.invoke<{}, CounterResult>(modules[0], "counter/read", {});
console.log(await request.result);
http.close();
```

HTTP exposes `listModules` and `invoke` with the same independent `requestId/result/cancel()` handle. It validates negotiated limits, preserves the original binding for cancellation, and does not reuse the legacy read-only authentication retry path for S1. HTTP registration and subscriptions throw unsupported_capability; there is no polling subscription fallback.

## Existing UI and content methods

`ConexWsClient({origin,csrfToken})` retains explicit client hello/profile/list, source and notification methods. Existing `ConexClient` source read/list/search and content-plane helpers remain available. JS uses the built package just like TS.

S1 is for statically approved development nodes/spaces/modules. WSS requires TLS remotely (loopback development may use WS). Static policy is not S4 user management: do not publish S1 as a public multi-user application before the security management boundary exists.

MIT licensed. See the repository S1 contract for wire details and cross-language acceptance.
