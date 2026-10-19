#!/usr/bin/env bun
// Test-only raw WSS consumer probe for the S1 e2e harness (xtask/src/s1_e2e.rs,
// A04/A10). Connects with the real host hello/ready handshake but bypasses the
// SDK, so the harness can drive explicit request ids and forged frames that
// would never pass the SDK's own encoder. All output is JSONL on stdout.
import { createInterface } from "node:readline";
import { mintUlid } from "../src/client";

function emit(value: unknown): void { console.log(JSON.stringify(value)); }
function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`missing ${name}`);
  return value;
}

const url = requireEnv("CONEX_URL");
const token = requireEnv("CONEX_TOKEN");
const spaceId = requireEnv("CONEX_SPACE");

const PROFILE_ID = "conex-jsonrpc2-wss";
const CONSUMER_METHODS = ["module/list", "module/invoke", "module/cancel", "state/subscribe", "state/unsubscribe"];

interface ProbeError { code: number; message: string; data?: unknown }
interface RejectOutcome { rejected: "answered" | "error" | "dropped"; value?: unknown; message?: string }
interface ProbeResponse { result?: unknown; error?: ProbeError }
interface Target { providerEndpointId: string; registrationId: string }
interface ProbeCommand {
  id: string; op: string; spaceId?: string; requestId?: string;
  target?: Target; method?: string; argument?: unknown; fakePrincipal?: string; subscriptionId?: string;
}

function has(value: unknown, key: string): boolean {
  return typeof value === "object" && value !== null && key in value;
}
function parseCommand(line: string): ProbeCommand {
  const value = JSON.parse(line) as Record<string, unknown>;
  if (typeof value.id !== "string" || typeof value.op !== "string") throw new Error("id and op must be strings");
  const command = value;
  const target = has(command, "target") ? (command.target as Record<string, unknown>) : undefined;
  let parsedTarget: Target | undefined;
  if (target !== undefined) {
    if (!has(target, "providerEndpointId") || !has(target, "registrationId")) throw new Error("target needs providerEndpointId and registrationId");
    const t = target as Record<string, unknown>;
    if (typeof t.providerEndpointId !== "string" || typeof t.registrationId !== "string") throw new Error("target fields must be strings");
    parsedTarget = { providerEndpointId: t.providerEndpointId, registrationId: t.registrationId };
  }
  const pickString = (key: string): string | undefined => {
    const field = command[key];
    return typeof field === "string" ? field : undefined;
  };
  return {
    id: command.id as string,
    op: command.op as string,
    spaceId: pickString("spaceId"),
    requestId: pickString("requestId"),
    target: parsedTarget,
    method: pickString("method"),
    argument: command.argument,
    fakePrincipal: pickString("fakePrincipal"),
    subscriptionId: pickString("subscriptionId"),
  };
}

const pending = new Map<string, (value: ProbeResponse) => void>();
const wireErrors: Array<{ code: unknown; message: unknown }> = [];
let socket: WebSocket;
let opened: Promise<void>;

function connect(): Promise<void> {
  // The host requires bearer auth on the upgrade and negotiates the JSON S1
  // subprotocol; Bun's WebSocket accepts both through the headers option.
  const upgrade = WebSocket as unknown as new (
    url: string,
    options: { headers: Record<string, string> },
  ) => WebSocket;
  socket = new upgrade(url, {
    headers: {
      authorization: `Bearer ${requireEnv("CONEX_TOKEN")}`,
      "sec-websocket-protocol": "conex-jsonrpc2-wss",
    },
  });
  opened = new Promise<void>((resolve, reject) => {
    socket.onopen = () => resolve();
    socket.onerror = () => reject(new Error("websocket error"));
    socket.onclose = () => {
      for (const reject of pending.values()) reject({ error: { code: -32000, message: "socket closed" } });
      pending.clear();
    };
    socket.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      let frame: unknown;
      try { frame = JSON.parse(event.data); } catch { return; }
      if (!has(frame, "id")) return;
      const id = (frame as Record<string, unknown>).id;
      // Boundary rejects without a request id (unknown/invalid envelope):
      // surface them so the harness can assert explicit rejection.
      if (typeof id !== "string") {
        if (has(frame, "error")) {
          const e = (frame as Record<string, unknown>).error as Record<string, unknown>;
          wireErrors.push({ code: e.code, message: e.message });
          emit({ event: "wire_error", code: e.code, message: e.message });
        }
        return;
      }
      if (!pending.has(id)) return;
      const resolve = pending.get(id)!;
      pending.delete(id);
      const error = has(frame, "error") ? (frame as Record<string, unknown>).error as unknown : undefined;
      const result = has(frame, "result") ? (frame as Record<string, unknown>).result : undefined;
      if (error !== undefined && has(error, "code") && has(error, "message")) {
        const e = error as Record<string, unknown>;
        resolve({ error: { code: e.code as number, message: e.message as string, data: e.data } });
      } else {
        resolve({ result });
      }
    };
  });
  return opened;
}

function send(request: Record<string, unknown>): Promise<ProbeResponse> {
  return new Promise<ProbeResponse>((resolve) => {
    const id = typeof request.id === "string" ? request.id : mintUlid();
    request.id = id;
    pending.set(id, resolve);
    socket.send(JSON.stringify(request));
  });
}

function failure(error: unknown): { code: number; message: string; execution: string; retry: string } {
  return { code: -32603, message: error instanceof Error ? error.message : String(error), execution: "not_started", retry: "never" };
}

function invoke(target: Target, method: string, argument: unknown, requestId: string): Promise<ProbeResponse> {
  return send({
    jsonrpc: "2.0", id: requestId, method: "module/invoke",
    params: { context: { providerEndpointId: target.providerEndpointId, plane: "broker", spaceId, registrationId: target.registrationId }, timeoutBudgetMs: 5000, input: { method, argument: argument ?? {} } },
  });
}

async function run(command: ProbeCommand): Promise<unknown> {
  switch (command.op) {
    case "hello": {
      const hello = await send({
        jsonrpc: "2.0", id: mintUlid(), method: "conex/hello",
        params: { context: { plane: "broker" }, timeoutBudgetMs: 5000, input: { profileId: PROFILE_ID, plane: "PLANE_BROKER", provides: [], requires: CONSUMER_METHODS } },
      });
      if (hello.error || !has(hello.result, "bindingId")) throw new Error(`hello rejected: ${JSON.stringify(hello.error ?? hello.result)}`);
      const helloBody = hello.result as Record<string, unknown>;
      const ready = await send({
        jsonrpc: "2.0", id: mintUlid(), method: "conex/ready",
        params: { context: { plane: "broker" }, timeoutBudgetMs: 5000, input: { negotiationId: helloBody.bindingId, profileId: PROFILE_ID, plane: "PLANE_BROKER", provides: [], limits: helloBody.limits } },
      });
      if (ready.error || !has(ready.result, "linkIdentity")) throw new Error(`ready rejected: ${JSON.stringify(ready.error ?? ready.result)}`);
      const listed = await send({
        jsonrpc: "2.0", id: mintUlid(), method: "module/list",
        params: { context: { plane: "broker" }, timeoutBudgetMs: 5000, input: { spaceId } },
      });
      if (listed.error || !has(listed.result, "modules")) throw new Error(`list rejected: ${JSON.stringify(listed.error ?? listed.result)}`);
      const modules = (listed.result as Record<string, unknown>).modules;
      emit({ event: "ready", modules });
      return { modules };
    }
    case "list": {
      const listed = await send({
        jsonrpc: "2.0", id: mintUlid(), method: "module/list",
        params: { context: { plane: "broker" }, timeoutBudgetMs: 5000, input: { spaceId: command.spaceId ?? spaceId } },
      });
      if (listed.error) throw new Error(`list rejected: ${JSON.stringify(listed.error)}`);
      return listed.result;
    }
    case "invoke": {
      const target = command.target;
      if (!target || !command.method) throw new Error("invoke needs target and method");
      const response = await invoke(target, command.method, command.argument, command.requestId ?? mintUlid());
      return response.error ? { error: response.error } : response.result;
    }
    case "forged_identity": {
      // 客户端自报身份 → 边界解码必须拒绝：显式错误帧或静默丢弃都算
      // 拒绝，但不能进入业务（harness 另行断言 executions 不变）。
      const marker = wireErrors.length;
      const pendingBefore = pending.size;
      const outcome = await Promise.race([
        run({ ...command, op: "unwrapped_identity" }).then(
          (value): RejectOutcome => ({ rejected: "answered", value }),
          (error): RejectOutcome => ({ rejected: "error", message: String(error) }),
        ),
        new Promise<RejectOutcome>(resolve =>
          setTimeout(() => resolve({ rejected: "dropped" }), 4000),
        ),
      ]);
      return {
        rejected: outcome.rejected,
        wireErrors: wireErrors.slice(marker),
        pendingDelta: pending.size - pendingBefore,
        value: outcome.rejected === "answered" ? outcome.value : undefined,
      };
    }
    case "unwrapped_identity": {
      const target = command.target;
      if (!target || !command.method) throw new Error("unwrapped_identity needs target and method");
      const response = await send({
        jsonrpc: "2.0", id: mintUlid(), method: "module/invoke",
        params: { context: { providerEndpointId: target.providerEndpointId, plane: "broker", spaceId, registrationId: target.registrationId, principalId: command.fakePrincipal ?? "counter-rust" }, timeoutBudgetMs: 5000, input: { method: command.method, argument: command.argument ?? {} } },
      });
      return response.error ? { error: response.error } : response.result;
    }
    case "forbidden_target": {
      // 已认证但未获消费批准的主体访问目标 → forbidden，不转发业务帧。
      const target = command.target;
      if (!target || !command.method) throw new Error("forbidden_target needs target and method");
      const response = await invoke(target, command.method, command.argument, mintUlid());
      return response.error ? { error: response.error } : response.result;
    }
    case "forged_sub": {
      // 伪造的 subscriptionId 通知：中心必须无视（不结束任何真实订阅）。
      const target = command.target;
      if (!target) throw new Error("forged_sub needs target");
      const subscriptionId = command.subscriptionId ?? mintUlid();
      const context = { providerEndpointId: target.providerEndpointId, plane: "broker", spaceId, registrationId: target.registrationId };
      socket.send(JSON.stringify({ jsonrpc: "2.0", method: "state/update", params: { context, input: { subscriptionId, seq: "1", sample: { revision: "1", value: { count: 1 }, sampledAtMs: "0", quality: "current" } } } }));
      socket.send(JSON.stringify({ jsonrpc: "2.0", method: "state/end", params: { context, input: { subscriptionId, error: { code: "ERROR_CODE_UNAVAILABLE", message: "forged", diagnosticId: "", execution: "completed", retry: "safe" } } } }));
      return { sent: true };
    }
    case "shutdown": return { closed: true };
    default: throw new Error(`unknown operation ${command.op}`);
  }
}

async function main(): Promise<void> {
  await connect();
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    if (!line.trim()) continue;
    let command: ProbeCommand;
    try { command = parseCommand(line); } catch (error) { emit({ id: null, ok: false, error: failure(error) }); continue; }
    void run(command).then(
      (value) => {
        emit({ id: command.id, ok: true, value });
        if (command.op === "shutdown") process.exit(0);
      },
      (error) => emit({ id: command.id, ok: false, error: failure(error) }),
    );
  }
  process.exit(0);
}
void main().catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exit(1); });
