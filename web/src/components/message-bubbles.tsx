// The hello scene's stage: greetings as message bubbles.
//
// A hello is a request that crosses the host and comes back, so the bubble
// shows both ends — who sent it and, for an outgoing one, how long the round
// trip actually took. A greeting that has not answered yet says so rather than
// showing a latency of zero.

import { Hand } from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { Bubble } from "@/hooks/use-hello-page";

function roundTrip(ms: number | undefined): string {
  if (ms === undefined) return "等待回应…";
  // A sub-millisecond round trip is a real measurement that truncates to 0;
  // showing "0 ms" would read as a failure instead of a fast answer.
  return ms < 1 ? "<1 ms" : `${ms} ms`;
}

export function MessageBubbles({
  bubbles,
  selfLabel,
  peerCount,
}: {
  bubbles: Bubble[];
  selfLabel: string;
  peerCount: number;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>消息泡泡</CardTitle>
      </CardHeader>
      <CardContent>
        {bubbles.length === 0 ? (
          <p className="rounded-md border border-dashed border-[var(--color-border)] p-6 text-center text-sm text-[var(--color-muted-foreground)]">
            {peerCount === 0
              ? "还没有同组的其他客户端。换个一样的组名，再开一个页面。"
              : "还没有消息。在下面的客户端列表里点一次「发送 hello」。"}
          </p>
        ) : (
          <ul className="flex flex-col gap-2" aria-live="polite">
            {bubbles.map((bubble) => {
              const mine = bubble.direction === "out";
              return (
                <li
                  key={bubble.id}
                  className={`flex ${mine ? "justify-end" : "justify-start"}`}
                >
                  <div
                    className={`max-w-[80%] rounded-2xl px-3.5 py-2 text-sm ${
                      mine
                        ? "rounded-br-sm bg-[var(--color-primary)] text-[var(--color-primary-foreground)]"
                        : "rounded-bl-sm border border-[var(--color-border)] bg-[var(--color-muted)]"
                    }`}
                  >
                    <p className={`text-xs ${mine ? "opacity-80" : "text-[var(--color-muted-foreground)]"}`}>
                      {mine ? "我 → " : `${bubble.from} → `}
                      <time dateTime={new Date(bubble.at).toISOString()}>
                        {new Date(bubble.at).toLocaleTimeString()}
                      </time>
                    </p>
                    <p className="mt-0.5 break-words">{bubble.text}</p>
                    {mine ? (
                      <p className="mt-1 flex items-center justify-end gap-1 text-xs opacity-90">
                        <Hand className="size-3" aria-hidden />
                        {roundTrip(bubble.roundTripMs)}
                      </p>
                    ) : null}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        <p className="mt-3 text-xs text-[var(--color-muted-foreground)]">
          你是 {selfLabel}。收到的泡泡由对方页面直接显示，发送者名字由 Host 解析。
        </p>
      </CardContent>
    </Card>
  );
}
