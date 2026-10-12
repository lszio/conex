# @conex/sdk

TypeScript SDK for [conex](https://github.com/lszio/conex): the broker-plane
WSS client (`ConexWsClient`), the content-plane helpers, and the generated
protocol types shared with the Rust core.

```bash
npm install @conex/sdk
```

```ts
import { ConexWsClient } from "@conex/sdk";

const client = new ConexWsClient({ origin: "https://your-host", csrfToken });
await client.connect();
await client.sendHello(targetLinkId, "hello");
```

The client speaks JSON-RPC 2.0 over WebSocket against a conex host and keeps
the same contract the landing page uses: `conex/hello` → `conex/ready`, then
`client/*`, `source/*` and `blob/*` methods with ULID request ids and
decimal-string `uint32`/`uint64` values.

MIT licensed. See the [repository](https://github.com/lszio/conex) for the
Rust workspace, the standalone host, and the contract docs.
