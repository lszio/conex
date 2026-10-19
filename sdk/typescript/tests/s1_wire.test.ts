import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  decodeS1Frame, validateFrame, validateResult, validateLimits,
  validateDomainSchema, validateHttpFrame, S1WireError, parseStateEndError,
  stateEndErrorToJson, stateUpdateInput, isCanonicalUint64,
  type S1Boundary, type S1Frame,
} from "../src/s1-wire";
import { ErrorCode } from "../src/generated/conex/common";

type Case = {
  name: string;
  wire: string;
  transport?: "http";
  expect: { accept: boolean; hostAccept?: boolean; kind?: S1Frame["kind"]; method?: string; requestId?: string; errorCode?: number };
};
const vectors = JSON.parse(readFileSync(new URL("../../../conformance/vectors/s1/capabilities.json", import.meta.url), "utf8")) as {
  cases: Case[];
  limits: { name: string; json: Record<string, unknown>; accept: boolean }[];
  counterArguments: { name: string; schema: string; argument: unknown; accept: boolean }[];
};

function pipeline(c: Case, boundary: S1Boundary) {
  const frame = decodeS1Frame(c.wire);
  validateFrame(frame, boundary);
  if (c.transport === "http") validateHttpFrame(frame);
  if (frame.kind === "success" && c.expect.method) validateResult(c.expect.method, frame.result);
  return frame;
}

for (const c of vectors.cases) {
  test(`S1 wire: ${c.name}`, () => {
    for (const boundary of ["client", "host"] as const) {
      if (boundary === "host" && c.expect.hostAccept === undefined) continue;
      const accept = boundary === "client" ? c.expect.accept : c.expect.hostAccept!;
      let error: unknown;
      try {
        const frame = pipeline(c, boundary);
        expect(accept, `${boundary}: unexpectedly accepted`).toBe(true);
        if (c.expect.kind) expect(frame.kind).toBe(c.expect.kind);
        if (c.expect.requestId) expect(frame.id).toBe(c.expect.requestId);
        if (c.expect.method && frame.kind !== "success") expect(frame.method).toBe(c.expect.method);
      } catch (e) {
        error = e;
      }
      if (accept) {
        if (error) throw error;
      } else {
        expect(error).toBeInstanceOf(S1WireError);
        if (c.expect.errorCode !== undefined) expect((error as S1WireError).rpcCode).toBe(c.expect.errorCode);
      }
    }
  });
}

for (const c of vectors.limits) {
  test(`S1 limits: ${c.name}`, () => {
    if (c.accept) validateLimits(c.json);
    else expect(() => validateLimits(c.json)).toThrow(S1WireError);
  });
}

for (const c of vectors.counterArguments) {
  test(`S1 installed domain: ${c.name}`, () => {
    if (c.accept) validateDomainSchema(c.schema, c.argument);
    else expect(() => validateDomainSchema(c.schema, c.argument)).toThrow(S1WireError);
  });
}

test("state helpers preserve uint64 and enum names without Number coercion", () => {
  const input = stateUpdateInput("01ARZ3NDEKTSV4RRFFQ69G5FAX", "1", {
    revision: "18446744073709551615", sampledAtMs: "1792195200000", quality: "current", value: { count: 2 },
  });
  expect(input).toEqual({
    subscriptionId: "01ARZ3NDEKTSV4RRFFQ69G5FAX", seq: "1",
    sample: { revision: "18446744073709551615", sampledAtMs: "1792195200000", quality: "current", value: { count: 2 } },
  });
  const error = stateEndErrorToJson({ code: ErrorCode.ERROR_CODE_FORBIDDEN, message: "revoked", execution: "completed", retry: "never" });
  expect(error).toEqual({ code: "ERROR_CODE_FORBIDDEN", message: "revoked", diagnosticId: "", execution: "completed", retry: "never" });
  expect(parseStateEndError(error).code).toBe(ErrorCode.ERROR_CODE_FORBIDDEN);
  expect(() => stateEndErrorToJson({ code: ErrorCode.ERROR_CODE_FORBIDDEN, execution: "completed", retry: "safe" })).toThrow(S1WireError);
});

test("uint64 canonical boundaries", () => {
  for (const s of ["0", "1", "9007199254740993", "18446744073709551615"]) expect(isCanonicalUint64(s)).toBe(true);
  for (const s of ["", "00", "007", "-1", "+1", "1.5", "1e3", "18446744073709551616"]) expect(isCanonicalUint64(s)).toBe(false);
});

test("JSON nesting is bounded, braces inside strings are not nesting", () => {
  const envelope = { jsonrpc: "2.0", id: "01ARZ3NDEKTSV4RRFFQ69G5FAV", result: "{".repeat(256) };
  expect(decodeS1Frame(JSON.stringify(envelope)).result).toBe(envelope.result);
  expect(() => decodeS1Frame("[".repeat(129) + "0" + "]".repeat(129))).toThrow(S1WireError);
});
