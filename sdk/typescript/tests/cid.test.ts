import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { CID } from "multiformats/cid";
import { sha256 } from "multiformats/hashes/sha2";
import * as raw from "multiformats/codecs/raw";

const vectorsUrl = new URL("../../../conformance/vectors/p0/cid.json", import.meta.url);
const vectors = JSON.parse(readFileSync(vectorsUrl, "utf8")) as Array<{
  name: string;
  input_utf8: string;
  cid: string;
  bytes_hex: string;
}>;

test("independent multiformats implementation matches shared CID vectors", async () => {
  expect(vectors.length).toBeGreaterThan(0);
  for (const c of vectors) {
    const bytes = new TextEncoder().encode(c.input_utf8);
    const cid = CID.createV1(raw.code, await sha256.digest(bytes));
    expect(cid.toString(), c.name).toBe(c.cid);
    expect(Buffer.from(cid.bytes).toString("hex"), c.name).toBe(c.bytes_hex);
    expect(CID.parse(c.cid).toString(), c.name).toBe(c.cid);
  }
});
