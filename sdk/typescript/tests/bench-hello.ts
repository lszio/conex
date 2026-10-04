// Measures the hello path against a running host: round-trip percentiles at
// small and large client counts, plus the cost of the list every page polls.
// Emits JSON so the performance page shows measurements, not claims.
//
// Run via `cargo xtask bench --suite hello`, which starts a real host.

import { ConexWsClient, type WebSocketConstructor } from "@conex/sdk";

const origin = process.env.CONEX_BENCH_ORIGIN ?? "";
if (!origin) {
  console.error("CONEX_BENCH_ORIGIN is required");
  process.exit(1);
}
const outPath = process.env.CONEX_BENCH_OUT;
const SMALL = Number(process.env.CONEX_BENCH_SMALL ?? "8");
const LARGE = Number(process.env.CONEX_BENCH_LARGE ?? "64");
const ROUNDS = Number(process.env.CONEX_BENCH_ROUNDS ?? "60");

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[index];
}

function summarize(samples: number[]) {
  const sorted = [...samples].sort((a, b) => a - b);
  const sum = sorted.reduce((total, value) => total + value, 0);
  return {
    n: sorted.length,
    min: sorted[0] ?? 0,
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
    max: sorted.at(-1) ?? 0,
    mean: sorted.length ? Math.round((sum / sorted.length) * 100) / 100 : 0,
    // Outliers are reported rather than averaged away: a p95 far from a p50
    // is a fact about the system, and hiding it inside a mean would make the
    // performance page misleading.
    over5ms: sorted.filter((value) => value > 5).length,
  };
}

interface BenchClient {
  client: ConexWsClient;
  linkId: string;
}

async function open(name: string): Promise<BenchClient> {
  const session = await fetch(`${origin}/web/session`);
  const { csrf } = (await session.json()) as { csrf: string };
  const cookie = (session.headers.get("set-cookie") || "").split(";")[0];
  const client = new ConexWsClient({
    origin,
    csrfToken: csrf,
    reconnect: false,
    // Bun's WebSocket omits Origin; browsers send it. The SDK types the
    // constructor for browsers, so the extra options need the cast.
    WebSocket: function (url: string) {
      return new (globalThis.WebSocket)(url, {
        headers: { origin },
      } as unknown as string[]) as unknown as WebSocket;
    } as unknown as WebSocketConstructor,
    fetch: (url, init = {}) => {
      const headers = new Headers(init.headers);
      headers.set("cookie", cookie);
      headers.set("origin", origin);
      return fetch(url, { ...init, headers });
    },
  });
  await client.connect();
  const self = await client.setProfile({ displayName: name, group: "bench", visible: true });
  return { client, linkId: self.self?.linkId ?? "" };
}

async function openMany(count: number): Promise<BenchClient[]> {
  const clients: BenchClient[] = [];
  for (let i = 0; i < count; i += 1) {
    clients.push(await open(`bench-${i}`));
  }
  return clients;
}

function closeAll(clients: BenchClient[]): void {
  for (const { client } of clients) client.close();
}

async function measureHello(senders: BenchClient[], target: BenchClient, rounds: number) {
  // Warm before measuring. Bun pays a one-time ~40 ms cost on the first send
  // over each *additional* socket in a process; it is not request latency, so
  // leaving it in would misreport the system as having a 41 ms p95. The warmup
  // runs more than once per sender so a lazily-opened socket cannot leak into
  // the samples.
  for (let warm = 0; warm < 3; warm += 1) {
    for (const { client } of senders) await client.sendHello(target.linkId, "warmup");
  }
  const samples: number[] = [];
  for (let round = 0; round < rounds; round += 1) {
    for (let position = 0; position < senders.length; position += 1) {
      const started = performance.now();
      await senders[position]!.client.sendHello(target.linkId, "bench");
      samples.push(performance.now() - started);
    }
  }
  return summarize(samples.map((value) => Math.round(value * 100) / 100));
}

async function measureList(clients: BenchClient[], rounds: number) {
  const samples: number[] = [];
  for (let round = 0; round < rounds; round += 1) {
    for (const { client } of clients) {
      const started = performance.now();
      const result = await client.listClients();
      samples.push(performance.now() - started);
      if (round === 0) {
        console.error(`list returned ${result.clients?.length ?? 0} clients`);
      }
    }
  }
  return summarize(samples.map((value) => Math.round(value * 100) / 100));
}

const report: Record<string, unknown> = {
  measuredAt: new Date().toISOString(),
  node: process.version,
  rounds: ROUNDS,
};

// A self-hello uses one socket, so it isolates host dispatch latency from any
// cost of a second connection. If self and cross-socket share the same outlier
// shape, the stall is host-side, not a per-connection effect.
const solo = await open("bench-solo");
report.helloSelf = await measureHello([solo], solo, ROUNDS);
solo.client.close();

// Small pool: the common case, a handful of people on the page.
const small = await openMany(SMALL);
report.clients = { small: SMALL, large: LARGE };
report.helloSmallPool = await measureHello(small.slice(0, 4), small[0], ROUNDS);
report.listSmallPool = await measureList(small, 20);
// Per-sender spread: if one sender in a round is consistently slow the cost is
// per-connection, not per-request.
report.perSender = await Promise.all(
  small.slice(0, 4).map((sender, index) =>
    measureHello([sender], small[0], 20).then((stats) => ({ sender: index, ...stats })),
  ),
);
closeAll(small);

// Large pool: the headroom question — does a hello stay flat as the registry
// grows, and what does the polled list cost at that size?
const large = await openMany(LARGE);
report.helloLargePool = await measureHello(large.slice(0, 4), large[0], ROUNDS);
report.listLargePool = await measureList(large, 10);
closeAll(large);

const json = JSON.stringify(report, null, 2);
if (outPath) {
  await Bun.write(outPath, `${json}\n`);
  console.error(`wrote ${outPath}`);
}
console.log(json);
process.exit(0);
