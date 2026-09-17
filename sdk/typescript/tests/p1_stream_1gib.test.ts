//! 1 GiB incremental blob upload over the WSS stream path (design §14 P1 #1).
//!
//! Spawned by `cargo xtask e2e --suite p1-stream-1gib` against a live
//! `conex-host`. Requires env: CONEX_E2E_WS (ws://host/wss), CONEX_E2E_TOKEN.
//! CONEX_E2E_1GIB=1 runs the full 1 GiB upload (4096 × 256 KiB chunks); the
//! default is 8 MiB so the test stays fast for local `bun test` runs.
//!
//! The chunk bytes are streamed one business message per StreamFrame
//! (blob/chunk), with periodic stream/flow to extend credit. Then:
//!   1. blob/commit verifies the server-computed root matches the golden
//!      manifest root for the deterministic content.
//!   2. blob/get round-trips the first chunk's bytes.
//!   3. a bad chunk (wrong chunkCid) is rejected with bad_blob.
//!   4. disconnection: upload 2 chunks, drop, reconnect — the host staging
//!      persists (alreadyHaveChunkCids), the rest completes and commits.

import { expect, test } from "bun:test";
import { cidForRaw, contentCidForParts } from "../src/content";

const wsUrl = process.env.CONEX_E2E_WS;
const token = process.env.CONEX_E2E_TOKEN;
const fullGib = process.env.CONEX_E2E_1GIB === "1";
const enabled = Boolean(wsUrl && token);
const maybe = enabled ? test : test.skip;

const CHUNK = 262_144; // 256 KiB
const TOTAL = fullGib ? 1024 * 1024 * 1024 : 8 * 1024 * 1024;
const CHUNKS = Math.ceil(TOTAL / CHUNK);

const ULID_A = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const ULID_B = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
const ULID_C = "01ARZ3NDEKTSV4RRFFQ69G5FAX";

function chunkBytes(index: number): Uint8Array {
  const bytes = new Uint8Array(CHUNK);
  bytes.fill((index * 7 + 11) % 251);
  return bytes;
}

function stringField(value: unknown, key: string): string {
  if (value === null || typeof value !== "object") return "";
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : String(field ?? "");
}

function listField(value: unknown, key: string): unknown[] {
  if (value === null || typeof value !== "object") return [];
  const field = (value as Record<string, unknown>)[key];
  return Array.isArray(field) ? field : [];
}

interface WireMsg {
  request_id?: unknown;
  id?: unknown;
  method?: unknown;
  error?: unknown;
  result?: unknown;
}

interface WsClient {
  ws: WebSocket;
  pending: Map<string, (msg: WireMsg) => void>;
  next: Promise<void>;
  release: () => void;
}

function isWireMsg(value: unknown): value is WireMsg {
  return (
    typeof value === "object" &&
    value !== null &&
    ("result" in value || "error" in value || "method" in value)
  );
}

async function connect(): Promise<WsClient> {
  const { promise, resolve, reject } = Promise.withResolvers<WsClient>();
  const pending = new Map<string, (msg: WireMsg) => void>();
  const { promise: next, resolve: release } = Promise.withResolvers<void>();
  // Bun's runtime accepts `{ headers }` for the WebSocket constructor; the
  // DOM lib in this tsconfig types the second argument as string[].
  const ws = new WebSocket(wsUrl!, {
    headers: { Authorization: `Bearer ${token}` },
  } as unknown as string[]);
  ws.onopen = () => resolve({ ws, pending, next, release });
  ws.onerror = (event) => {
    // Bun fires an ErrorEvent; the DOM lib only knows Event.
    const message = (event as unknown as { message?: unknown } | undefined)?.message;
    reject(new Error(`ws error: ${typeof message === "string" ? message : "unknown"}`));
  };
  ws.onmessage = (event) => {
    let raw: unknown;
    try {
      raw = JSON.parse(String(event.data));
    } catch {
      return;
    }
    if (!isWireMsg(raw)) return;
    const id = stringField(raw, "request_id") || stringField(raw, "id");
    const cb = id ? pending.get(id) : undefined;
    if (cb) {
      pending.delete(id);
      cb(raw);
    }
  };
  return promise;
}

async function rpc(client: WsClient, frame: Record<string, unknown>): Promise<WireMsg> {
  const id =
    stringField(frame, "request_id") || stringField(frame, "id") || ULID_A;
  const { promise, resolve } = Promise.withResolvers<WireMsg>();
  client.pending.set(id, resolve);
  client.ws.send(JSON.stringify(frame));
  return promise;
}

async function bootstrap(client: WsClient): Promise<void> {
  const hello = await rpc(client, {
    jsonrpc: "2.0",
    id: ULID_A,
    method: "conex/hello",
    params: { profileId: "conex-jsonrpc2-wss-v1", plane: "broker", provides: [], requires: [] },
  });
  const negotiationId = stringField(hello.result, "negotiationId");
  const ready = await rpc(client, {
    jsonrpc: "2.0",
    id: ULID_B,
    method: "conex/ready",
    params: { negotiationId, profileId: "conex-jsonrpc2-wss-v1", plane: "broker" },
  });
  expect(stringField(ready.result, "negotiationId")).toBe(negotiationId);
}

function businessFrame(method: string, id: string, endpointId: string, input: unknown) {
  return {
    request_id: id,
    method,
    context: { providerEndpointId: endpointId, plane: 1 },
    params: input,
  };
}

function openUploadFrame(expectedRootCid: string) {
  return businessFrame("blob/put", ULID_C, "blob-store", {
    formatVersion: "1",
    declaredSizeBytes: String(TOTAL),
    declaredChunkSize: String(CHUNK),
    expectedRoot: { manifestCid: expectedRootCid },
    access: { providerId: "source", plane: "broker", resourceId: "1gib.bin" },
  });
}

function streamFrame(seq: number, inner: Record<string, unknown>): Record<string, unknown> {
  return {
    request_id: `st-${seq}`,
    method: "stream/frame",
    context: { providerEndpointId: "stream", plane: 1 },
    params: {
      sessionId: "s1gib",
      attachmentId: "a1gib",
      epoch: 1,
      streamId: "up",
      seq,
      message: Buffer.from(JSON.stringify(inner), "utf8").toString("base64"),
    },
  };
}

function flowFrame(tag: string, consumed: number): Record<string, unknown> {
  return {
    request_id: tag,
    method: "stream/flow",
    context: { providerEndpointId: "stream", plane: 1 },
    params: {
      sessionId: "s1gib",
      attachmentId: "a1gib",
      epoch: 1,
      streamId: "up",
      consumedBytes: String(consumed),
      requestedWindowBytes: String(4 << 20),
    },
  };
}

function chunkFrame(uploadId: string, index: number, cid: string, bytes: Uint8Array) {
  return businessFrame("blob/chunk", `ch-${index}`, "blob-store", {
    uploadId,
    chunkIndex: String(index),
    chunkCid: cid,
    chunkBytes: Buffer.from(bytes).toString("base64"),
  });
}

maybe(
  "1 GiB (or 8 MiB default) upload over stream with commit + bad-chunk + reconnect resume",
  async () => {
    const client = await connect();
    await bootstrap(client);

    const leafCids: string[] = [];
    for (let i = 0; i < CHUNKS; i++) {
      leafCids.push(await cidForRaw(chunkBytes(i)));
    }
    const expectedRoot = await contentCidForParts(CHUNK, TOTAL, leafCids);

    const put = await rpc(client, openUploadFrame(expectedRoot));
    expect(put.error ?? null, JSON.stringify(put)).toBeNull();
    const uploadId = stringField(put.result, "uploadId");

    let consumed = 0;
    for (let i = 0; i < CHUNKS; i++) {
      const bytes = chunkBytes(i);
      const frame = streamFrame(i + 1, chunkFrame(uploadId, i, leafCids[i], bytes));
      const reply = await rpc(client, frame);
      expect(reply.error ?? null, `chunk ${i}: ${JSON.stringify(reply)}`).toBeNull();
      consumed += bytes.length;
      if ((i + 1) % 8 === 0) {
        await rpc(client, flowFrame(`fl-${i}`, consumed));
      }
    }

    const commit = await rpc(
      client,
      businessFrame("blob/commit", "commit-1", "blob-store", {
        uploadId,
        declaredRoot: { manifestCid: expectedRoot },
        persistence: "local",
      }),
    );
    expect(commit.error ?? null, JSON.stringify(commit)).toBeNull();
    expect(stringField(commit.result, "resourceRootCid")).toBe(expectedRoot);

    const get = await rpc(
      client,
      businessFrame("blob/get", "get-1", "blob-store", { chunkCid: leafCids[0] }),
    );
    expect(get.error ?? null, JSON.stringify(get)).toBeNull();
    const got = Buffer.from(stringField(get.result, "chunkBytes"), "base64");
    expect(got.length).toBe(CHUNK);
    expect(got.every((b) => b === chunkBytes(0)[0])).toBe(true);

    // Bad chunk: bytes do not match the declared chunkCid.
    const badPut = await rpc(client, openUploadFrame(expectedRoot));
    const badUpload = stringField(badPut.result, "uploadId");
    const badCid = await cidForRaw(new Uint8Array(CHUNK).fill(2));
    const badBytes = new Uint8Array(CHUNK).fill(1);
    const badReply = await rpc(
      client,
      streamFrame(9001, chunkFrame(badUpload, 0, badCid, badBytes)),
    );
    const inner = isWireMsg(badReply.result) ? badReply.result.result : undefined;
    const innerError =
      inner !== null && typeof inner === "object" && "error" in inner
        ? (inner as Record<string, unknown>)["error"]
        : undefined;
    expect(
      badReply.error ?? innerError ?? null,
      JSON.stringify(badReply),
    ).not.toBeNull();

    client.ws.close();
    client.release();

    // Reconnect resume: host staging survives the dropped connection.
    const client2 = await connect();
    await bootstrap(client2);
    const put2 = await rpc(client2, openUploadFrame(expectedRoot));
    const upload2 = stringField(put2.result, "uploadId");
    for (let i = 0; i < 2; i++) {
      await rpc(
        client2,
        streamFrame(2 + i, chunkFrame(upload2, i, leafCids[i], chunkBytes(i))),
      );
    }
    client2.ws.close();
    client2.release();

    const client3 = await connect();
    await bootstrap(client3);
    const put3 = await rpc(client3, openUploadFrame(expectedRoot));
    const already = listField(put3.result, "alreadyHaveChunkCids").map(String);
    expect(already).toContain(leafCids[0]);
    expect(already).toContain(leafCids[1]);
    const upload3 = stringField(put3.result, "uploadId");

    let consumed3 = 0;
    for (let i = 0; i < CHUNKS; i++) {
      if (i < 2) continue; // already held by the host
      const bytes = chunkBytes(i);
      const reply = await rpc(
        client3,
        streamFrame(5000 + i, chunkFrame(upload3, i, leafCids[i], bytes)),
      );
      expect(reply.error ?? null, `resumed chunk ${i}: ${JSON.stringify(reply)}`).toBeNull();
      consumed3 += bytes.length;
      if ((i + 1) % 8 === 0) {
        await rpc(client3, flowFrame(`fl3-${i}`, consumed3));
      }
    }
    const commit3 = await rpc(
      client3,
      businessFrame("blob/commit", "commit-3", "blob-store", {
        uploadId: upload3,
        declaredRoot: { manifestCid: expectedRoot },
        persistence: "local",
      }),
    );
    expect(commit3.error ?? null, JSON.stringify(commit3)).toBeNull();
    expect(stringField(commit3.result, "resourceRootCid")).toBe(expectedRoot);
    client3.ws.close();
    client3.release();
  },
  10 * 60_000,
);
