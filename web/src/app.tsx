// The landing page: one screen, no tabs.
//
// The workbench put five panels behind a switch, which hid the only two facts
// a visitor needs — who they are, and who else is here — behind whatever
// panel happened to be open. So this is one screen: your identity, the people
// you can reach, and what you have already exchanged. Files and the status
// read are folded into the same scroll rather than hidden behind a nav.

import { Activity, Copy, FolderOpen, Gauge, Hand, Users } from "lucide-react";

import { Badge, Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ClientList } from "@/components/client-list";
import { FileScene } from "@/components/file-scene";
import { MessageLog } from "@/components/message-log";
import { ProfileCard } from "@/components/profile-card";
import { StatusPanel } from "@/components/status-panel";
import { ToastStack } from "@/components/toast-stack";
import { useHelloPage } from "@/hooks/use-hello-page";

const STATUS_VARIANT = {
  connecting: "secondary",
  ready: "success",
  reconnecting: "secondary",
  error: "destructive",
} as const;

export function App() {
  const page = useHelloPage();
  const self = page.clients.find((row) => row.linkId === page.selfLinkId);
  const selfName = self?.profile?.displayName || page.draft.displayName || "你";
  const ownGroupKey = self?.groupKey ?? "";
  const peers = page.clients.length - 1;
  // A short list with peers online is the confusing case: the host counts
  // them, `client/list` withholds them. Say which rule applied instead of
  // leaving a bare 0 to be read as "nobody else is here".
  const peerHint =
    peers > 0
      ? "个可问候的客户端"
      : page.status.clientsOnline > 1
        ? `全站 ${page.status.clientsOnline} 个在线——只有同组且可见的才出现在这里`
        : "当前只有你一个在线";

  return (
    <div className="mx-auto grid w-full max-w-3xl gap-4 px-4 py-6">
      <header className="grid gap-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h1 className="text-lg font-semibold tracking-tight">
            <span className="text-[var(--color-primary)]">conex</span>
            <span className="ml-2 font-normal text-[var(--color-muted-foreground)]">
              双向能力路由内核
            </span>
          </h1>
<div className="flex items-center gap-2">
              <Badge variant={STATUS_VARIANT[page.connection]}>
                {page.connection === "ready" ? "已连接" : page.connection === "error" ? "异常" : "连接中"}
              </Badge>
              {page.connection !== "ready" ? (
                <span className="text-xs text-[var(--color-muted-foreground)]" role="status">
                  {page.detail}
                </span>
              ) : null}
            </div>
        </div>

        {/* Who this tab is, in one line. Two tabs of one browser must never read
            as one client, so the link is shown rather than implied. */}
        <Card>
          <CardContent className="flex flex-wrap items-center justify-between gap-3 p-3">
            <div className="flex min-w-0 items-center gap-2">
              <Users className="size-4 shrink-0 text-[var(--color-muted-foreground)]" aria-hidden />
              <span className="truncate font-medium">{selfName}</span>
              <Badge variant="outline">你</Badge>
              {page.selfLinkId ? (
                <span className="truncate font-mono text-xs text-[var(--color-muted-foreground)]">
                  链接 {page.selfLinkId.slice(0, 8)}
                </span>
              ) : null}
            </div>
            <div className="flex items-center gap-4 text-xs text-[var(--color-muted-foreground)]">
              <span className="flex items-center gap-1">
                <Hand className="size-3.5" aria-hidden />
                在线 {Math.max(peers, 0)}
              </span>
              <span className="flex items-center gap-1">
                <Gauge className="size-3.5" aria-hidden />
                全站 {page.status.clientsOnline}
              </span>
            </div>
          </CardContent>
        </Card>

        <p className="text-xs text-[var(--color-muted-foreground)]">
          再开一个标签页，就能和这里的人互相打招呼。每个标签页是一个独立客户端。
        </p>
      </header>

      <main className="grid gap-4">
        <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_18rem] sm:items-start">
          <ClientList
            clients={page.clients}
            selfLinkId={page.selfLinkId}
            clientsOnline={page.status.clientsOnline}
            selfVisible={page.draft.visible}
            onGreet={page.greet}
            peerHint={peerHint}
          />
          <ProfileCard
            draft={page.draft}
            notice={page.savedNotice}
            onChange={page.setDraft}
            onSave={(next) => {
              void page.save(next);
            }}
          />
        </div>

        <MessageLog messages={page.messages} />

        {/* Files come after the people and the exchange: on a landing page the
            point is who is here and whether the greeting landed, and an empty
            share panel sitting above that reads as the page having nothing to
            show. */}
        <FileScene
          files={page.files}
          selfLinkId={page.selfLinkId}
          groupLabel={self?.profile?.group || page.draft.group}
          groupKey={ownGroupKey}
          notice={page.shareNotice}
          uploading={page.uploading}
          onShare={(picked) => void page.shareFiles(picked)}
          onWithdraw={(file) => void page.withdrawFile(file)}
          fileUrl={page.fileUrl}
        />

        <details className="rounded-md border border-[var(--color-border)]">
          <summary className="cursor-pointer px-4 py-2 text-xs font-medium text-[var(--color-muted-foreground)]">
            运行状态与延迟
          </summary>
          <div className="border-t border-[var(--color-border)] p-3">
            <StatusPanel status={page.status} ownGroupKey={ownGroupKey} />
          </div>
        </details>
      </main>

      <footer className="flex flex-wrap items-center justify-between gap-3 pt-1 text-xs text-[var(--color-muted-foreground)]">
        <p className="min-w-[14rem] flex-1">
          这个页面跑的是真实的 conex-host 进程：在线的人、往返延迟和文件都来自实际连接。
          不填组名就是全员一组；填了组名就只和同组的人互相可见。
        </p>
        <div className="flex items-center gap-3">
          <span className="flex items-center gap-1">
            <Activity className="size-3.5" aria-hidden />
            {page.status.groupsOnline} 个分组
          </span>
          <span className="flex items-center gap-1">
            <FolderOpen className="size-3.5" aria-hidden />
            {page.files.length} 个文件
          </span>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void navigator.clipboard?.writeText(location.href)}
          >
            <Copy className="size-3.5" aria-hidden />
            复制链接
          </Button>
        </div>
      </footer>

      {/* Corner notifications, outside the layout: a hello must never move the
          thing the visitor is about to click. */}
      <ToastStack toasts={page.toasts} onAnswer={page.answerToast} onDismiss={page.dismissToast} />
    </div>
  );
}