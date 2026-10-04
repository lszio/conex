// The online client list, grouped by the group each client declared.

import type { ClientSummary } from "@conex/sdk";
import { Hand } from "lucide-react";

import { Badge, Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";

export interface ClientListProps {
  clients: ClientSummary[];
  selfLinkId: string;
  onGreet: (target: ClientSummary) => void;
}

function label(row: ClientSummary): string {
  return row.profile?.displayName || row.linkId?.slice(0, 6) || "?";
}

function seen(ms: string | undefined): string {
  const value = Number(ms ?? "0");
  if (!Number.isFinite(value) || value <= 0) return "刚刚";
  const seconds = Math.max(0, Math.round((Date.now() - value) / 1000));
  if (seconds < 60) return `${seconds} 秒前活跃`;
  if (seconds < 3600) return `${Math.round(seconds / 60)} 分钟前活跃`;
  return `${Math.round(seconds / 3600)} 小时前活跃`;
}

export function ClientList({ clients, selfLinkId, onGreet }: ClientListProps) {
  // Group first, then name: the grouping a visitor chose is what they should
  // see their own page through.
  const sorted = [...clients].sort((a, b) => {
    const groupA = a.profile?.group || "";
    const groupB = b.profile?.group || "";
    if (groupA !== groupB) return groupA.localeCompare(groupB);
    return label(a).localeCompare(label(b));
  });

  const groups = new Map<string, ClientSummary[]>();
  for (const row of sorted) {
    const key = row.profile?.group || "未分组";
    const bucket = groups.get(key);
    if (bucket) bucket.push(row);
    else groups.set(key, [row]);
  }

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <CardTitle>在线客户端</CardTitle>
        <Badge variant="secondary">{clients.length} 个</Badge>
      </CardHeader>
      <CardContent>
        {clients.length === 0 ? (
          <p className="rounded-md border border-dashed border-[var(--color-border)] p-6 text-center text-sm text-[var(--color-muted-foreground)]">
            还没有其他可见的客户端。再开一个页面就能互相 hello。
          </p>
        ) : (
          <div className="grid gap-4">
            {[...groups.entries()].map(([group, rows]) => (
              <section key={group} className="grid gap-2">
                <h4 className="text-xs font-semibold uppercase tracking-wide text-[var(--color-muted-foreground)]">
                  {group}
                </h4>
                <ul className="grid gap-2">
                  {rows.map((row) => {
                    const isSelf = row.linkId === selfLinkId;
                    return (
                      <li
                        key={row.linkId}
                        className="flex items-center justify-between gap-3 rounded-md border border-[var(--color-border)] px-3 py-2.5"
                      >
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="truncate text-sm font-medium">{label(row)}</span>
                          {isSelf ? <Badge variant="outline">你</Badge> : null}
                          <span className="hidden truncate text-xs text-[var(--color-muted-foreground)] sm:inline">
                            {seen(row.lastSeenAtMs)}
                          </span>
                        </div>
                        <Button variant="secondary" size="sm" onClick={() => onGreet(row)}>
                          <Hand className="size-3.5" aria-hidden />
                          {isSelf ? "hello 给自己" : "发送 hello"}
                        </Button>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
