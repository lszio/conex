// Executed by cargo xtask e2e --suite module-principals; not a unit-test fixture.
// Topology: one real host, two real nodes. Node B publishes a counter; node A
// runs derived-counter apps that consume it over pinned remote bindings with
// module consumer principals (contracts/s1-module-principals.md). M-A02/M-A03
// were host-judgment behaviors covered at the real-Broker unit level
// (crates/conex-host/tests/s1_calls.rs); since the raw-frame probe landed they
// run here too against the real host. This probe exercises the full wire +
// SDK identity chain (M-A01/M-A04/M-A06) plus the operator regression (M-A07).
import { mkdtemp, writeFile, chmod, rm } from "node:fs/promises";
import { join } from "node:path";
import { createConnection } from "node:net";
import { createHash } from "node:crypto";
import { strict as assert } from "node:assert";
import type { Subprocess } from "bun";
import Ajv from "ajv";
import { isRecord, validateDomainSchema } from "../src/s1-wire";
import { ConexWsClient } from "../src/ws-client";
import { mintUlid } from "../src/client";
import type { WebSocketLike } from "../src/ws-client";
import type { CapabilityContract } from "../src/capability";

type ServiceSocketCtor = new (target: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
const ServiceSocket = globalThis.WebSocket as unknown as ServiceSocketCtor;

const counterContract: CapabilityContract = {
  contractId: "conex.counter",
  methods: {
    "counter/read": {
      execution: "read_only",
      input: value => { validateDomainSchema("conex.CounterReadInput", value); return value; },
      output: value => { validateDomainSchema("conex.CounterResult", value); return value; },
    },
    "counter/add": {
      execution: "non_replayable",
      input: value => { validateDomainSchema("conex.CounterAddInput", value); return value; },
      output: value => { validateDomainSchema("conex.CounterResult", value); return value; },
    },
  },
  states: {},
};
const consumers: ConexWsClient[] = [];

interface ProbeModule {
  name: string;
  moduleKey: string;
  generation: string;
  publication: string;
  processId: number | null;
  reference: { registrationId: string } | null;
}
interface ProbeInstance { id: string; appId: string; state: string; modules: ProbeModule[] }
interface ProbeSnapshot { nodeRuntimeEpoch: string; instances: ProbeInstance[]; resources: unknown[] }
const ajv = new Ajv();
const snapshotValidator = ajv.compile<ProbeSnapshot>({
  type: "object", required: ["nodeRuntimeEpoch", "instances"],
  properties: {
    nodeRuntimeEpoch: { type: "string" },
    instances: { type: "array", items: {
      type: "object", required: ["id", "appId", "state", "modules"],
      properties: {
        id: { type: "string" }, appId: { type: "string" }, state: { type: "string" },
        modules: { type: "array", items: {
          type: "object", required: ["name", "moduleKey", "generation", "publication", "processId", "reference"],
          properties: {
            name: { type: "string" }, generation: { type: "string", pattern: "^[1-9][0-9]*$" },
            moduleKey: { type: "string" },
            publication: { type: "string" }, processId: { type: ["number", "null"] },
            reference: { anyOf: [{ type: "null" }, { type: "object", required: ["registrationId"], properties: { registrationId: { type: "string" } } }] },
          },
        } },
      },
    } },
  },
});
async function readSnapshot(socket: string): Promise<ProbeSnapshot> {
  const value = await rpc(socket, "node/snapshot");
  if (!snapshotValidator(value)) throw new Error(`invalid node snapshot: ${ajv.errorsText(snapshotValidator.errors)}`);
  return value;
}

// The per-node data dir nests a worker control socket under the node id;
// macOS caps sun_path at 104 bytes, so the tree must stay short (P2's flat
// layout fits under os.tmpdir(), two-node nesting does not).
const root = await mkdtemp("/tmp/cx-mp-");
await chmod(root, 0o700);
const nodeBin = process.env.CONEX_NODE_BINARY!;
const hostBin = process.env.CONEX_HOST_BINARY!;
const tokenA = "module-principals-node-a";
const tokenB = "module-principals-node-b";
const tokenOp = "module-principals-operator";
// ---- Raw WSS frames (M-A02/M-A03) ------------------------------------------
// The SDK identity chain refuses to send illegal declarations, so the
// identity-spoofing scenarios speak the wire directly: an operator-
// authenticated raw connection completes hello/ready and crafts module/invoke
// frames with foreign or malformed consumerModule claims — the same judgment
// the unit tests cover in s1_calls.rs, now against the real host.
const RAW_PROVIDES = ["module/invoke", "module/list"];

interface RawReply {
  id: string;
  result?: Record<string, unknown>;
  error?: { code: number; message?: string };
}

function rawConnect(
  url: string,
  token: string,
): Promise<{ socket: WebSocketLike; bindingId: string }> {
  return new Promise((resolve, reject) => {
    const socket = new ServiceSocket(url, { headers: { Authorization: `Bearer ${token}` } });
    const HELLO_ID = mintUlid();
    const READY_ID = mintUlid();
    const fail = (error: unknown) => {
      try { socket.close(); } catch {}
      reject(error instanceof Error ? error : new Error(String(error)));
    };
    const timer = setTimeout(() => fail(new Error("raw S1 handshake timed out")), 10000);
    socket.onmessage = (event: unknown) => {
      const data = (event as { data?: unknown })?.data;
      if (typeof data !== "string") return;
      let frame: RawReply;
      try { frame = JSON.parse(data) as RawReply; } catch { return; }
      if (frame.id === HELLO_ID && frame.result) {
        const result = frame.result as { bindingId: string; limits: Record<string, unknown> };
        socket.send(JSON.stringify({
          jsonrpc: "2.0", id: READY_ID, method: "conex/ready",
          params: {
            context: { plane: "broker" }, timeoutBudgetMs: 8000,
            input: {
              negotiationId: result.bindingId, profileId: "conex-jsonrpc2-wss",
              plane: "PLANE_BROKER", provides: RAW_PROVIDES, limits: result.limits,
            },
          },
        }));
      } else if (frame.id === READY_ID && frame.result) {
        // The pbjson ready reply names the binding "negotiationId".
        const result = frame.result as { negotiationId: string };
        clearTimeout(timer);
        resolve({ socket, bindingId: result.negotiationId });
      } else if (frame.error) {
        fail(new Error(`raw S1 handshake rejected: ${JSON.stringify(frame.error)}`));
      }
    };
    socket.onerror = () => fail(new Error("raw S1 socket errored"));
    socket.onopen = () => {
      socket.send(JSON.stringify({
        jsonrpc: "2.0", id: HELLO_ID, method: "conex/hello",
        params: {
          context: { plane: "broker" }, timeoutBudgetMs: 8000,
          input: {
            profileId: "conex-jsonrpc2-wss", plane: "PLANE_BROKER",
            provides: RAW_PROVIDES, requires: ["module/list"],
          },
        },
      }));
    };
  });
}

function rawInvoke(id: string, registrationId: string, consumerModule: string): unknown {
  return {
    jsonrpc: "2.0", id, method: "module/invoke",
    params: {
      context: {
        plane: "broker", spaceId: "dev",
        providerEndpointId: providerEndpoint, registrationId,
        consumerModule,
      },
      timeoutBudgetMs: 8000,
      input: { method: "counter/read", argument: {} },
    },
  };
}

function rawAwait(socket: WebSocketLike, id: string, ms = 8000): Promise<RawReply> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`raw invoke ${id} timed out`)), ms);
    const previous = socket.onmessage;
    socket.onmessage = (event: unknown) => {
      const data = (event as { data?: unknown })?.data;
      if (typeof data !== "string") return;
      let frame: RawReply;
      try { frame = JSON.parse(data) as RawReply; } catch { return; }
      if (frame.id !== id) return;
      clearTimeout(timer);
      socket.onmessage = previous;
      resolve(frame);
    };
  });
}

const nodeIdA = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const nodeIdB = "01ARZ3NDEKTSV4RRFFQ69G5FB0";
const principalA = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
const principalB = "01ARZ3NDEKTSV4RRFFQ69G5FB1";
const operator = "01ARZ3NDEKTSV4RRFFQ69G5FAX";
const socketA = join(root, nodeIdA, "node.sock");
const socketB = join(root, nodeIdB, "node.sock");
const modulePrincipal = `module/${principalA}/derived/counter`;
const providerEndpoint = "mp-provider-counter";
const derivedEndpoint = "mp-derived-counter";
const children: Subprocess[] = [];
const logs: string[] = [];

function launch(args: string[], env: Record<string, string> = {}) {
  const child = Bun.spawn(args, { env: { ...process.env, ...env }, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  children.push(child);
  void new Response(child.stderr).text().then(text => { if (text.trim()) logs.push(text); });
  return child;
}

function exchange(path: string, body: unknown): Promise<unknown> {
  const { promise, resolve, reject } = Promise.withResolvers<unknown>();
  const connection = createConnection(path);
  let bytes = "";
  connection.setTimeout(5000, () => connection.destroy(new Error("local rpc timeout")));
  connection.on("connect", () => connection.write(JSON.stringify(body) + "\n"));
  connection.on("data", chunk => {
    bytes += chunk;
    if (!bytes.includes("\n")) return;
    connection.end();
    try {
      const reply: unknown = JSON.parse(bytes.split("\n")[0]!);
      resolve(reply);
    } catch (error) { reject(error); }
  });
  connection.on("error", reject);
  connection.on("close", () => { if (!bytes.includes("\n")) reject(new Error("local rpc closed before reply")); });
  return promise;
}

async function rpc(path: string, method: string, input: unknown = {}): Promise<unknown> {
  const reply = await exchange(path, { jsonrpc: "2.0", id: "01ARZ3NDEKTSV4RRFFQ69G5FAY", method, params: { context: { plane: "broker" }, input } });
  if (!isRecord(reply)) throw new Error("invalid local RPC response");
  if (isRecord(reply.error)) throw Object.assign(new Error(String(reply.error.message)), { wire: reply.error });
  return reply.result;
}

async function until<T>(label: string, get: () => Promise<T>, accept: (value: T) => boolean, ms = 20000): Promise<T> {
  const end = Date.now() + ms;
  let error: unknown;
  while (Date.now() < end) {
    try { const value = await get(); if (accept(value)) return value; } catch (cause) { error = cause; }
    await Bun.sleep(50);
  }
  throw new Error(`${label}: deadline; ${error ?? "condition not met"}; ${logs.join("\n")}`);
}

function instanceOf(snapshot: ProbeSnapshot, app: string): ProbeInstance {
  const instance = snapshot.instances.find(instance => instance.appId === app);
  if (!instance) throw new Error(`missing application ${app}`);
  return instance;
}
function moduleOf(snapshot: ProbeSnapshot, app: string): ProbeModule {
  const module = instanceOf(snapshot, app).modules[0];
  if (!module) throw new Error(`missing module ${app}`);
  return module;
}
async function localInvoke(socket: string, snapshot: ProbeSnapshot, app: string, method: string, argument: unknown) {
  const instance = instanceOf(snapshot, app), module = moduleOf(snapshot, app);
  return rpc(socket, "node/invoke", { instanceId: instance.id, module: module.name, generation: module.generation, method, argument });
}
async function readCounter(socket: string, snapshot: ProbeSnapshot, app: string): Promise<number> {
  const result = await localInvoke(socket, snapshot, app, "counter/read", {});
  if (!isRecord(result) || typeof result.count !== "number") throw new Error(`invalid counter result: ${JSON.stringify(result)}`);
  return result.count;
}
function wireError(error: unknown): { code: number; message?: string } {
  assert(isRecord(error) && isRecord((error as { wire?: unknown }).wire), `expected wire error, got ${String(error)}`);
  const wire = (error as { wire: Record<string, unknown> }).wire;
  assert(typeof wire.code === "number", `wire error without code: ${JSON.stringify(wire)}`);
  return { code: wire.code, message: typeof wire.message === "string" ? wire.message : undefined };
}

function appPackage(app: string, remote: boolean): string {
  return `schema = "conex.application/1"
app_id = "${app}"
version = "0.1.0"
owner_node_ref = "node-a"
[[modules]]
id = "counter"
implementation = "derived-counter"
permissions = ["counter.read", "counter.add"]
exports = [{contract_id="conex.counter",strategy="relay-allowed"}]
${remote ? `[[remote_bindings]]
name = "upstream"
node_ref = "node-b"
provider_endpoint_ref = "${providerEndpoint}"
contract_id = "conex.counter"
` : ""}`;
}

function nodeConfig(socket: string, nodeId: string, operatorPrincipal: string, exposurePrincipals: string[], packages: string[], remoteRegistration: string | null): string {
  return `node_id="${nodeId}"
node_ref="${nodeId === nodeIdA ? "node-a" : "node-b"}"
operator_principal="${operatorPrincipal}"
data_dir="${root}/${nodeId}"
local_socket="${socket}"
applications=${JSON.stringify(packages)}
permissions=["counter.read","counter.add"]
[configs.default]
[publication]
url="ws://127.0.0.1:${port}/wss"
token_backend="env:CONEX_MP_TOKEN"
space_id="dev"
integration_id="integration"
endpoint_prefix="mp"
allow_loopback_ws=true
exposure_principals=[${exposurePrincipals.map(principal => `"${principal}"`).join(",")}]
exposure_tenants=["dev"]
${remoteRegistration ? `[[remote_endpoints]]
node_ref = "node-b"
provider_endpoint_ref = "${providerEndpoint}"
contract_id = "conex.counter"
space_id = "dev"
provider_endpoint_id = "${providerEndpoint}"
registration_id = "${remoteRegistration}"
` : ""}`;
}

let port = 0;
try {
  const portServer = Bun.serve({ port: 0, fetch: () => new Response() });
  port = portServer.port!; await portServer.stop(true);
  const hash = (token: string) => createHash("sha256").update(token).digest("hex");
  const hostConfig = (withModuleApproval: boolean) => join(root, `host-${withModuleApproval}.toml`);
  const writeHostConfig = async (withModuleApproval: boolean) => {
    await writeFile(hostConfig(withModuleApproval), `listen = "127.0.0.1:${port}"
allow_loopback_http = true
audience = "module-principals"
content_root = "${root}/content"
session_root = "${root}/sessions"
operation_root = "${root}/operations"
` + [[tokenA, principalA], [tokenB, principalB], [tokenOp, operator]].map(([token, principal]) => `[[tokens]]
token_hash = "${hash(token)}"
principal_id = "${principal}"
tenant_id = "dev"
audience = "module-principals"
role = "service"
`).join("") + `[[capabilities.contracts]]
contract_id = "conex.counter"
[[capabilities.contracts.methods]]
name = "counter/read"
input_schema = "conex.CounterReadInput"
output_schema = "conex.CounterResult"
action = "counter.read"
execution = "read_only"
[[capabilities.contracts.methods]]
name = "counter/add"
input_schema = "conex.CounterAddInput"
output_schema = "conex.CounterResult"
action = "counter.add"
execution = "non_replayable"
` + [providerEndpoint, derivedEndpoint, "mp-unapproved-counter"].map(endpoint => `[[capabilities.publish]]
node_id = "${endpoint === providerEndpoint ? principalB : principalA}"
space_id = "dev"
integration_id = "integration"
provider_endpoint_id = "${endpoint}"
contract_id = "conex.counter"
`).join("") + `[[capabilities.consume]]
principal_id = "${operator}"
space_id = "dev"
provider_endpoint_id = "${derivedEndpoint}"
contract_id = "conex.counter"
[[capabilities.consume]]
principal_id = "${operator}"
space_id = "dev"
provider_endpoint_id = "${providerEndpoint}"
contract_id = "conex.counter"
` + (withModuleApproval ? `[[capabilities.consume]]
principal_id = "${modulePrincipal}"
space_id = "dev"
provider_endpoint_id = "${providerEndpoint}"
contract_id = "conex.counter"
` : ""));
  };
  await writeHostConfig(true);
  let host = launch([hostBin, hostConfig(true)]);
  await until("host http ready", async () => {
    try { await fetch(`http://127.0.0.1:${port}/rpc`, { method: "POST", body: "{}" }); return true; }
    catch { return false; }
  }, value => value);

  // Node B publishes the upstream counter; its registration pins node A's binding.
  const packageB = join(root, "provider.toml");
  await writeFile(packageB, `schema = "conex.application/1"
app_id = "provider"
version = "0.1.0"
owner_node_ref = "node-b"
[[modules]]
id = "counter"
implementation = "controlled-counter"
config_ref = "default"
permissions = ["counter.read", "counter.add"]
exports = [{contract_id="conex.counter",strategy="relay-allowed"}]
`);
  const configB = join(root, "node-b.toml");
  await writeFile(configB, nodeConfig(socketB, nodeIdB, operator, [principalB, modulePrincipal, operator], [packageB], null));
  launch([nodeBin, "--node-config", configB], { CONEX_MP_TOKEN: tokenB });
  const operatorClient = new ConexWsClient({
    origin: `http://127.0.0.1:${port}`, reconnect: true,
    tokenProvider: async () => tokenOp,
    createWebSocket: (target, headers) => new ServiceSocket(target, { headers }),
    capabilities: { contracts: [counterContract] },
  });
  consumers.push(operatorClient);
  await operatorClient.connect();
  const upstreamReference = await until("upstream published", async () =>
    (await operatorClient.listModules("dev")).find(module => module.providerEndpointId === providerEndpoint),
    value => value !== undefined);
  assert(upstreamReference);

  // Node A: approved + unapproved derived consumers, pinned to the upstream
  // registration; SDK declares module/<principalA>/derived|unapproved/counter.
  const packageDerived = join(root, "derived.toml");
  const packageUnapproved = join(root, "unapproved.toml");
  await writeFile(packageDerived, appPackage("derived", true));
  await writeFile(packageUnapproved, appPackage("unapproved", true));
  const configA = join(root, "node-a.toml");
  const writeNodeAConfig = (registration: string) =>
    writeFile(configA, nodeConfig(socketA, nodeIdA, operator, [principalA, operator], [packageDerived, packageUnapproved], registration));
  await writeNodeAConfig(upstreamReference.registrationId);
  const nodeAProcess = launch([nodeBin, "--node-config", configA], { CONEX_MP_TOKEN: tokenA });
  const snapshotA = await until("node-a apps active and published", () => readSnapshot(socketA),
    value => value.instances.length === 2 && value.instances.every(instance => instance.state === "active" && moduleOf({ instances: [instance] } as ProbeSnapshot, instance.appId).reference));

  // M-A04: the approved module principal passes hub judgment; the derived
  // read reflects the upstream count and add forwards upstream exactly +3.
  // The P2 demo worker ticks upward with wall time (count = ticks + adjustment),
  // so every cross-read assertion is bounded by tick drift, never exact.
  const tickDrift = 15; // 100ms ticks; reads are seconds apart at most.
  const upstreamBefore = await readCounter(socketB, await readSnapshot(socketB), "provider");
  const derivedRead = await readCounter(socketA, snapshotA, "derived");
  assert(
    derivedRead >= upstreamBefore && derivedRead <= upstreamBefore + tickDrift,
    `derived read must observe the upstream counter: ${derivedRead} vs ${upstreamBefore}`,
  );
  const addReply = await localInvoke(socketA, snapshotA, "derived", "counter/add", { delta: 3 });
  assert(isRecord(addReply) && typeof addReply.count === "number", `invalid add reply: ${JSON.stringify(addReply)}`);
  assert(
    addReply.count >= upstreamBefore + 3 && addReply.count <= upstreamBefore + 3 + tickDrift,
    `derived add must forward upstream exactly +3 within tick drift: ${addReply.count} vs ${upstreamBefore}`,
  );
  const upstreamAfter = await readCounter(socketB, await readSnapshot(socketB), "provider");
  assert(upstreamAfter >= addReply.count, "upstream count must be monotonic after the forwarded add");
  // Third consumer: the operator consumes the republished capability; its
  // approval is its own (empty consumerModule = operator path, M-A07).
  const derivedReference = (await operatorClient.listModules("dev")).find(module => module.providerEndpointId === derivedEndpoint);
  assert(derivedReference, "derived capability must be published");
  const thirdRead = await operatorClient.invoke<Record<string, never>, { count: number; revision: string }>(derivedReference, "counter/read", {}).result;
  assert.equal(thirdRead.count, upstreamAfter);
  console.log("M-A04 actual=approved module consumer passes judgment; derived read/add forward upstream; republished capability serves a third operator consumer ok");

  // M-A01: the unapproved module principal is forbidden; zero hub frames reach
  // the provider handler (error surfaces from the hub judgment).
  const unapprovedError = await localInvoke(socketA, snapshotA, "unapproved", "counter/read", {}).then(
    () => { throw new Error("unapproved module principal must be forbidden"); },
    error => wireError(error),
  );
  assert.equal(unapprovedError.code, -32002, `expected forbidden, got ${JSON.stringify(unapprovedError)}`);
  console.log("M-A01 actual=unapproved module principal forbidden; operator approval on the same node does not extend to it ok");

  // M-A02/M-A03: raw frames against the real host. The connection is
  // operator-authenticated, so a claim naming another node's principal fails
  // the peer-attribution step and a malformed shape fails the shape step —
  // both before any provider frame, and the catalog must not change under
  // probing (M-A03's "no disclosure" is observable through module/list).
  const catalogBefore = await operatorClient.listModules("dev");
  const raw = await rawConnect(`ws://127.0.0.1:${port}/wss`, tokenOp);
  assert(raw.bindingId.length > 0, "raw S1 handshake must bind");

  // M-A02: a foreign-node module principal over this connection. Request
  // ids are ULIDs everywhere on the wire (bootstrap and business alike).
  const foreignId = mintUlid();
  raw.socket.send(JSON.stringify(rawInvoke(foreignId, upstreamReference.registrationId, `module/${principalB}/provider/counter`)));
  const foreignReply = await rawAwait(raw.socket, foreignId);
  assert(foreignReply.error, `foreign-node claim must be refused: ${JSON.stringify(foreignReply)}`);
  assert.equal(foreignReply.error!.code, -32002, `expected forbidden, got ${JSON.stringify(foreignReply.error)}`);

  // M-A03: malformed shapes are bad_request before any disclosure.
  const malformedClaims = [
    "module/only-two",
    "not-a-module-principal",
    "module//app/m",
    `module/${principalA}/app/m/extra`,
  ];
  for (const [index, claim] of malformedClaims.entries()) {
    const id = mintUlid();
    raw.socket.send(JSON.stringify(rawInvoke(id, upstreamReference.registrationId, claim)));
    const reply = await rawAwait(raw.socket, id);
    assert(reply.error, `malformed claim must be refused: ${claim} -> ${JSON.stringify(reply)}`);
    assert.equal(reply.error!.code, -32602, `expected bad_request for ${claim}, got ${JSON.stringify(reply.error)}`);
  }
  raw.socket.close();

  const catalogAfter = await operatorClient.listModules("dev");
  assert.deepEqual(
    catalogAfter.map(module => [module.providerEndpointId, module.registrationId]),
    catalogBefore.map(module => [module.providerEndpointId, module.registrationId]),
    "probing must not change the catalog or registrations",
  );
  console.log("M-A02/M-A03 actual=raw-frame foreign claim forbidden and malformed shape bad_request; catalog unchanged under probing ok");

  // M-A06: revocation = config without the module approval. The provider's
  // registration rotates on reconnect, so node A is re-pinned and restarted
  // before the judgment; the operator path stays unaffected.
  await host.kill(); await host.exited;
  await writeHostConfig(false);
  host = launch([hostBin, hostConfig(false)]);
  const republished = await until("upstream republished after restart", async () =>
    (await operatorClient.listModules("dev")).find(module => module.providerEndpointId === providerEndpoint && module.registrationId !== upstreamReference.registrationId),
    value => value !== undefined);
  assert(republished);
  await writeNodeAConfig(republished.registrationId);
  await rpc(socketA, "node/shutdown");
  await nodeAProcess.exited;
  const nodeA = launch([nodeBin, "--node-config", configA], { CONEX_MP_TOKEN: tokenA });
  const snapshotA2 = await until("node-a back with fresh pin", () => readSnapshot(socketA),
    value => value.instances.length === 2 && value.instances.every(instance => instance.state === "active" && moduleOf({ instances: [instance] } as ProbeSnapshot, instance.appId).reference));
  const revokedError = await localInvoke(socketA, snapshotA2, "derived", "counter/read", {}).then(
    () => { throw new Error("revoked module principal must be forbidden"); },
    error => wireError(error),
  );
  assert.equal(revokedError.code, -32002, `expected forbidden, got ${JSON.stringify(revokedError)}`);
  // 注册随重启轮换：第三消费者重新发现。
  const freshUpstream = (await operatorClient.listModules("dev")).find(module => module.providerEndpointId === providerEndpoint);
  const freshDerived = (await operatorClient.listModules("dev")).find(module => module.providerEndpointId === derivedEndpoint);
  assert(freshUpstream && freshDerived);
  // 操作员自己的批准完好：直连上游仍然可用。
  const operatorDirect = await operatorClient.invoke<Record<string, never>, { count: number; revision: string }>(freshUpstream, "counter/read", {}).result;
  assert(typeof operatorDirect.count === "number");
  // 组合模块的上游消费随撤销而断：经组合的调用显式失败（-32002），不降级
  // 为伪造成功——组合能力的可用性诚实依赖其上游授权。
  const degraded = await operatorClient.invoke(freshDerived, "counter/read", {}).result.then(
    () => { throw new Error("revoked composite must fail explicitly"); },
    (error: { code?: number }) => error.code,
  );
  assert.equal(degraded, -32002, `expected forbidden, got ${String(degraded)}`);
  console.log("M-A06 actual=module approval removal forbids new calls; operator direct approval unaffected; composite degrades explicitly ok");
  console.log("module-principals acceptance: all scenarios passed");
} finally {
  for (const consumer of consumers) consumer.close();
  for (const child of children.reverse()) { if (child.exitCode === null) child.kill(); await child.exited; }
  if (!process.env.CONEX_MP_KEEP) await rm(root, { recursive: true, force: true });
  else console.log(`kept ${root}`);
}
