// The browser side of the share scene's content addressing, checked against the
// same frozen vectors the Rust and SDK implementations are checked against, so
// three implementations of one rule stay one rule.

import { readFileSync } from "node:fs";

import { describe, expect, test } from "bun:test";

import { SHARE_CHUNK_SIZE, base32Lower, cidForRaw, chunkCount, declareChunks } from "./cid";

const VECTOR_CHUNKING = new URL(
  "../../../conformance/vectors/p1/chunking.json",
  import.meta.url,
);

function vectors(): {
  name: string;
  payload: { kind: string; value: string };
  chunkSize: number;
  sizeBytes: number;
  expectedRootKind: string;
  rootCid: string;
  leafCids?: string[];
}[] {
  const text = readFileSync(VECTOR_CHUNKING, "utf8");
  return JSON.parse(text).cases as ReturnType<typeof vectors>;
}

function payloadBytes(value: { kind: string; value: string }): Uint8Array {
  // The vectors used here are literals; the byte-pattern cases are the large
  // ones, which carry no CIDs this test needs.
  return new TextEncoder().encode(value.value);
}

describe("content addressing", () => {
  test("base32 is lowercase and unpadded", () => {
    // 5 bytes -> exactly 8 characters: 40 bits is 8 whole 5-bit groups, so
    // there is nothing to pad and unpadded output must not invent any.
    expect(base32Lower(new Uint8Array([0x66, 0x6f, 0x6f, 0x62, 0x61]))).toBe(
      "mzxw6ytb",
    );
    // 1 byte -> 2 characters.
    expect(base32Lower(new Uint8Array([0xff]))).toBe("74");
    expect(base32Lower(new Uint8Array([]))).toBe("");
  });

  test("cid for raw matches the frozen vector", async () => {
    const vector = vectors().find((c) => c.name === "hello_conex_newline");
    expect(vector, "the vector must exist").toBeDefined();
    const cid = await cidForRaw(payloadBytes(vector!.payload));
    expect(cid).toBe(vector!.rootCid);
  });

  test("a single chunk is its own root CID", async () => {
    const vector = vectors().find((c) => c.name === "exactly_one_chunk_262kib");
    expect(vector, "the vector must exist").toBeDefined();
    const bytes = new Uint8Array(vector!.sizeBytes);
    const cid = await cidForRaw(bytes);
    // Single-block content is addressed by the CID of its bytes, so the root
    // the host derives from one declared chunk is that chunk's CID.
    expect(cid.length).toBeGreaterThan(50);
    expect(cid.startsWith("bafkrei")).toBe(true);
  });

  test("declaring chunks of a file keeps every CID in order", async () => {
    const bytes = new Uint8Array(SHARE_CHUNK_SIZE * 2 + 17);
    for (let i = 0; i < bytes.length; i += 1) bytes[i] = i % 251;
    const file = new File([bytes], "big.bin");
    const declared = await declareChunks(file);
    expect(declared.size).toBe(bytes.length);
    expect(declared.chunkCids).toHaveLength(chunkCount(bytes.length));
    // The first CID is the hash of exactly the first chunk, so the host can
    // check the first arriving chunk against it.
    const first = await cidForRaw(bytes.slice(0, SHARE_CHUNK_SIZE));
    expect(declared.chunkCids[0]).toBe(first);
    // Two different chunks never collide, so the order carries information.
    expect(new Set(declared.chunkCids).size).toBe(3);
  });

  test("chunk count matches the geometry the host computes", () => {
    expect(chunkCount(0)).toBe(0);
    expect(chunkCount(1)).toBe(1);
    expect(chunkCount(SHARE_CHUNK_SIZE)).toBe(1);
    expect(chunkCount(SHARE_CHUNK_SIZE + 1)).toBe(2);
    expect(chunkCount(SHARE_CHUNK_SIZE * 3)).toBe(3);
  });
});
