// Notifications, in the corner, one at a time.
//
// A greeting is an event rather than a conversation: it arrives, it is read,
// it goes away. So these are not a transcript — a notice expires on its own,
// and only a question stays until someone answers it. That question is the
// one that carries a form: the sender asked for specific fields, and the
// confirm button answers with exactly those.
//
// The position matters as much as the shape. They sit top-right and never
// push the page around, so a hello arriving while someone is reading the
// client list does not move the thing they are about to click.

import { useEffect, useState } from "react";
import { Check, Hand, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/card";
import type { Toast } from "@/hooks/use-hello-page";

const TONE = {
  info: "border-l-[var(--color-primary)]",
  success: "border-l-[var(--color-success)]",
  error: "border-l-[var(--color-destructive)]",
} as const;

export function ToastStack({
  toasts,
  onAnswer,
  onDismiss,
}: {
  toasts: Toast[];
  onAnswer: (toast: Toast, values: Record<string, string>) => void;
  onDismiss: (id: number) => void;
}) {
  if (toasts.length === 0) return null;
  return (
    <div
      // `assertive` for everything: a hello is addressed to whoever is looking
      // at this tab, and a polite queue would let a burst bury the question
      // that actually needs an answer.
      role="alert"
      aria-live="assertive"
      aria-label="消息提示"
      className="pointer-events-none fixed right-4 top-4 z-50 flex w-[min(22rem,calc(100vw-2rem))] flex-col gap-2"
    >
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} onAnswer={onAnswer} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function ToastCard({
  toast,
  onAnswer,
  onDismiss,
}: {
  toast: Toast;
  onAnswer: (toast: Toast, values: Record<string, string>) => void;
  onDismiss: (id: number) => void;
}) {
  const [values, setValues] = useState<Record<string, string>>({});

  // The timer is per-toast and restarted if the entry is replaced, so a notice
  // never disappears while it is being read. A question has `ttlMs: 0` and no
  // timer at all: the host is waiting on a person, and a person is not a
  // timeout.
  useEffect(() => {
    if (toast.ttlMs <= 0) return;
    const timer = window.setTimeout(() => onDismiss(toast.id), toast.ttlMs);
    return () => window.clearTimeout(timer);
  }, [onDismiss, toast.id, toast.ttlMs]);

  const fields = toast.fields ?? [];

  return (
    <div
      className={`pointer-events-auto rounded-md border border-[var(--color-border)] border-l-4 ${TONE[toast.tone]} bg-[var(--color-card)] p-3 shadow-lg`}
    >
      <div className="flex items-start justify-between gap-2">
        <p className="flex min-w-0 items-center gap-1.5 text-sm font-medium">
          <Hand className="size-3.5 shrink-0" aria-hidden />
          <span className="truncate">{toast.title}</span>
        </p>
        <button
          type="button"
          onClick={() => onDismiss(toast.id)}
          className="shrink-0 rounded p-0.5 text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]"
          aria-label={`关闭提示：${toast.title}`}
        >
          <X className="size-3.5" aria-hidden />
        </button>
      </div>
      <p className="mt-1 break-words text-xs text-[var(--color-muted-foreground)]">{toast.body}</p>

      {toast.replyId ? (
        <form
          className="mt-2 grid gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            onAnswer(toast, values);
          }}
        >
          {fields.length === 0 ? (
            <p className="text-xs text-[var(--color-muted-foreground)]">
              对方没有指定字段，确认即作答。
            </p>
          ) : (
            fields.map((field) => (
              <div key={field} className="grid gap-1">
                <Label htmlFor={`answer-${toast.id}-${field}`} className="text-xs">
                  {field}
                </Label>
                <Input
                  id={`answer-${toast.id}-${field}`}
                  className="h-8 text-xs"
                  placeholder="填好后点确认"
                  value={values[field] ?? ""}
                  onChange={(event) =>
                    setValues((previous) => ({ ...previous, [field]: event.target.value }))
                  }
                />
              </div>
            ))
          )}
          <Button type="submit" size="sm" className="justify-self-start">
            <Check className="size-3.5" aria-hidden />
            确认并回复
          </Button>
        </form>
      ) : null}
    </div>
  );
}
