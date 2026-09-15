//! Client protocol tests against a scripted fake fetch (no network).
import { expect, test } from "bun:test";

import { ConexClient, type ClientFetch } from "../src/client";
import { ConexError } from "../src/errors";

const CID = "bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy";

function jsonrpc(id: unknown, result: unknown): Response {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id, result }), {
    headers: { "content-type": "application/json" },
  });
}

function rpcError(id: unknown, error: unknown): Response {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id, error }), {
    headers: { "content-type": "application/json" },
  });
}

function helloResult(bindingId = "binding-a") {
  return {
    bindingId,
    expiresInMs: 60000,
    profileId: "conex-jsonrpc2-http-v1",
    plane: "broker",
    provides: ["source/list", "source/read", "source/search"],
    rejectedCapabilities: [],
    limits: {},
  };
}

function recorder(handler: (request: any, headers: Headers) => Response) {
  const seen: any[] = [];
  const headers: Headers[] = [];
  const fetchImpl: ClientFetch = async (_url, init) => {
    const request = JSON.parse(String(init?.body));
    const head = new Headers(init?.headers);
    seen.push(request);
    headers.push(head);
    return handler(request, head);
  };
  return { fetch: fetchImpl, seen, headers };
}

function makeClient(fetchImpl: ClientFetch, extra: Record<string, unknown> = {}) {
  return ConexClient.connect({
    url: "https://host.example/rpc",
    tokenProvider: async () => "test-only-token",
    fetch: fetchImpl,
    ...extra,
  });
}

test("read preserves resource and keeps the token out of JSON", async () => {
  const { fetch, seen, headers } = recorder((request) => {
    if (request.method === "conex/hello") {
      return jsonrpc(request.id, helloResult());
    }
    return jsonrpc(request.id, {
      resource: { resourceId: "hello.md", title: "Hello", mime: "text/markdown" },
      text: "hello conex\n",
      cid: CID,
    });
  });
  const client = await makeClient(fetch);
  const result = await client.read("notes-local", { resourceId: "hello.md" });
  expect(result.text).toBe("hello conex\n");
  expect(result.cid).toBe(CID);
  expect(JSON.stringify(seen)).not.toContain("test-only-token");
  expect(headers[0].get("authorization")).toBe("Bearer test-only-token");
  client.close();
});

test("business calls use context/input and correlate ULID ids", async () => {
  const { fetch, seen } = recorder((request) =>
    request.method === "conex/hello" ? jsonrpc(request.id, helloResult()) : jsonrpc(request.id, { items: [] }),
  );
  const client = await makeClient(fetch);
  await client.list("notes-local", { root: "" });
  for (const request of seen) {
    expect(request.id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
  }
  const business = seen.find((request) => request.method === "source/list");
  expect(business.params.context.providerEndpointId).toBe("notes-local");
  expect(business.params.context.plane).toBe("broker");
  expect(business.params.context.bindingId).toBe("binding-a");
  expect(business.params.input).toEqual({ root: "" });
  expect(business.jsonrpc).toBe("2.0");
  client.close();
});

test("errors keep code, diagnostic fields and http status", async () => {
  const { fetch } = recorder((request) =>
    request.method === "conex/hello"
      ? jsonrpc(request.id, helloResult())
      : rpcError(request.id, {
          code: -32002,
          message: "denied",
          data: { code: "forbidden", diagnosticId: "d1", execution: "not_started", retry: "never" },
        }),
  );
  const client = await makeClient(fetch);
  try {
    await client.read("notes-local", { resourceId: "x" });
    throw new Error("expected failure");
  } catch (error) {
    const conex = error as ConexError;
    expect(conex.code).toBe(-32002);
    expect(conex.diagnosticId).toBe("d1");
    expect(conex.execution).toBe("not_started");
    expect(conex.retry).toBe("never");
    expect(conex.status).toBe(200);
  }
  client.close();
});

test("searchMany keeps successful and failed targets apart", async () => {
  const { fetch } = recorder((request) => {
    if (request.method === "conex/hello") {
      return jsonrpc(request.id, helloResult());
    }
    if (request.params.context.providerEndpointId === "bad") {
      return rpcError(request.id, { code: -32005, message: "unavailable", data: { code: "unavailable" } });
    }
    return jsonrpc(request.id, { items: [] });
  });
  const client = await makeClient(fetch);
  const results = await client.searchMany([
    { endpointId: "good", input: { root: "", query: "x" } },
    { endpointId: "bad", input: { root: "", query: "x" } },
  ]);
  expect(results.length).toBe(2);
  expect(results[0].result).toBeDefined();
  expect(results[0].error).toBeUndefined();
  expect(results[1].result).toBeUndefined();
  expect(results[1].error?.code).toBe(-32005);
  client.close();
});

test("oversized responses are rejected while streaming", async () => {
  const big = "x".repeat(2 * 1024 * 1024);
  const { fetch } = recorder((request) =>
    request.method === "conex/hello" ? jsonrpc(request.id, helloResult()) : jsonrpc(request.id, { text: big }),
  );
  const client = await makeClient(fetch, { maxResponseBytes: 1024 });
  await expect(client.read("notes-local", { resourceId: "x" })).rejects.toMatchObject({ code: -32007 });
  client.close();
});

test("a message with both result and error surfaces the error", async () => {
  const { fetch } = recorder((request) => {
    if (request.method === "conex/hello") {
      return jsonrpc(request.id, helloResult());
    }
    return new Response(
      JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { text: "x" }, error: { code: -32603, message: "both" } }),
      { headers: { "content-type": "application/json" } },
    );
  });
  const client = await makeClient(fetch);
  await expect(client.read("notes-local", { resourceId: "x" })).rejects.toMatchObject({ code: -32603 });
  client.close();
});

test("null fields are not silently coerced", async () => {
  const { fetch } = recorder((request) =>
    request.method === "conex/hello"
      ? jsonrpc(request.id, helloResult())
      : jsonrpc(request.id, { resource: null, text: null, cid: null }),
  );
  const client = await makeClient(fetch);
  const result = (await client.read("notes-local", { resourceId: "x" })) as any;
  expect(result.resource).toBeNull();
  expect(result.text).toBeNull();
  client.close();
});

test("persistent 401 stops after two attempts that share the deadline", async () => {
  let calls = 0;
  const fetch: ClientFetch = async () => {
    calls += 1;
    return new Response("unauthorized", { status: 401 });
  };
  const client = await makeClient(fetch);
  await expect(client.read("notes-local", { resourceId: "x" })).rejects.toMatchObject({ status: 401 });
  expect(calls).toBe(2);
  client.close();
});

test("an expired binding is refreshed once and the read retried", async () => {
  let business = 0;
  let hellos = 0;
  const { fetch } = recorder((request) => {
    if (request.method === "conex/hello") {
      hellos += 1;
      return jsonrpc(request.id, helloResult());
    }
    business += 1;
    if (business === 1) {
      return rpcError(request.id, { code: -32001, message: "binding expired", data: { code: "unauthorized" } });
    }
    return jsonrpc(request.id, { text: "ok", resource: null, cid: CID });
  });
  const client = await makeClient(fetch);
  const result = await client.read("notes-local", { resourceId: "x" });
  expect(result.text).toBe("ok");
  expect(business).toBe(2);
  expect(hellos).toBe(2);
  client.close();
});

test("close aborts in-flight work", async () => {
  const fetch: ClientFetch = async (_url, init) => {
    if (init?.signal?.aborted) {
      throw new Error("aborted");
    }
    return new Promise((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(new Error("aborted")));
    });
  };
  const client = await makeClient(fetch);
  const pending = client.read("notes-local", { resourceId: "x" });
  client.close();
  await expect(pending).rejects.toBeDefined();
});
