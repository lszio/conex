// Message log for hellos sent and received. Newest first, capped, and split by
// direction so an outbound greeting is never mistaken for a reply.

import { AlertTriangle, ArrowDownLeft, ArrowUpRight, Info } from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { LogEntry } from "@/hooks/use-hello-page";

const ICON = {
  out: ArrowUpRight,
  in: ArrowDownLeft,
  system: Info,
  error: AlertTriangle,
} as const;

const TONE = {
  out: "text-[var(--color-primary)]",
  in: "text-[var(--color-success)]",
  system: "text-[var(--color-muted-foreground)]",
  error: "text-[var(--color-destructive)]",
} as const;

export function MessageLog({ messages }: { messages: LogEntry[] }) {
  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <CardTitle>消息</CardTitle>
        <span className="text-xs text-[var(--color-muted-foreground)]">
          最近 {messages.length} 条
        </span>
      </CardHeader>
      <CardContent>
        {messages.length === 0 ? (
          <p className="rounded-md border border-dashed border-[var(--color-border)] p-6 text-center text-sm text-[var(--color-muted-foreground)]">
            还没有消息。给某个客户端点一次「发送 hello」。
          </p>
        ) : (
          <ul className="grid gap-1.5">
            {messages.map((entry) => {
              const Icon = ICON[entry.tone];
              return (
                <li
                  key={entry.id}
                  className="flex items-start gap-2 border-b border-[var(--color-border)] py-1.5 text-sm last:border-b-0"
                >
                  <Icon className={`mt-0.5 size-3.5 shrink-0 ${TONE[entry.tone]}`} aria-hidden />
                  <time
                    dateTime={new Date(entry.at).toISOString()}
                    className="shrink-0 font-mono text-xs text-[var(--color-muted-foreground)]"
                  >
                    {new Date(entry.at).toLocaleTimeString()}
                  </time>
                  <span className="min-w-0 break-words">{entry.text}</span>
                </li>
              );
            })}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
