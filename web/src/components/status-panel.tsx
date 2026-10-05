// Status: how many clients and groups are on this host, and the latency
// actually measured in each group.
//
// The latency rule is the important one. A group where no hello has completed
// has no latency to show, and printing 0 ms there would look like a fast
// connection rather than a missing measurement — so unmeasured groups say so.

import { Activity, Users } from "lucide-react";

import { Badge, Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { GroupRow, StatusState } from "@/hooks/use-hello-page";

function latency(ms: number, roundTrips: number): string {
  if (roundTrips === 0) return "尚未测量";
  return ms < 1 ? "<1 ms" : `${ms} ms`;
}

function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

export function StatusPanel({
  status,
  ownGroupKey,
}: {
  status: StatusState;
  ownGroupKey: string;
}) {
  return (
    <div className="grid gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <Card>
          <CardHeader className="flex-row items-center gap-2 space-y-0 pb-3">
            <Users className="size-4 text-[var(--color-primary)]" aria-hidden />
            <CardTitle className="text-sm">在线客户端</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-semibold tabular-nums">{status.clientsOnline}</p>
            <p className="text-xs text-[var(--color-muted-foreground)]">本 Host 上可被寻址的连接</p>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="flex-row items-center gap-2 space-y-0 pb-3">
            <Activity className="size-4 text-[var(--color-primary)]" aria-hidden />
            <CardTitle className="text-sm">在线分组</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-semibold tabular-nums">{status.groupsOnline}</p>
            <p className="text-xs text-[var(--color-muted-foreground)]">按组名哈希区分，互不可见</p>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0">
          <CardTitle>各组延迟</CardTitle>
          <Badge variant="secondary">{status.groups.length} 组</Badge>
        </CardHeader>
        <CardContent>
          {status.groups.length === 0 ? (
            <p className="rounded-md border border-dashed border-[var(--color-border)] p-6 text-center text-sm text-[var(--color-muted-foreground)]">
              还没有客户端上线。
            </p>
          ) : (
            <ul className="grid gap-2">
              {status.groups.map((group: GroupRow) => {
                const mine = group.groupKey === ownGroupKey;
                return (
                  <li
                    key={group.groupKey}
                    className="grid gap-2 rounded-md border border-[var(--color-border)] px-3 py-2.5 sm:grid-cols-[minmax(0,1fr)_auto_auto_auto] sm:items-center"
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-sm font-medium">
                        {group.label || "未命名分组"}
                      </span>
                      {mine ? <Badge variant="default">你的组</Badge> : null}
                      <span className="truncate font-mono text-xs text-[var(--color-muted-foreground)]">
                        {group.groupKey.slice(0, 8)}
                      </span>
                    </div>
                    <span className="text-xs text-[var(--color-muted-foreground)]">
                      {group.clientsOnline} 客户端
                    </span>
                    <span className="text-xs tabular-nums">
                      最近 {latency(group.lastRoundTripMs, group.roundTrips)}
                    </span>
                    <span className="text-xs tabular-nums text-[var(--color-muted-foreground)]">
                      均值 {latency(group.avgRoundTripMs, group.roundTrips)}
                      {group.filesShared > 0 ? ` · ${group.filesShared} 个文件 / ${bytes(group.sharedBytes)}` : ""}
                    </span>
                  </li>
                );
              })}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
