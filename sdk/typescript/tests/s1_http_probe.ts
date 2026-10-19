#!/usr/bin/env bun
// Test-only JSONL control process. All calls use the real HTTP SDK/binding.
import { createInterface } from "node:readline";
import { ConexClient, mintUlid } from "../src/client";
import type { CapabilityContract, Invocation, ModuleReference } from "../src/capability";
import { validateDomainSchema } from "../src/s1-wire";
import { ConexError } from "../src/errors";
import type { CounterResult } from "../src/generated/conex/counter";

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
const contract: CapabilityContract = {
  contractId: "conex.counter",
  methods: {
    "counter/read": { execution: "read_only", input: value => parse("conex.CounterReadInput", value), output: value => parse<CounterResult>("conex.CounterResult", value) },
    "counter/add": { execution: "non_replayable", input: value => parse("conex.CounterAddInput", value), output: value => parse<CounterResult>("conex.CounterResult", value) },
  },
  states: { "counter/value": { input: value => parse("conex.CounterReadInput", value), value: value => parse("conex.CounterStateValue", value) } },
};
const httpUrl = requireEnv("CONEX_HTTP_URL");
const token = requireEnv("CONEX_TOKEN");
const spaceId = requireEnv("CONEX_SPACE");
const calls = new Map<string, Invocation<CounterResult>>();
let bindingId: string | undefined;
let client: ConexClient;
interface ProbeCommand {
  id: string; op: string; target?: ModuleReference; method?: string; argument?: unknown; callId?: string;
}

async function forbidden(method: string): Promise<unknown> {
  if (!bindingId) throw new Error("no observed SDK binding");
  const input = method === "module/register"
    ? { spaceId, integrationId: "integration", providerEndpointId: "counter-ts", contractId: "conex.counter" }
    : { subscriptionId: mintUlid(), state: "counter/value", argument: {} };
  const modules = await client.listModules(spaceId);
  const target = modules.find(module => module.providerEndpointId === "counter-rust");
  if (!target) throw new Error("counter-rust not discoverable");
  const context = method === "module/register" ? { plane: "broker", bindingId } : {
    plane: "broker", bindingId, spaceId, providerEndpointId: target.providerEndpointId, registrationId: target.registrationId,
  };
  const response = await fetch(httpUrl, {
    method: "POST", headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: mintUlid(), method, params: { context, timeoutBudgetMs: 5000, input } }),
  });
  const body = await response.json() as { error?: { code: number; message: string } };
  if (!body.error) throw new Error(`HTTP unexpectedly admitted ${method}`);
  return body.error;
}

async function run(command: ProbeCommand): Promise<unknown> {
  switch (command.op) {
    case "list": return client.listModules(spaceId);
    case "invoke": {
      if (!command.target || !command.method || !command.callId) throw new Error("invoke requires target, method, callId");
      const call = client.invoke<unknown, CounterResult>(command.target, command.method, command.argument ?? {});
      calls.set(command.callId, call);
      try { return await call.result; } finally { calls.delete(command.callId); }
    }
    case "cancel": {
      const call = command.callId ? calls.get(command.callId) : undefined;
      if (!call) throw new Error("unknown invocation");
      return call.cancel();
    }
    case "forbidden": {
      if (command.method !== "module/register" && command.method !== "state/subscribe") throw new Error("unsupported probe method");
      return forbidden(command.method);
    }
    case "shutdown": client.close(); return { closed: true };
    default: throw new Error(`unknown operation ${command.op}`);
  }
}
async function main(): Promise<void> {
  client = await ConexClient.connect({
    url: httpUrl, tokenProvider: async () => token, capabilities: { contracts: [contract] },
    // Observe, never modify, the real SDK's negotiated binding context.
    fetch: async (url, init) => {
      if (typeof init?.body === "string") {
        const request = JSON.parse(init.body);
        if (typeof request.params?.context?.bindingId === "string") bindingId = request.params.context.bindingId;
      }
      return fetch(url, init);
    },
  });
  await client.listModules(spaceId);
  emit({ event: "ready" });
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    if (!line.trim()) continue;
    const command = JSON.parse(line) as ProbeCommand;
    void run(command).then(value => {
      emit({ id: command.id, ok: true, value });
      if (command.op === "shutdown") process.exit(0);
    }, error => {
      const failure = error instanceof ConexError ? { code: error.code, message: error.message, execution: error.execution, retry: error.retry } : { message: String(error) };
      emit({ id: command.id, ok: false, error: failure });
    });
  }
  client.close();
}
void main().catch(error => { console.error(String(error)); process.exit(1); });
