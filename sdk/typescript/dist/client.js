//! Typed HTTP consumer for conex. Business params go in context/input; the
//! bearer token only ever appears in the Authorization header.
import { ConexError, toConexError } from "./errors";
const PROFILE_ID = "conex-jsonrpc2-http";
const DEFAULT_TIMEOUT_MS = 8000;
const DEFAULT_MAX_RESPONSE_BYTES = 1024 * 1024;
const ULID_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export function mintUlid(now = Date.now()) {
    const bytes = new Uint8Array(16);
    let timestamp = BigInt(now);
    for (let index = 5; index >= 0; index -= 1) {
        bytes[index] = Number(timestamp & 0xffn);
        timestamp >>= 8n;
    }
    const random = new Uint8Array(10);
    globalThis.crypto.getRandomValues(random);
    bytes.set(random, 6);
    let bits = 0n;
    for (const byte of bytes) {
        bits = (bits << 8n) | BigInt(byte);
    }
    let out = "";
    for (let index = 0; index < 26; index += 1) {
        out = ULID_ALPHABET[Number(bits & 31n)] + out;
        bits >>= 5n;
    }
    return out;
}
export class ConexClient {
    url;
    tokenProvider;
    fetchImpl;
    profileId;
    requires;
    timeoutMs;
    maxResponseBytes;
    binding;
    closed = false;
    abort = new AbortController();
    constructor(options) {
        this.url = options.url;
        this.tokenProvider = options.tokenProvider;
        this.fetchImpl = options.fetch ?? ((url, init) => globalThis.fetch(url, init));
        this.profileId = options.profileId ?? PROFILE_ID;
        this.requires = options.requires ?? [];
        this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
        this.maxResponseBytes = options.maxResponseBytes ?? DEFAULT_MAX_RESPONSE_BYTES;
    }
    static async connect(options) {
        return new ConexClient(options);
    }
    list(endpointId, input) {
        return this.call("source/list", endpointId, input, true);
    }
    read(endpointId, input) {
        return this.call("source/read", endpointId, input, true);
    }
    search(endpointId, input) {
        return this.call("source/search", endpointId, input, true);
    }
    /// Independent per-target calls; one failure never discards the others.
    async searchMany(targets) {
        const results = [];
        for (const target of targets) {
            try {
                results.push({ endpointId: target.endpointId, result: await this.search(target.endpointId, target.input) });
            }
            catch (error) {
                results.push({ endpointId: target.endpointId, error: toConexError(error) });
            }
        }
        return results;
    }
    close() {
        this.closed = true;
        this.abort.abort();
    }
    async call(method, endpointId, input, readOnly) {
        const deadline = Date.now() + this.timeoutMs;
        let attempt = 0;
        for (;;) {
            attempt += 1;
            try {
                await this.ensureBinding(deadline);
                const requestId = mintUlid();
                const response = await this.post(this.envelope(method, endpointId, input, requestId), deadline);
                const json = await this.parse(response);
                if (json && json.error) {
                    throw errorFrom(json.error, response.status);
                }
                return json.result;
            }
            catch (error) {
                const conex = toConexError(error);
                const expired = conex.status === 401 || conex.code === -32001;
                if (readOnly && attempt < 2 && expired) {
                    this.binding = undefined;
                    continue;
                }
                throw conex;
            }
        }
    }
    async ensureBinding(deadline) {
        if (this.binding && Date.now() < this.binding.expiresAt - 1000) {
            return;
        }
        const requestId = mintUlid();
        const hello = {
            profileId: this.profileId,
            plane: "broker",
            provides: [],
            requires: this.requires,
        };
        const response = await this.post(this.envelope("conex/hello", "", hello, requestId), deadline);
        const json = await this.parse(response);
        if (json && json.error) {
            throw errorFrom(json.error, response.status);
        }
        const result = json.result;
        if (!result || typeof result.bindingId !== "string") {
            throw new ConexError("hello response is missing bindingId", { status: response.status });
        }
        this.binding = { id: result.bindingId, expiresAt: Date.now() + (result.expiresInMs ?? 60000) };
    }
    envelope(method, endpointId, input, requestId) {
        const context = { providerEndpointId: endpointId, plane: "broker" };
        if (this.binding && method !== "conex/hello") {
            context.bindingId = this.binding.id;
        }
        return {
            jsonrpc: "2.0",
            id: requestId,
            method,
            params: { context, timeoutBudgetMs: this.timeoutMs, input },
        };
    }
    async post(body, deadline) {
        if (this.closed) {
            throw new ConexError("client is closed", { code: -32011 });
        }
        const remaining = deadline - Date.now();
        if (remaining <= 0) {
            throw new ConexError("deadline exceeded before request", { code: -32006 });
        }
        const token = await this.tokenProvider();
        const controller = new AbortController();
        const onAbort = () => controller.abort();
        this.abort.signal.addEventListener("abort", onAbort, { once: true });
        if (this.abort.signal.aborted) {
            onAbort();
        }
        const timer = setTimeout(() => controller.abort(), remaining);
        try {
            const init = {
                method: "POST",
                headers: { "content-type": "application/json", authorization: "Bearer " + token },
                body: JSON.stringify(body),
                signal: controller.signal,
                redirect: "error",
            };
            return await this.fetchImpl(this.url, init);
        }
        catch (error) {
            if (this.closed) {
                throw new ConexError("client is closed", { code: -32011 });
            }
            throw new ConexError("transport error", { details: error instanceof Error ? error.name : String(error) });
        }
        finally {
            clearTimeout(timer);
            this.abort.signal.removeEventListener("abort", onAbort);
        }
    }
    async parse(response) {
        if (response.status === 401) {
            throw new ConexError("unauthorized", { status: 401, code: -32001 });
        }
        const text = await this.readCapped(response);
        if (!text && response.status >= 400) {
            throw new ConexError("http status " + response.status, { status: response.status });
        }
        try {
            return JSON.parse(text);
        }
        catch {
            throw new ConexError("invalid JSON response", { status: response.status });
        }
    }
    async readCapped(response) {
        const body = response.body;
        if (!body) {
            const text = await response.text();
            if (new TextEncoder().encode(text).length > this.maxResponseBytes) {
                throw new ConexError("response exceeds maxResponseBytes", { code: -32007 });
            }
            return text;
        }
        const reader = body.getReader();
        const chunks = [];
        let total = 0;
        for (;;) {
            const { done, value } = await reader.read();
            if (done) {
                break;
            }
            total += value.byteLength;
            if (total > this.maxResponseBytes) {
                await reader.cancel().catch(() => undefined);
                throw new ConexError("response exceeds maxResponseBytes", { code: -32007 });
            }
            chunks.push(value);
        }
        const merged = new Uint8Array(total);
        let offset = 0;
        for (const chunk of chunks) {
            merged.set(chunk, offset);
            offset += chunk.byteLength;
        }
        return new TextDecoder().decode(merged);
    }
}
function errorFrom(error, status) {
    const data = (error && error.data) || {};
    return new ConexError(error?.message ?? "rpc error", {
        code: typeof error?.code === "number" ? error.code : undefined,
        diagnosticId: data.diagnosticId,
        execution: data.execution,
        retry: data.retry,
        details: data.details,
        status,
    });
}
//# sourceMappingURL=client.js.map