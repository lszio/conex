import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { ErrorCode } from "../src/generated/conex/v1/common";

const vectorsUrl = new URL("../../../conformance/vectors/p0/wire.json", import.meta.url);
const vectors = JSON.parse(readFileSync(vectorsUrl, "utf8")) as {
  errorCodes: Array<{ name: string; value: number }>;
  cases: Array<{ name: string; wire: string; expect: Record<string, unknown> }>;
};

test("generated ErrorCode enum matches the frozen shared table", () => {
  expect(vectors.errorCodes.length).toBeGreaterThan(0);
  for (const entry of vectors.errorCodes) {
    const key = ("ERROR_CODE_" + entry.name.toUpperCase()) as keyof typeof ErrorCode;
    expect(ErrorCode[key], entry.name).toBe(entry.value);
  }
});

test("every shared wire case carries an expectation", () => {
  for (const c of vectors.cases) {
    expect(c.wire.length, c.name).toBeGreaterThan(0);
    expect(Object.keys(c.expect).length, c.name).toBeGreaterThan(0);
  }
});
