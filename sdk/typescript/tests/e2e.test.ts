//! Runs only when CONEX_E2E_URL/TOKEN are set by `cargo xtask e2e --suite p0-ts`.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

import { ConexClient } from "../src/client";

const url = process.env.CONEX_E2E_URL;
const token = process.env.CONEX_E2E_TOKEN;
const enabled = Boolean(url && token);

const cidVectors = JSON.parse(
  readFileSync(new URL("../../../conformance/vectors/p0/cid.json", import.meta.url), "utf8"),
) as Array<{ input_utf8: string; cid: string }>;

const maybe = enabled ? test : test.skip;

maybe("real rust host: read/list/search match the shared CID vectors", async () => {
  const client = await ConexClient.connect({
    url: url as string,
    tokenProvider: async () => token as string,
    requires: ["source/list", "source/read", "source/search"],
  });
  const read = await client.read("notes-local", { resourceId: "hello.md" });
  expect(read.text).toBe("hello conex\n");
  expect(read.cid).toBe(cidVectors.find((entry) => entry.input_utf8 === "hello conex\n")?.cid);

  const list = await client.list("notes-local", { root: "" });
  expect((list.items ?? []).length).toBeGreaterThanOrEqual(2);

  const search = await client.search("notes-local", { root: "", query: "conex" });
  expect((search.items ?? []).length).toBeGreaterThanOrEqual(1);
  client.close();
});
