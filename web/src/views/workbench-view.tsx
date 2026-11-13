// 工作台壳：顶部是会话/服务器/图谱三个真实视图的切换与连接状态；私有部署
// 没有"演示数据"可填空——每块区域要么有真实数据，要么说清为什么没有。

import { Badge } from "@/components/ui/card";
import { ConversationView } from "@/views/conversation-view";
import { ServersView } from "@/views/servers-view";
import { GraphView } from "@/views/graph-view";
import { ModelsView } from "@/views/models-view";
import { useWorkbench } from "@/hooks/use-workbench";
import type { PrivateHubClient } from "@conex/sdk";

const VIEWS = [
  ["conversation", "会话"],
  ["servers", "服务器"],
  ["models", "模型服务"],
  ["graph", "图谱"],
] as const;

function ChannelBadge({ state, detail }: { state: string; detail?: string }) {
  const variant = state === "ready" ? "success" : state === "reconnecting" ? "secondary" : "destructive";
  const label = state === "ready" ? "直连" : state === "reconnecting" ? "重连中" : state === "connecting" ? "连接中" : "离线";
  return (
    <span className="flex items-center gap-1">
      <Badge variant={variant}>{label}</Badge>
      {detail && state !== "ready" ? (
        <span className="max-w-48 truncate text-xs text-[var(--color-muted-foreground)]">{detail}</span>
      ) : null}
    </span>
  );
}

export function WorkbenchView({ hub }: { hub: PrivateHubClient }) {
  const bench = useWorkbench(hub);
  const channels = Object.entries(bench.channelInfo);

  return (
    <div className="mx-auto grid w-full max-w-6xl gap-4 px-4 py-6">
      <header className="grid gap-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h1 className="text-lg font-semibold tracking-tight">
            <span className="text-[var(--color-primary)]">conex</span>
            <span className="ml-2 font-normal text-[var(--color-muted-foreground)]">私有工作台</span>
          </h1>
          <nav className="flex gap-1" aria-label="视图">
            {VIEWS.map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => bench.setView(key)}
                aria-current={bench.view === key}
                className={
                  "rounded-md px-3 py-1.5 text-sm " +
                  (bench.view === key
                    ? "bg-[var(--color-primary)] text-[var(--color-primary-foreground)]"
                    : "text-[var(--color-muted-foreground)] hover:bg-[var(--color-secondary)]")
                }
              >
                {label}
              </button>
            ))}
          </nav>
        </div>

        {bench.directoryError ? (
          <p role="alert" className="rounded-md border border-[var(--color-destructive)] px-3 py-2 text-xs text-[var(--color-destructive)]">
            目录不可达：{bench.directoryError}（每 10 秒重试）
          </p>
        ) : null}

        {bench.node ? (
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-md border border-[var(--color-border)] bg-[var(--color-card)] px-3 py-2 text-xs">
            <span className="font-medium">{bench.node.principal}</span>
            <span className="font-mono text-[var(--color-muted-foreground)]">
              节点 {bench.node.nodeId.slice(0, 8)} · epoch {bench.node.nodeEpoch.slice(0, 8)}
            </span>
            {channels.length === 0 ? (
              <span className="text-[var(--color-muted-foreground)]">直连未建立</span>
            ) : (
              channels.map(([contract, info]) => (
                <ChannelBadge key={contract} state={info.state} detail={info.detail} />
              ))
            )}
            <label className="ml-auto flex items-center gap-2">
              <span className="text-[var(--color-muted-foreground)]">节点地址</span>
              <input
                value={bench.address}
                onChange={(event) => bench.updateAddress(event.target.value)}
                spellCheck={false}
                className="w-56 rounded-md border border-[var(--color-input)] bg-transparent px-2 py-1 font-mono text-xs outline-none focus:border-[var(--color-primary)]"
              />
            </label>
          </div>
        ) : (
          <p className="rounded-md border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-muted-foreground)]">
            目录中还没有已注册节点。启动一个节点并让它连上这个中心后，这里会出现它的身份与模块清单。
          </p>
        )}
      </header>

      {bench.loadError ? (
        <p role="alert" className="text-xs text-[var(--color-destructive)]">{bench.loadError}</p>
      ) : null}

      <main>
        {bench.view === "conversation" ? <ConversationView bench={bench} /> : null}
        {bench.view === "servers" ? <ServersView bench={bench} /> : null}
        {bench.view === "models" ? <ModelsView bench={bench} /> : null}
        {bench.view === "graph" ? <GraphView bench={bench} /> : null}
      </main>
    </div>
  );
}
