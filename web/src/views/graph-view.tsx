// 图谱视图：与对话视图共用同一批实体 id（会话/执行/端点/Workspace），
// 关系来自投影（backend 来源），Tags 是用户标注（节点侧存储、可增删）。
// 关系线不授予权限；展示边界与来源，未知不伪造。

import { useState } from "react";

import { Button } from "@/components/ui/button";
import type { Workbench } from "@/hooks/use-workbench";

const RELATION_LABEL: Record<string, string> = {
  "runs-on": "运行于",
  workspace: "关联 Workspace",
  conversation: "参与会话",
  execution: "派生执行",
  subagent: "子代理",
};

function entityLabel(bench: Workbench, id: string | undefined): string {
  if (!id) return "未知";
  const conversation = bench.projection?.conversation;
  if (conversation?.id === id) return `会话 ${id.slice(0, 8)}`;
  const execution = bench.projection?.executions?.find((entry) => entry.id === id);
  if (execution) return `执行 ${id.slice(0, 8)}（${execution.state ?? "?"}）`;
  if (bench.endpointId === id) return "Agent 端点";
  if (bench.workspaceId === id) return "Workspace";
  return id.slice(0, 12);
}

export function GraphView({ bench }: { bench: Workbench }) {
  const [tagName, setTagName] = useState("");
  const [tagError, setTagError] = useState<string | null>(null);
  const projection = bench.projection;
  const relationships = projection?.relationships ?? [];

  if (!bench.conversationId) {
    return (
      <p className="text-sm text-[var(--color-muted-foreground)]">
        图谱展示当前会话的实体关系；先在“会话”视图选择一个会话。
      </p>
    );
  }

  const addTag = () => {
    if (!tagName.trim()) return;
    setTagError(null);
    bench
      .putTag("conversation", bench.conversationId ?? "", tagName.trim())
      .then(() => setTagName(""))
      .catch((cause) => setTagError(cause instanceof Error ? cause.message : String(cause)));
  };

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <section className="rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-4">
        <h2 className="mb-3 text-sm font-semibold">关系（来源：后端报告）</h2>
        {relationships.length === 0 ? (
          <p className="text-xs text-[var(--color-muted-foreground)]">
            此会话还没有已观察到的描述关系。执行发生后会出现在这里。
          </p>
        ) : (
          <ul className="grid gap-2">
            {relationships.map((relation) => (
              <li key={relation.id} className="rounded-md border border-[var(--color-border)] px-3 py-2 text-sm">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{RELATION_LABEL[relation.kind ?? ""] ?? relation.kind}</span>
                  <span className="text-[var(--color-muted-foreground)]">
                    {entityLabel(bench, relation.from)} → {entityLabel(bench, relation.to)}
                  </span>
                  <span className="ml-auto text-xs text-[var(--color-muted-foreground)]">
                    {relation.source} · {relation.quality}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}

        <h2 className="mb-3 mt-5 text-sm font-semibold">执行</h2>
        <ul className="grid gap-1">
          {(projection?.executions ?? []).map((execution) => (
            <li key={execution.id} className="flex items-center gap-2 text-sm">
              <span className="font-mono text-xs">{execution.id?.slice(0, 8)}</span>
              <span>{execution.state}</span>
              {execution.parentExecutionId ? (
                <span className="text-xs text-[var(--color-muted-foreground)]">
                  派生自 {execution.parentExecutionId.slice(0, 8)}
                </span>
              ) : null}
            </li>
          ))}
          {(projection?.executions ?? []).length === 0 ? (
            <li className="text-xs text-[var(--color-muted-foreground)]">还没有执行记录。</li>
          ) : null}
        </ul>
      </section>

      <section className="rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-4">
        <h2 className="mb-3 text-sm font-semibold">Tags（用户标注）</h2>
        <div className="flex flex-wrap gap-2">
          {bench.tags.map((tag) => (
            <span
              key={String(tag.name)}
              className="flex items-center gap-1 rounded-full bg-[var(--color-secondary)] px-3 py-1 text-sm"
            >
              {String(tag.name)}
              <button
                type="button"
                aria-label={`删除标注 ${String(tag.name)}`}
                className="text-xs text-[var(--color-muted-foreground)] hover:text-[var(--color-destructive)]"
                onClick={() => {
                  bench
                    .deleteTag("conversation", bench.conversationId ?? "", String(tag.name))
                    .catch((cause) => setTagError(cause instanceof Error ? cause.message : String(cause)));
                }}
              >
                ×
              </button>
            </span>
          ))}
          {bench.tags.length === 0 ? (
            <span className="text-xs text-[var(--color-muted-foreground)]">还没有标注。</span>
          ) : null}
        </div>
        <form
          className="mt-3 flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            addTag();
          }}
        >
          <input
            value={tagName}
            onChange={(event) => setTagName(event.target.value)}
            placeholder="添加标注"
            className="min-w-0 flex-1 rounded-md border border-[var(--color-input)] bg-transparent px-3 py-1.5 text-sm outline-none focus:border-[var(--color-primary)]"
          />
          <Button type="submit" size="sm" variant="secondary" disabled={!tagName.trim()}>
            添加
          </Button>
        </form>
        {tagError ? <p role="alert" className="mt-2 text-xs text-[var(--color-destructive)]">{tagError}</p> : null}
        <p className="mt-4 text-xs text-[var(--color-muted-foreground)]">
          Tags 是标注不是权限；删除标注不会影响任何后端执行。关系与能力绑定分别查询、分别授权。
        </p>
      </section>
    </div>
  );
}
