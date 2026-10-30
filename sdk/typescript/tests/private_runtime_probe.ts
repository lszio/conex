// Executed by cargo xtask e2e --suite private-runtime; not a unit-test fixture.
import { mkdtemp, writeFile, chmod, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createConnection } from "node:net";
import { createHash } from "node:crypto";
import { strict as assert } from "node:assert";
import type { Subprocess } from "bun";
import Ajv from "ajv";
import { isRecord, validateDomainSchema } from "../src/s1-wire";
import { ConexWsClient } from "../src/ws-client";
import type { WebSocketLike } from "../src/ws-client";
import type { CapabilityContract } from "../src/capability";

type ServiceSocketCtor = new (target: string, options: { headers: Readonly<Record<string, string>> }) => WebSocketLike;
// Bun supports header authentication; the DOM constructor type omits that overload.
const ServiceSocket = globalThis.WebSocket as unknown as ServiceSocketCtor;
import { Database } from "bun:sqlite";

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
interface ProbeResource { id: string; moduleKey: string; generation: string; state: string }
interface ProbeSnapshot { nodeRuntimeEpoch: string; instances: ProbeInstance[]; resources: ProbeResource[] }
const ajv = new Ajv();
const snapshotValidator = ajv.compile<ProbeSnapshot>({
  type: "object", required: ["nodeRuntimeEpoch", "instances", "resources"],
  properties: {
    nodeRuntimeEpoch: { type: "string" },
    resources: { type: "array", items: {
      type: "object", required: ["id", "moduleKey", "generation", "state"],
      properties: { id: { type: "string" }, moduleKey: { type: "string" }, generation: { type: "string" }, state: { type: "string" } },
    } },
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
async function readSnapshot(): Promise<ProbeSnapshot> {
  const value = await rpc(socket, "node/snapshot");
  if (!snapshotValidator(value)) throw new Error(`invalid node snapshot: ${ajv.errorsText(snapshotValidator.errors)}`);
  return value;
}

const root = await mkdtemp(join(tmpdir(), "cx-p2-"));
await chmod(root, 0o700);
const nodeBin = process.env.CONEX_NODE_BINARY!;
const hostBin = process.env.CONEX_HOST_BINARY!;
const token = "private-runtime-test-credential";
const nodeId = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const principal = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
const operator = "01ARZ3NDEKTSV4RRFFQ69G5FAX";
const socket = join(root, "node.sock");
const children: Subprocess[] = [];
const survivors = new Map<number, string>();
const logs: string[] = [];

function launch(args: string[], input?: string) {
  const child = Bun.spawn(args, { env: { ...process.env, CONEX_P2_TEST_TOKEN: token }, stdin: input === undefined ? "ignore" : new TextEncoder().encode(input), stdout: "pipe", stderr: "pipe" });
  children.push(child);
  void new Response(child.stderr).text().then(text => logs.push(text));
  return child;
}

async function processIdentity(pid: number): Promise<string | undefined> {
  try {
    const stat = await Bun.file(`/proc/${pid}/stat`).text();
    const fields = stat.slice(stat.lastIndexOf(")") + 1).trim().split(/\s+/);
    if (!fields[19] || fields[0] === "Z" || fields[0] === "X") return undefined;
    const boot = await Bun.file("/proc/sys/kernel/random/boot_id").text();
    return `${boot.trim()}:${fields[19]}`;
  } catch { return undefined; }
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

async function until<T>(label: string, get: () => Promise<T>, accept: (value: T) => boolean, ms = 15000): Promise<T> {
  const end = Date.now() + ms;
  let error: unknown;
  while (Date.now() < end) {
    try { const value = await get(); if (accept(value)) return value; } catch (cause) { error = cause; }
    await Bun.sleep(50);
  }
  throw new Error(`${label}: deadline; ${error ?? "condition not met"}; ${logs.join("\n")}`);
}

function ids(snapshot: ProbeSnapshot, app: string): ProbeInstance {
  const instance = snapshot.instances.find(instance => instance.appId === app);
  if (!instance) throw new Error(`missing application ${app}`);
  return instance;
}
function moduleOf(snapshot: ProbeSnapshot, app: string): ProbeModule {
  const module = ids(snapshot, app).modules[0];
  if (!module) throw new Error(`missing module ${app}`);
  return module;
}
async function invoke(snapshot: ProbeSnapshot, app: string) {
  const instance = ids(snapshot, app), module = moduleOf(snapshot, app);
  const result = await rpc(socket, "node/invoke", { instanceId: instance.id, module: module.name, generation: module.generation, method: "counter/read", argument: {} });
  if (!isRecord(result) || typeof result.count !== "number" || typeof result.revision !== "string") throw new Error("invalid counter result");
  return { count: result.count, revision: result.revision };
}

try {
  // Bind once to obtain an unused port, then the actual Rust host owns it.
  const portServer = Bun.serve({ port: 0, fetch: () => new Response() });
  const port = portServer.port!; await portServer.stop(true);
  const hostConfig = join(root, "host.toml");
  const hash = createHash("sha256").update(token).digest("hex");
  await writeFile(hostConfig, `listen = "127.0.0.1:${port}"
allow_loopback_http = true
audience = "private-runtime"
content_root = "${root}/content"
session_root = "${root}/sessions"
operation_root = "${root}/operations"
[[tokens]]
token_hash = "${hash}"
principal_id = "${principal}"
tenant_id = "dev"
audience = "private-runtime"
role = "service"
[[capabilities.contracts]]
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
` + ["a", "b", "external"].map(app => `[[capabilities.publish]]
node_id = "${principal}"
space_id = "dev"
integration_id = "integration"
provider_endpoint_id = "p2-${app}-counter"
contract_id = "conex.counter"
[[capabilities.consume]]
principal_id = "${principal}"
space_id = "dev"
provider_endpoint_id = "p2-${app}-counter"
contract_id = "conex.counter"
`).join(""));
  let host = launch([hostBin, hostConfig]);
  const externalSocket = join(root, "external.sock");
  const external = launch([nodeBin, "--module-worker"], JSON.stringify({ socket: externalSocket, token }) + "\n");
  await until("external worker", () => exchange(externalSocket, { token, method: "inspect", delta: 0 }), value => isRecord(value) && typeof value.pid === "number");
  const packages: string[] = [];
  for (const app of ["a", "b", "external", "direct"]) {
    const path = join(root, `${app}.toml`); packages.push(path);
    await writeFile(path, `schema = "conex.application/1"
app_id = "${app}"
version = "0.1.0"
owner_node_ref = "test-node"
[[modules]]
id = "counter"
implementation = "${app === "external" ? "external-counter" : "controlled-counter"}"
config_ref = "${app === "external" ? "external" : "default"}"
permissions = ["counter.read", "counter.add"]
exports = [{contract_id="conex.counter",strategy="${app === "direct" ? "direct-only" : "relay-allowed"}"}]
`);
  }
  const configPath = join(root, "node.toml");
  await writeFile(configPath, `node_id="${nodeId}"
node_ref="test-node"
operator_principal="${operator}"
data_dir="${root}"
local_socket="${socket}"
applications=${JSON.stringify(packages)}
permissions=["counter.read","counter.add"]
[configs.default]
[configs.external]
external_socket="${externalSocket}"
external_token_backend="env:CONEX_P2_TEST_TOKEN"
[publication]
url="ws://127.0.0.1:${port}/wss"
token_backend="env:CONEX_P2_TEST_TOKEN"
space_id="dev"
integration_id="integration"
endpoint_prefix="p2"
allow_loopback_ws=true
exposure_principals=["${principal}"]
exposure_tenants=["dev"]
`);
  let node = launch([nodeBin, "--node-config", configPath]);
  let snapshot = await until("published applications", readSnapshot, value => value.instances.length === 4 && value.instances.every(instance => instance.state === "active" && (instance.appId === "direct" || instance.modules.every(module => module.reference))));
  const original = snapshot;
  const first = await invoke(snapshot, "a");
  const consumer = new ConexWsClient({
    origin: `http://127.0.0.1:${port}`, reconnect: true,
    tokenProvider: async () => token,
    createWebSocket: (target, headers) => new ServiceSocket(target, { headers }),
    capabilities: { contracts: [counterContract] },
  });
  consumers.push(consumer);
  await consumer.connect();
  const catalog = await consumer.listModules("dev");
  const oldReference = catalog.find(module => module.providerEndpointId === "p2-a-counter");
  assert(oldReference, JSON.stringify({ catalog, local: snapshot.instances.map(instance => ({ app: instance.appId, refs: instance.modules.map(module => module.reference) })) }));
  assert.equal(moduleOf(snapshot, "direct").reference, null);
  assert(!catalog.some(module => module.providerEndpointId === "p2-direct-counter"));
  await invoke(snapshot, "direct");
  const spoofed = await exchange(socket, { jsonrpc: "2.0", id: "01ARZ3NDEKTSV4RRFFQ69G5FAY", method: "node/snapshot", params: { context: { plane: "broker", principalId: operator }, input: {} } });
  assert(isRecord(spoofed) && isRecord(spoofed.error) && spoofed.error.code === -32002);
  await consumer.invoke(oldReference, "counter/read", {}).result;
  host.kill(); await host.exited;
  await until("disconnected publication", readSnapshot, value => moduleOf(value, "a").publication !== "published");
  await until("local counter survives actual center shutdown", () => invoke(snapshot, "a"), value => value.count > first.count);
  host = launch([hostBin, hostConfig]);
  snapshot = await until("fresh registration", readSnapshot, value => Boolean(moduleOf(value, "a").reference && moduleOf(value, "a").reference?.registrationId !== moduleOf(original, "a").reference?.registrationId));
  assert.equal(snapshot.nodeRuntimeEpoch, original.nodeRuntimeEpoch);
  assert.equal(moduleOf(snapshot, "a").generation, moduleOf(original, "a").generation);
  assert.equal(moduleOf(snapshot, "a").processId, moduleOf(original, "a").processId);
  await until("consumer reconnect", async () => consumer.state, state => state === "ready");
  const bReference = await until("second application republished", async () => (await consumer.listModules("dev")).find(module => module.providerEndpointId === "p2-b-counter"), value => value !== undefined);
  assert(bReference);
  await assert.rejects(consumer.invoke(oldReference, "counter/read", {}).result, error => {
    return error instanceof Error && "code" in error && error.code === -32013;
  });
  console.log("P2-A04 actual=center killed/restarted; local counter advances; same pid/generation/nodeEpoch; fresh registration ok");
  const stopped = await Promise.all([rpc(socket, "node/stop", { instanceId: ids(snapshot, "a").id }), rpc(socket, "node/stop", { instanceId: ids(snapshot, "a").id })]);
  assert.deepEqual(stopped[0], stopped[1]);
  await assert.rejects(() => invoke(snapshot, "a"));
  await consumer.invoke(bReference, "counter/read", {}).result;
  await invoke(snapshot, "b"); await invoke(snapshot, "external");
  assert.equal(external.exitCode, null);
  console.log("P2-A13 actual=concurrent stop same report; stopped route fenced; other application/external worker survive ok");
  const bGeneration = moduleOf(snapshot, "b").generation;
  await rpc(socket, "node/shutdown"); await node.exited;
  assert.equal(external.exitCode, null);
  node = launch([nodeBin, "--node-config", configPath]);
  snapshot = await until("second node boot", readSnapshot, value => value.instances.some(instance => instance.appId === "b" && instance.state === "active"));
  assert.notEqual(snapshot.nodeRuntimeEpoch, original.nodeRuntimeEpoch);
  assert(BigInt(moduleOf(snapshot, "b").generation) > BigInt(bGeneration));
  const orphanPid = moduleOf(snapshot, "b").processId;
  assert(orphanPid !== null);
  const orphanIdentity = await processIdentity(orphanPid);
  assert(orphanIdentity);
  survivors.set(orphanPid, orphanIdentity);
  node.kill("SIGKILL"); await node.exited;
  // Abrupt parent death must not be mistaken for child exit. New node probes
  // persisted control identity, terminates authenticated old worker, then restarts.
  node = launch([nodeBin, "--node-config", configPath]);
  const recovered = await until("crash resource reconciliation", readSnapshot, value => value.nodeRuntimeEpoch !== snapshot.nodeRuntimeEpoch && value.instances.some(instance => instance.appId === "b" && instance.state === "active"));
  assert.notEqual(moduleOf(recovered, "b").processId, orphanPid);
  assert(BigInt(moduleOf(recovered, "b").generation) > BigInt(moduleOf(snapshot, "b").generation));
  survivors.delete(orphanPid);
  console.log("P2-RECOVERY actual=boot epoch rotates; durable generations increase; crash orphan reconciled by authenticated original control, no PID-only kill ok");
  const blockedPid = moduleOf(recovered, "b").processId;
  assert(blockedPid !== null);
  const blockedIdentity = await processIdentity(blockedPid);
  assert(blockedIdentity);
  survivors.set(blockedPid, blockedIdentity);
  const resource = recovered.resources.find(resource => resource.moduleKey === moduleOf(recovered, "b").moduleKey);
  assert(resource);
  // The fixture creator removes its control path. Losing the channel is not
  // proof of process exit and does not transfer process ownership to Node.
  await rm(join(root, `w-${resource.id}.sock`));
  node.kill("SIGKILL"); await node.exited;
  node = launch([nodeBin, "--node-config", configPath]);
  const blocked = await until("unknown ownership blocks conflicting restart", readSnapshot, value => value.nodeRuntimeEpoch !== recovered.nodeRuntimeEpoch && value.resources.some(current => current.id === resource.id && current.state === "unknown"));
  assert(!blocked.instances.some(instance => instance.appId === "b"));
  assert.equal(await processIdentity(blockedPid), blockedIdentity);
  await invoke(blocked, "a");
  assert.equal(external.exitCode, null);
  console.log("P2-UNKNOWN actual=unreachable owned process stays alive/unknown; conflicting app not restarted; unrelated app and external process survive ok");
  const db = new Database(join(root, "domain.sqlite"));
  try {
    db.exec("CREATE TRIGGER fixture_denied_ownership BEFORE UPDATE ON owned_resources BEGIN SELECT RAISE(ABORT, 'fixture ownership write denied'); END");
    const failures = await Promise.all([rpc(socket, "node/stop", { instanceId: ids(blocked, "a").id }), rpc(socket, "node/stop", { instanceId: ids(blocked, "a").id })]);
    assert.deepEqual(failures[0], failures[1]);
    assert(isRecord(failures[0]) && failures[0].state === "failed");
    const directPid = moduleOf(blocked, "direct").processId;
    assert(directPid !== null);
    await invoke(blocked, "direct");
    await rpc(socket, "node/shutdown"); await node.exited;
    assert.equal(await processIdentity(directPid), undefined);
    assert.equal(external.exitCode, null);
    console.log("P2-CLEANUP actual=ownership write failure reports stable Failed; remaining owned app still cleaned; external process untouched ok");
  } finally {
    db.exec("DROP TRIGGER fixture_denied_ownership"); db.close();
  }
} finally {
  for (const consumer of consumers) consumer.close();
  for (const child of children.reverse()) { if (child.exitCode === null) child.kill(); await child.exited; }
  for (const [pid, identity] of survivors) {
    if (await processIdentity(pid) === identity) { try { process.kill(pid, "SIGTERM"); } catch {} }
  }
  await rm(root, { recursive: true, force: true });
}
