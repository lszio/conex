import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import Ajv2020 from "ajv/dist/2020";
import { Scalars } from "../src/generated/scalars";

const schemaUrl = new URL(
  "../../../schema/generated/jsonschema/conex.test.v1.Scalars.schema.json",
  import.meta.url,
);
const vectorsUrl = new URL("../../../conformance/vectors/p0/scalars.json", import.meta.url);

const schema = JSON.parse(readFileSync(schemaUrl, "utf8"));
const vectors = JSON.parse(readFileSync(vectorsUrl, "utf8")) as Array<{
  name: string;
  input: unknown;
  accept: boolean;
}>;

const ajv = new Ajv2020({ strict: false, allErrors: true });
ajv.addFormat("uint64", {
  type: "string",
  validate: (v: string) => /^[0-9]+$/.test(v) && BigInt(v) <= 18446744073709551615n,
});
ajv.addFormat("int64", {
  type: "string",
  validate: (v: string) =>
    /^-?[0-9]+$/.test(v) && BigInt(v) >= -9223372036854775808n && BigInt(v) <= 9223372036854775807n,
});
const validate = ajv.compile(schema);

test("shared scalar vectors match generated schema", () => {
  expect(vectors.length).toBeGreaterThan(0);
  for (const c of vectors) {
    const ok = validate(c.input);
    expect(ok, `${c.name}: ${JSON.stringify(validate.errors)}`).toBe(c.accept);
  }
});

test("ts-proto keeps uint64 as string, not Number", () => {
  const parsed = Scalars.fromJSON({ count: "18446744073709551615", enabled: false });
  expect(typeof parsed.count).toBe("string");
  expect(parsed.count).toBe("18446744073709551615");
  expect(parsed.enabled).toBe(false);
});

test("undefined and false are distinct", () => {
  const empty = Scalars.fromJSON({});
  expect(empty.count).toBeUndefined();
  const explicit = Scalars.fromJSON({ enabled: false });
  expect(explicit.enabled).toBe(false);
});
