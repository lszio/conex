//! P1 chunking & manifest golden CID vectors. The expected values come from
//! `conformance/vectors/p1/chunking.json`; they must match the Rust cid module
//! (see crates/conex-proto/tests/chunking.rs) and the TS chunking helper.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { CID } from "multiformats/cid";
import { sha256 } from "multiformats/hashes/sha2";
import * as raw from "multiformats/codecs/raw";

const CHUNK_SIZE = 262_144;
const MANIFEST_PREFIX = "manifest/v1/";

const vectorsUrl = new URL(
  "../../../conformance/vectors/p1/chunking.json",
  import.meta.url,
);
const vectors = JSON.parse(readFileSync(vectorsUrl, "utf8")) as {
  cases: Array<{
    name: string;
    input: string;
    expected: "raw" | "manifest";
    cid: string;
    sizeBytes?: number;
  }>;
};

async function cidForRaw(bytes: Uint8Array): Promise<string> {
  return CID.createV1(raw.code, await sha256.digest(bytes)).toString();
}

function bytesFor(input: string): Uint8Array {
  switch (input) {
    case "256kib-of-0xAB":
      return new Uint8Array(CHUNK_SIZE).fill(0xab);
    case "256kib-of-0xAB + 0xCD": {
      const v = new Uint8Array(CHUNK_SIZE + 1);
      v.fill(0xab);
      v[CHUNK_SIZE] = 0xcd;
      return v;
    }
    case "512kib-of-0x01":
      return new Uint8Array(CHUNK_SIZE * 2).fill(0x01);
    case "1.25MiB-pattern-byte[i%256]": {
      const v = new Uint8Array(CHUNK_SIZE * 5);
      for (let i = 0; i < v.length; i += 1) {
        v[i] = i & 0xff;
      }
      return v;
    }
    default:
      return new TextEncoder().encode(input);
  }
}

async function contentCid(bytes: Uint8Array): Promise<string> {
  if (bytes.length <= CHUNK_SIZE) {
    return cidForRaw(bytes);
  }
  const chunks: Uint8Array[] = [];
  for (let i = 0; i < bytes.length; i += CHUNK_SIZE) {
    chunks.push(bytes.subarray(i, Math.min(i + CHUNK_SIZE, bytes.length)));
  }
  const leafCids = await Promise.all(chunks.map(cidForRaw));
  const manifest = new TextEncoder().encode(MANIFEST_PREFIX);
  const total = manifest.length + leafCids.join("").length;
  const out = new Uint8Array(total);
  out.set(manifest, 0);
  let off = manifest.length;
  for (const cid of leafCids) {
    out.set(new TextEncoder().encode(cid), off);
    off += cid.length;
  }
  return cidForRaw(out);
}

test("shared chunking vectors match independent TS implementation", async () => {
  expect(vectors.cases.length).toBeGreaterThan(0);
  for (const c of vectors.cases) {
    const bytes = bytesFor(c.input);
    const actual = await contentCid(bytes);
    expect(actual, c.name).toBe(c.cid);
    if (c.expected === "raw") {
      expect(bytes.length, c.name).toBeLessThanOrEqual(CHUNK_SIZE);
    } else {
      expect(bytes.length, c.name).toBeGreaterThan(CHUNK_SIZE);
    }
  }
});

test("contentCid is deterministic", async () => {
  const bytes = new Uint8Array(CHUNK_SIZE + 7);
  for (let i = 0; i < bytes.length; i += 1) {
    bytes[i] = i & 0xff;
  }
  const a = await contentCid(bytes);
  const b = await contentCid(bytes);
  const c = await contentCid(bytes);
  expect(a).toBe(b);
  expect(b).toBe(c);
});