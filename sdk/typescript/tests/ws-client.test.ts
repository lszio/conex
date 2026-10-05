import { expect, test } from "bun:test";

import { ConexWsClient, type WebSocketLike } from "../src/ws-client";

const limits = { maxFrameBytes: 1_048_576, maxInflight: 4, maxQueuedBytes: 8_388_608, timeoutMs: 8_000 };

class FakeWebSocket implements WebSocketLike {
  static instances: FakeWebSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  readonly sent: string[] = [];
  readyState = 0;

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }

  receive(value: unknown): void {
    this.onmessage?.({ data: typeof value === "string" ? JSON.stringify(value) : JSON.stringify(value) });
  }

  disconnect(): void {
    this.readyState = 3;
    this.onclose?.();
  }
}

function helloResult() {
  return {
    negotiationId: "neg-1",
    profileId: "conex-jsonrpc2-wss",
    plane: "broker",
    provides: ["endpoint/list", "source/list", "source/read", "source/search"],
    requires: [],
    limits,
    linkIdentity: { linkId: "link-1", peerId: "host", tenantId: "tenant-a" },
  };
}

function respondHandshake(socket: FakeWebSocket): void {
  socket.open();
  const hello = JSON.parse(socket.sent[0]);
  socket.receive({ jsonrpc: "2.0", id: hello.id, result: helloResult() });
  const ready = JSON.parse(socket.sent[1]);
  socket.receive({ jsonrpc: "2.0", id: ready.id, result: helloResult() });
}

function fetchStub() {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  const fetchImpl = async (url: string | URL, init?: RequestInit): Promise<Response> => {
    calls.push({ url: String(url), init });
    return new Response(JSON.stringify({ ticket: `ticket-${calls.length}`, expiresAt: Date.now() + 30_000 }), {
      status: 201,
      headers: { "content-type": "application/json" },
    });
  };
  return { calls, fetchImpl };
}

test("connect performs ticket handshake and routes endpoint/source requests", async () => {
  FakeWebSocket.instances = [];
  const { calls, fetchImpl } = fetchStub();
  const client = new ConexWsClient({
    origin: "https://host.example",
    csrfToken: "csrf-1",
    fetch: fetchImpl,
    WebSocket: FakeWebSocket,
    reconnect: false,
  });
  const events: unknown[] = [];
  client.onEvent((event) => events.push(event));
  const socketReady = new Promise<void>((resolve) => {
    client.onEvent((event) => {
      if (event.phase === "negotiating") resolve();
    });
  });
  const connecting = client.connect();
  await socketReady;
  const socket = FakeWebSocket.instances[0];
  respondHandshake(socket);
  await connecting;

  expect(client.state).toBe("ready");
  expect(calls[0].init?.credentials).toBe("include");
  expect(new Headers(calls[0].init?.headers).get("x-csrf-token")).toBe("csrf-1");
  expect(client.negotiation).toMatchObject({
    negotiationId: "neg-1",
    profileId: "conex-jsonrpc2-wss",
    plane: "broker",
    provides: ["endpoint/list", "source/list", "source/read", "source/search"],
    linkIdentity: { linkId: "link-1", peerId: "host", tenantId: "tenant-a" },
  });
  expect(socket.url).toBe("wss://host.example/wss?ticket=ticket-1");

  const endpointCall = client.listEndpoints({ limit: 10 });
  const endpointRequest = JSON.parse(socket.sent[2]);
  socket.receive({ jsonrpc: "2.0", id: endpointRequest.id, result: { endpoints: [] } });
  expect(await endpointCall).toEqual({ endpoints: [] });

  const readCall = client.read("notes", { resourceId: "hello.md" });
  const readRequest = JSON.parse(socket.sent[3]);
  expect(readRequest.method).toBe("source/read");
  expect(readRequest.params.input).toEqual({ resourceId: "hello.md" });
  socket.receive({ jsonrpc: "2.0", id: readRequest.id, result: { text: "hello", cid: "cid-1" } });
  expect(await readCall).toEqual({ text: "hello", cid: "cid-1" });

  expect(JSON.stringify(events)).not.toContain("ticket-1");
  client.close();
});

test("close rejects pending business calls", async () => {
  FakeWebSocket.instances = [];
  const { fetchImpl } = fetchStub();
  const client = new ConexWsClient({ origin: "https://host.example", fetch: fetchImpl, WebSocket: FakeWebSocket, reconnect: false });
  const socketReady = new Promise<void>((resolve) => {
    client.onEvent((event) => {
      if (event.phase === "negotiating") resolve();
    });
  });
  const connecting = client.connect();
  await socketReady;
  respondHandshake(FakeWebSocket.instances[0]);
  await connecting;

  const pending = client.read("notes", { resourceId: "never" });
  client.close();
  await expect(pending).rejects.toMatchObject({ code: -32011 });
  expect(client.state).toBe("closed");
});

test("reconnect gets a fresh ticket and restores ready state", async () => {
  FakeWebSocket.instances = [];
  const { calls, fetchImpl } = fetchStub();
  const client = new ConexWsClient({
    origin: "https://host.example",
    fetch: fetchImpl,
    WebSocket: FakeWebSocket,
    reconnect: true,
  });
  const socketReady = new Promise<void>((resolve) => {
    client.onEvent((event) => {
      if (event.phase === "negotiating") resolve();
    });
  });
  const connecting = client.connect();
  await socketReady;
  respondHandshake(FakeWebSocket.instances[0]);
  await connecting;
  FakeWebSocket.instances[0].disconnect();

  const reconnecting = new Promise<void>((resolve) => {
    client.onEvent((event) => {
      if (event.phase === "negotiating" && FakeWebSocket.instances.length === 2) resolve();
    });
  });
  await reconnecting;
  expect(calls.length).toBe(2);
  const replacement = FakeWebSocket.instances[1];
  respondHandshake(replacement);
  await Promise.resolve();
  expect(client.state).toBe("ready");
  expect(replacement.url).toBe("wss://host.example/wss?ticket=ticket-2");
  client.close();
});

test("listConnections sends connection/list over the ready link", async () => {
  FakeWebSocket.instances = [];
  const { fetchImpl } = fetchStub();
  const client = new ConexWsClient({
    origin: "https://host.example",
    fetch: fetchImpl,
    WebSocket: FakeWebSocket,
    reconnect: false,
  });
  const socketReady = new Promise<void>((resolve) => {
    client.onEvent((event) => {
      if (event.phase === "negotiating") resolve();
    });
  });
  const connecting = client.connect();
  await socketReady;
  respondHandshake(FakeWebSocket.instances[0]);
  await connecting;
  expect(client.state).toBe("ready");

  const listPromise = client.listConnections({});
  const sent = JSON.parse(FakeWebSocket.instances[0].sent.at(-1)!);
  expect(sent.method).toBe("connection/list");
  expect(sent.params.input).toEqual({});

  FakeWebSocket.instances[0].receive({
    jsonrpc: "2.0",
    id: sent.id,
    result: {
      browserLinks: [
        {
          linkId: "abc12345",
          principalId: "alice",
          tenantId: "tenant-a",
          connectedAtMs: "1700000000000",
          lastSeenAtMs: "1700000000000",
          ticketsIssued: "1",
          callsTotal: "0",
          callsInFlight: "0",
        },
      ],
      agentLinks: [],
    },
  });
  const result = await listPromise;
  expect(result.browserLinks?.[0]?.linkId).toBe("abc12345");
  expect(result.browserLinks?.[0]?.principalId).toBe("alice");
  client.close();
});

test("client methods issue the right envelopes", async () => {
  FakeWebSocket.instances = [];
  const { fetchImpl } = fetchStub();
  const client = new ConexWsClient({
    origin: "https://host.example",
    csrfToken: "csrf-1",
    fetch: fetchImpl,
    WebSocket: FakeWebSocket,
    reconnect: false,
  });
  const connecting = client.connect();
  await new Promise<void>((resolve) => client.onEvent((e) => { if (e.phase === "negotiating") resolve(); }));
  const socket = FakeWebSocket.instances[0];
  respondHandshake(socket);
  await connecting;

  const listed = client.listClients();
  const listFrame = JSON.parse(socket.sent.at(-1)!);
  expect(listFrame.method).toBe("client/list");
  socket.receive({ jsonrpc: "2.0", id: listFrame.id, result: { clients: [] } });
  expect((await listed).clients).toEqual([]);

  const profiled = client.setProfile({ displayName: "alice", group: "team", visible: false });
  const profileFrame = JSON.parse(socket.sent.at(-1)!);
  expect(profileFrame.method).toBe("client/profile");
  // The profile must ride inside `input`, like every other business call.
  expect(profileFrame.params.input.profile).toEqual({ displayName: "alice", group: "team", visible: false });
  socket.receive({ jsonrpc: "2.0", id: profileFrame.id, result: { self: { linkId: "link-1" } } });
  expect((await profiled).self?.linkId).toBe("link-1");

  const greeted = client.sendHello("link-2", "hi");
  const helloFrame = JSON.parse(socket.sent.at(-1)!);
  expect(helloFrame.method).toBe("client/hello");
  expect(helloFrame.params.input).toEqual({ targetLinkId: "link-2", text: "hi" });
  socket.receive({ jsonrpc: "2.0", id: helloFrame.id, result: { targetLinkId: "link-2", roundTripMs: "7", reply: "pong" } });
  expect(await greeted).toMatchObject({ reply: "pong" });
});

test("a pushed client hello is answered with a pong before listeners run", async () => {
  FakeWebSocket.instances = [];
  const { fetchImpl } = fetchStub();
  const client = new ConexWsClient({
    origin: "https://host.example",
    csrfToken: "csrf-1",
    fetch: fetchImpl,
    WebSocket: FakeWebSocket,
    reconnect: false,
  });
  const connecting = client.connect();
  await new Promise<void>((resolve) => client.onEvent((e) => { if (e.phase === "negotiating") resolve(); }));
  const socket = FakeWebSocket.instances[0];
  respondHandshake(socket);
  await connecting;

  const seen: string[] = [];
  client.onHello((hello) => seen.push(hello.fromName || "(no name)"));

  const before = socket.sent.length;
  // A server-initiated frame carries no id: it is a notification.
  socket.receive({
    jsonrpc: "2.0",
    method: "conex/client-hello",
    params: { replyId: "9", from: "link-2", fromName: "alice", text: "hello" },
  });

  const pongs = socket.sent.slice(before).map((raw) => JSON.parse(raw));
  expect(pongs).toHaveLength(1);
  expect(pongs[0]).toMatchObject({
    jsonrpc: "2.0",
    method: "conex/client-pong",
    params: { replyId: "9", reply: "pong" },
  });
  // No id: a request would make the host wait for a response that never comes.
  expect(pongs[0].id).toBeUndefined();
  // The host resolves the sender's link to a display name, so a client that
  // never chose one is still greeted by something readable.
  expect(seen).toEqual(["alice"]);
});
