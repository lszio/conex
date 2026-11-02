// 服务器视图：节点直连读取的真实采样（CPU/内存/磁盘）。样本自带时间与
// quality：stale 表示采样循环停了（节点侧 30 秒窗口），不是前端猜的。

import type { Workbench } from "@/hooks/use-workbench";

function bytes(value: number | undefined): string {
  if (value === undefined) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${scaled.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function SampleRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between border-b border-[var(--color-border)] py-1.5 last:border-0">
      <span className="text-xs text-[var(--color-muted-foreground)]">{label}</span>
      <span className="font-mono text-sm">{value}</span>
    </div>
  );
}

export function ServersView({ bench }: { bench: Workbench }) {
  const byKind = new Map<string, Record<string, unknown>>();
  for (const sample of bench.metrics) {
    // 同 kind 取最新一条：环形缓冲按时间追加，后写覆盖先写。
    byKind.set(String(sample.kind), sample);
  }
  const latest = (kind: string): Record<string, unknown> | undefined => byKind.get(kind);
  // stale 只看最新样本：历史样本转 stale 是有界保留的预期行为，不代表断连。
  const newest = bench.metrics.reduce<Record<string, unknown> | null>((acc, sample) => {
    const current = Number(sample.sampledAtMs ?? 0);
    const best = Number(acc?.sampledAtMs ?? 0);
    return current > best ? sample : acc;
  }, null);
  const newestStale = newest?.quality === "stale";
  const sampledAt = latest("cpu_percent")?.sampledAtMs;

  return (
    <div className="grid gap-4">
      {!bench.node ? (
        <p className="text-sm text-[var(--color-muted-foreground)]">目录中没有节点。</p>
      ) : bench.metrics.length === 0 ? (
        <p className="rounded-md border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-muted-foreground)]">
          还没有采样数据：等待节点 metrics 模块的首个样本（每 5 秒一次），或检查直连状态。
        </p>
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          <section className="rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-4">
            <h2 className="mb-2 text-sm font-semibold">CPU / 内存 / 磁盘</h2>
            <SampleRow
              label="CPU"
              value={`${Number(latest("cpu_percent")?.value ?? 0).toFixed(1)} %`}
            />
            <SampleRow
              label="内存"
              value={`${bytes(Number(latest("memory_used_bytes")?.value))} / ${bytes(Number(latest("memory_total_bytes")?.value))}`}
            />
            <SampleRow
              label="磁盘（/）"
              value={`${bytes(Number(latest("disk_available_bytes")?.value))} 可用 / 共 ${bytes(Number(latest("disk_total_bytes")?.value))}`}
            />
            <SampleRow
              label="平台"
              value={String(latest("cpu_percent")?.platform ?? latest("memory_used_bytes")?.platform ?? "—")}
            />
            <SampleRow
              label="采样时间"
              value={sampledAt ? new Date(Number(sampledAt)).toLocaleTimeString() : "—"}
            />
          </section>
          <section className="rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-4">
            <h2 className="mb-2 text-sm font-semibold">样本质量</h2>
            <p className="text-sm">
              {newestStale ? (
                <span className="text-[var(--color-destructive)]">
                  有样本已过期：节点采样循环停止超过 30 秒（节点可能正在重启或直连断开）。
                </span>
              ) : (
                <span className="text-[var(--color-success)]">全部样本为 live，来自节点真实 OS 采集。</span>
              )}
            </p>
            <p className="mt-3 text-xs text-[var(--color-muted-foreground)]">
              采样每 5 秒一次；节点断开后投影转 stale 而不是删除，恢复后自动续采。
            </p>
          </section>
        </div>
      )}
    </div>
  );
}
