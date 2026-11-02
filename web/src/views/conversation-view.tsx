// 会话视图：端点/Workspace/会话三级选择 + 消息流 + 提交/取消/审批。
//
// 状态一律来自投影（snapshot + events 去重续读），不本地猜：busy 看
// conversation.activeExecutionId 与执行状态；streaming 的消息显式标注；
// 审批是节点侧带 revision 的一次性 CAS，重复回答会被拒绝并如实显示。

import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import type { AgentInteractionRequest, AgentMessage } from "@conex/sdk";
import type { Workbench } from "@/hooks/use-workbench";

const EXECUTION_LABEL: Record<string, string> = {
  accepted: "已受理",
  dispatching: "派发中",
  running: "执行中",
  unknown: "状态未知",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

const ROLE_LABEL: Record<string, string> = {
  user: "你",
  assistant: "助手",
  tool: "工具",
  system: "系统",
};

function messageText(message: AgentMessage): string {
  return (message.parts ?? [])
    .map((part) => part.text?.text ?? (part.tool ? `[工具 ${part.tool.name ?? "?"}] ${part.tool.summary ?? ""}` : ""))
    .join("");
}

function InteractionBanner({
  bench,
  interaction,
}: {
  bench: Workbench;
  interaction: AgentInteractionRequest;
}) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const answer = async (decision: Record<string, unknown>) => {
    setError(null);
    try {
      await bench.respondInteraction(interaction.id ?? "", interaction.revision ?? "0", decision);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };
  const kind = interaction.kind ?? "approval";
  return (
    <div role="region" aria-label="等待人工响应" className="grid gap-2 rounded-md border border-[var(--color-primary)] bg-[var(--color-card)] p-3">
      <p className="text-sm font-medium">{interaction.prompt || "（后端未给出提示文本）"}</p>
      <p className="text-xs text-[var(--color-muted-foreground)]">
        {kind === "input" ? "需要补充输入" : kind === "selection" ? "需要选择" : "需要审批"} · 修订 {interaction.revision}
      </p>
      {kind === "input" ? (
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (text.trim()) void answer({ kind: "input", text: text.trim() });
          }}
        >
          <input
            value={text}
            onChange={(event) => setText(event.target.value)}
            className="min-w-0 flex-1 rounded-md border border-[var(--color-input)] bg-transparent px-3 py-1.5 text-sm outline-none focus:border-[var(--color-primary)]"
            autoFocus
          />
          <Button type="submit" size="sm" disabled={!text.trim()}>提交</Button>
        </form>
      ) : (
        <div className="flex flex-wrap gap-2">
          {(interaction.choices ?? []).map((choice) => (
            <Button
              key={choice.id}
              size="sm"
              className={
                choice.effect === "deny"
                  ? "bg-[var(--color-destructive)] text-[var(--color-destructive-foreground)]"
                  : undefined
              }
              onClick={() =>
                void answer(
                  kind === "selection"
                    ? { kind: "selection", choiceIds: [choice.id ?? ""] }
                    : { kind: "approval", choiceId: choice.id ?? "" },
                )
              }
            >
              {choice.label || choice.id}
            </Button>
          ))}
        </div>
      )}
      {error ? <p role="alert" className="text-xs text-[var(--color-destructive)]">{error}</p> : null}
    </div>
  );
}

export function ConversationView({ bench }: { bench: Workbench }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  // 完成态消息的完整正文缓存（快照窗口为空，正文经 messages/read 取回）。
  const [bodies, setBodies] = useState<Record<string, string>>({});
  const fetchingRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    fetchingRef.current = new Set();
    setBodies({});
  }, [bench.conversationId]);

  const projection = bench.projection;
  const messages = projection?.messages ?? [];

  useEffect(() => {
    if (!bench.conversationId) return;
    for (const message of messages) {
      const id = message.id ?? "";
      if (!id || message.state === "streaming" || bodies[id] !== undefined) continue;
      if (fetchingRef.current.has(id)) continue;
      fetchingRef.current.add(id);
      bench
        .readMessage(bench.conversationId, id)
        .then((body) => setBodies((previous) => ({ ...previous, [id]: body })))
        .catch(() => fetchingRef.current.delete(id));
    }
  }, [messages, bench.conversationId, bench, bodies]);

  const conversation = projection?.conversation;
  const activeExecution = (projection?.executions ?? []).find(
    (execution) =>
      execution.conversationId === bench.conversationId &&
      ["accepted", "dispatching", "running", "unknown"].includes(execution.state ?? ""),
  );
  // 最新执行（含终态）：状态行常显，终态不消失。
  const latestExecution = (projection?.executions ?? [])
    .filter((execution) => execution.conversationId === bench.conversationId)
    .at(-1);
  const pendingInteractions = (projection?.interactions ?? []).filter(
    (interaction) => interaction.state === "pending",
  );

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [messages.length, activeExecution?.state]);

  const send = async () => {
    if (!draft.trim() || busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await bench.submit(draft.trim());
      setDraft("");
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid gap-4 lg:grid-cols-[17rem_minmax(0,1fr)] lg:items-start">
      <aside className="grid gap-3">
        <label className="grid gap-1 text-xs">
          <span className="font-medium">Agent 端点</span>
          <select
            value={bench.endpointId ?? ""}
            onChange={(event) => {
              bench.setEndpointId(event.target.value || null);
              bench.setWorkspaceId(null);
              bench.setConversationId(null);
            }}
            className="rounded-md border border-[var(--color-input)] bg-transparent px-2 py-1.5 text-sm"
          >
            <option value="" disabled>选择端点</option>
            {bench.endpoints.map((endpoint) => (
              <option key={String(endpoint.id)} value={String(endpoint.id)}>
                {String(endpoint.displayName ?? endpoint.adapter ?? endpoint.id)}
              </option>
            ))}
          </select>
        </label>

        <label className="grid gap-1 text-xs">
          <span className="font-medium">Workspace</span>
          <select
            value={bench.workspaceId ?? ""}
            onChange={(event) => {
              bench.setWorkspaceId(event.target.value || null);
              bench.setConversationId(null);
            }}
            className="rounded-md border border-[var(--color-input)] bg-transparent px-2 py-1.5 text-sm"
          >
            <option value="" disabled>选择工作区</option>
            {bench.workspaces.map((workspace) => (
              <option key={String(workspace.id)} value={String(workspace.id)}>
                {String(workspace.displayName ?? workspace.id)}
              </option>
            ))}
          </select>
        </label>

        <div className="grid gap-1">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium">会话</span>
            <Button
              size="sm"
              variant="secondary"
              disabled={!bench.endpointId || !bench.workspaceId}
              onClick={() => {
                setActionError(null);
                bench.createConversation().catch((cause) =>
                  setActionError(cause instanceof Error ? cause.message : String(cause)),
                );
              }}
            >
              新建
            </Button>
          </div>
          <ul className="grid max-h-64 gap-1 overflow-y-auto">
            {bench.conversations.map((entry) => (
              <li key={String(entry.id)}>
                <button
                  type="button"
                  onClick={() => bench.setConversationId(String(entry.id))}
                  className={
                    "w-full truncate rounded-md px-2 py-1.5 text-left text-sm " +
                    (bench.conversationId === entry.id
                      ? "bg-[var(--color-secondary)] font-medium"
                      : "hover:bg-[var(--color-secondary)]")
                  }
                >
                  {String(entry.title || entry.id).slice(0, 28)}
                  {entry.activeExecutionId ? <span className="ml-1 text-[var(--color-primary)]">●</span> : null}
                </button>
              </li>
            ))}
            {bench.conversations.length === 0 ? (
              <li className="px-2 py-1 text-xs text-[var(--color-muted-foreground)]">
                {bench.endpointId ? "此 Workspace 还没有会话" : "先选择端点与 Workspace"}
              </li>
            ) : null}
          </ul>
        </div>
      </aside>

      <section className="grid min-h-[60vh] gap-3">
        {bench.conversationId && conversation ? (
          <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--color-muted-foreground)]">
            <span>
              会话 {conversation.id?.slice(0, 8)} · 投影 {bench.followerStatus}
              {bench.followerStatus === "stale" && bench.follower?.lastError ? (
                <span className="ml-2 text-[var(--color-destructive)]">
                  {bench.follower.lastError instanceof Error
                    ? bench.follower.lastError.message
                    : String(bench.follower.lastError)}
                </span>
              ) : null}
            </span>
            {latestExecution ? (
              <>
                <Badge variant={latestExecution.state === "unknown" ? "destructive" : latestExecution.state === "succeeded" ? "success" : "secondary"}>
                  {EXECUTION_LABEL[latestExecution.state ?? ""] ?? latestExecution.state}
                </Badge>
                {activeExecution && activeExecution.state !== "unknown" ? (
                  <Button
                    size="sm"
                    className="bg-[var(--color-destructive)] text-[var(--color-destructive-foreground)]"
                    onClick={() => {
                      setActionError(null);
                      bench
                        .cancelExecution(activeExecution.id ?? "", bench.agentGeneration ?? "0")
                        .catch((cause) =>
                          setActionError(cause instanceof Error ? cause.message : String(cause)),
                        );
                    }}
                  >
                    取消执行
                  </Button>
                ) : activeExecution ? (
                  <span>状态未知期间不能发起新提交，也不自动重试</span>
                ) : null}
              </>
            ) : null}
          </div>
        ) : null}

        {pendingInteractions.length > 0 && bench.conversationId ? (
          <InteractionBanner bench={bench} interaction={pendingInteractions[0]} />
        ) : null}

        <div className="grid content-start gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-4">
          {!bench.conversationId ? (
            <p className="py-10 text-center text-sm text-[var(--color-muted-foreground)]">
              从左侧选择或新建一个会话。
            </p>
          ) : (
            messages.map((message) => (
              <article key={message.id} className="grid gap-1">
                <div className="flex items-center gap-2 text-xs text-[var(--color-muted-foreground)]">
                  <span className="font-medium text-[var(--color-foreground)]">
                    {ROLE_LABEL[message.role ?? ""] ?? message.role}
                  </span>
                  {message.state === "streaming" ? <span>正在输入…</span> : null}
                  {message.state === "incomplete" ? <span className="text-[var(--color-destructive)]">不完整</span> : null}
                </div>
                <div className="whitespace-pre-wrap break-words text-sm leading-relaxed">
                  {message.state === "streaming"
                    ? messageText(message)
                    : (bodies[message.id ?? ""] ?? messageText(message))}
                  {message.state === "streaming" ? <span aria-hidden>▍</span> : null}
                </div>
              </article>
            ))
          )}
          <div ref={bottomRef} />
        </div>

        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            void send();
          }}
        >
          <input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            disabled={!bench.conversationId || Boolean(activeExecution)}
            placeholder={
              activeExecution
                ? "执行进行中：同一会话一次只有一个顶层执行"
                : bench.conversationId
                  ? "输入要发给 Agent 的内容"
                  : "先选择会话"
            }
            className="min-w-0 flex-1 rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)]"
          />
          <Button type="submit" disabled={!bench.conversationId || busy || !draft.trim() || Boolean(activeExecution)}>
            发送
          </Button>
        </form>
        {actionError ? <p role="alert" className="text-xs text-[var(--color-destructive)]">{actionError}</p> : null}
      </section>
    </div>
  );
}

function Badge({ children, variant }: { children: React.ReactNode; variant: string }) {
  const styles: Record<string, string> = {
    secondary: "bg-[var(--color-secondary)] text-[var(--color-secondary-foreground)]",
    destructive: "bg-[var(--color-destructive)] text-[var(--color-destructive-foreground)]",
    success: "bg-[var(--color-success)] text-white",
  };
  return (
    <span className={"rounded px-1.5 py-0.5 text-xs " + (styles[variant] ?? styles.secondary)}>
      {children}
    </span>
  );
}
