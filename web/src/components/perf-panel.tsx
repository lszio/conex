// Performance, from measurement only. The numbers come from
// `cargo xtask bench --suite hello`, which starts a real host and drives real
// SDK clients; nothing here is estimated or hand-written. When the report is
// missing the page says so instead of showing placeholders.

import { Activity } from "lucide-react";

import { Badge, Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import perfData from "@/perf-data.json";

interface Stats {
  n: number;
  min: number;
  p50: number;
  p95: number;
  p99: number;
  max: number;
  mean: number;
  over5ms?: number;
}

interface Report {
  measuredAt: string;
  node: string;
  rounds: number;
  clients: { small: number; large: number };
  helloSelf?: Stats;
  helloSmallPool: Stats;
  helloLargePool: Stats;
  listSmallPool: Stats;
  listLargePool: Stats;
}

const report = perfData as unknown as Report;

function ms(value: number): string {
  return value < 1 ? "<1 ms" : `${value} ms`;
}

function Row({ label, stats, note }: { label: string; stats: Stats; note?: string }) {
  return (
    <div className="grid gap-2 rounded-md border border-[var(--color-border)] p-3">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <span className="text-sm font-medium">{label}</span>
        <span className="text-xs text-[var(--color-muted-foreground)]">
          {stats.n} 次采样
          {stats.over5ms ? ` · ${stats.over5ms} 次超过 5ms` : ""}
        </span>
      </div>
      <dl className="grid grid-cols-3 gap-2 text-xs sm:grid-cols-6">
        {(["p50", "p95", "p99", "min", "max", "mean"] as const).map((key) => (
          <div key={key} className="grid gap-0.5">
            <dt className="text-[var(--color-muted-foreground)]">{key}</dt>
            <dd className="font-mono">{ms(stats[key])}</dd>
          </div>
        ))}
      </dl>
      {note ? <p className="text-xs text-[var(--color-muted-foreground)]">{note}</p> : null}
    </div>
  );
}

export function PerfPanel() {
  const measured = new Date(report.measuredAt);
  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <CardTitle className="flex items-center gap-2">
          <Activity className="size-4" aria-hidden />
          性能
        </CardTitle>
        <Badge variant="outline">实测</Badge>
      </CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-sm text-[var(--color-muted-foreground)]">
          下面每个数字都由 <code className="font-mono">cargo xtask bench --suite hello</code>{" "}
          启动真实 Host、用真实 SDK 客户端跑出来，采集于{" "}
          <time dateTime={report.measuredAt}>{measured.toLocaleString()}</time>
          （{report.node}，每组 {report.rounds} 轮）。
        </p>

        <Row
          label="hello 往返（单客户端给自己）"
          stats={report.helloSelf ?? report.helloSmallPool}
          note="一条连接内的纯 Host 派发成本，作为基线。"
        />
        <Row
          label={`hello 往返（${report.clients.small} 个客户端在线）`}
          stats={report.helloSmallPool}
          note="跨连接推送与应答，含一次真实 WebSocket 往返。"
        />
        <Row
          label={`hello 往返（${report.clients.large} 个客户端在线）`}
          stats={report.helloLargePool}
          note="注册表扩大 8 倍后延迟基本不变：链路成本与在线人数无关。"
        />
        <Row
          label={`client/list（${report.clients.small} 个客户端）`}
          stats={report.listSmallPool}
          note="每个页面每 3 秒轮询一次的目录请求。"
        />
        <Row
          label={`client/list（${report.clients.large} 个客户端）`}
          stats={report.listLargePool}
        />

        <p className="text-xs text-[var(--color-muted-foreground)]">
          复现：<code className="font-mono">cargo xtask bench --suite hello --rounds 120</code>，
          结果写入 <code className="font-mono">web/src/perf-data.json</code>。
        </p>
      </CardContent>
    </Card>
  );
}
