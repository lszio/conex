//! P1 contract classification vectors (TS side).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

type Doc = Record<string, unknown>;

function load(name: string): Doc {
  const url = new URL(`../../../conformance/vectors/p1/${name}`, import.meta.url);
  return JSON.parse(readFileSync(url, "utf8"));
}

function expectAllHaveExpect(
  doc: Doc,
  key: string,
  container: string,
): void {
  const cases = doc[container] as Array<{ name?: string; expect?: unknown }>;
  expect(cases.length, `${key} ${container}`).toBeGreaterThan(0);
  for (const case_ of cases) {
    expect(
      typeof case_.expect,
      `${key}/${container} case ${case_.name ?? "?"}: expect must be a string`,
    ).toBe("string");
  }
}

test("blob vectors are well formed", () => {
  const doc = load("blob.json");
  expect(Array.isArray(doc.stateMachine)).toBe(true);
  expect(Array.isArray(doc.pin)).toBe(true);
  expectAllHaveExpect(doc, "blob", "cases");
});

test("stream vectors cover ack / credit / reset / epoch / slow consumer", () => {
  const doc = load("stream.json");
  for (const key of [
    "ackCases",
    "creditCases",
    "resetCases",
    "slowConsumerCases",
    "epochCases",
  ]) {
    expectAllHaveExpect(doc, "stream", key);
  }
  const frameShape = doc.frameShape as { zeroByteMessage?: string; seqStart?: number };
  expect(frameShape.zeroByteMessage).toBe("rejected");
  expect(frameShape.seqStart).toBe(1);
});

test("operation vectors cover dedup, execution classes, and conditional write", () => {
  const doc = load("operation.json");
  const classes = (doc.executionClasses as Array<{ name: string }>).map((c) => c.name);
  for (const required of ["read_only", "idempotent", "deduplicated", "non_replayable"]) {
    expect(classes.includes(required), `execution class ${required}`).toBe(true);
  }
  expectAllHaveExpect(doc, "operation", "cases");
  expectAllHaveExpect(doc, "operation", "expectedRevisionCases");
});

test("chunking vectors enumerate invalid CID shapes", () => {
  const doc = load("chunking.json");
  const rejects = doc.rejects as Array<{ name: string }>;
  expect(rejects.some((r) => r.name === "non_raw_cid_rejected")).toBe(true);
  expect(rejects.some((r) => r.name === "non_sha256_multihash_rejected")).toBe(true);
});