// The workbench: one screen, with the connection's own state always visible.
//
// The previous shape was a stack of tabs, which hid the two things a visitor
// actually needs to know — whether they are connected, and who they are right
// now — behind whatever tab happened to be open. So the header is a status bar
// with the live link, the scenes are panels, and nothing owns the whole page.

import { useState } from "react";
import { Activity, Copy, FolderOpen, Gauge, Hand, Info, Users } from "lucide-react";

import { Badge, Card, CardContent, CardHeader, CardTitle, Separator } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ClientList } from "@/components/client-list";
import { FileScene } from "@/components/file-scene";
import { Intro } from "@/components/intro";
import { MessageLog } from "@/components/message-log";
import { PerfPanel } from "@/components/perf-panel";
import { ProfileCard } from "@/components/profile-card";
import { StatusPanel } from "@/components/status-panel";
import { ToastStack } from "@/components/toast-stack";
import { useHelloPage } from "@/hooks/use-hello-page";

const PANELS = [
  { id: "hello", label: "消息", icon: Hand },
  { id: "file", label: "文件", icon: FolderOpen },
  { id: "status", label: "状态", icon: Activity },
  { id: "about", label: "介绍", icon: Info },
  { id: "perf", label: "性能", icon: Gauge },
] as const;

type PanelId = (typeof PANELS)[number]["id"];

const STATUS_VARIANT = {
  connecting: "secondary",
  ready: "success",
  reconnecting: "secondary",
  error: "destructive",
} as const;

export function App() {
  const page = useHelloPage();
  // Every login lands on the message panel: it is the one that proves the
  // connection is real without asking the visitor to pick a file.
  const [panel, setPanel] = useState<PanelId>("hello");
  const self = page.clients.find((row) => row.linkId === page.selfLinkId);
  const selfName = self?.profile?.displayName || page.draft.displayName || "你";
  const ownGroupKey = self?.groupKey ?? "";
  const peers = page.clients.length - 1;

  return (
    <div className="mx-auto grid w-full max-w-5xl gap-4 px-4 py-6">
      <header className="grid gap-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="text-xs font-bold uppercase tracking-[0.14em] text-[var(--color-primary)]">
              conex
            </p>
            <h1 className="text-2xl font-semibold tracking-tight">双向能力路由内核</h1>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-xs text-[var(--color-muted-foreground)]" role="status">
              {page.detail}
            </span>
            <Badge variant={STATUS_VARIANT[page.connection]}>
              {page.connection === "ready" ? "已连接" : page.connection === "error" ? "异常" : "连接中"}
            </Badge>
          </div>
        </div>

        {/* The status bar, not a tab: who this tab is, which link it holds, and
            how many peers can hear it. This is what makes two tabs in one
            browser read as two clients rather than one flickering row. */}
        <Card>
          <CardContent className="grid gap-3 p-4 sm:grid-cols-2 lg:grid-cols-4">
            <Stat
              icon={Users}
              label="本标签页"
              value={selfName}
              hint={page.selfLinkId ? `链接 ${page.selfLinkId.slice(0, 8)}` : "尚未就绪"}
            />
            <Stat
              icon={Activity}
              label="本组分组"
              value={ownGroupKey ? ownGroupKey.slice(0, 8) : "—"}
              hint={self?.profile?.group || "未分组"}
            />
            <Stat
              icon={Hand}
              label="在线同组"
              value={String(Math.max(peers, 0))}
              hint="个可问候的客户端"
            />
            <Stat
              icon={Gauge}
              label="全站在线"
              value={String(page.status.clientsOnline)}
              hint={`${page.status.groupsOnline} 个分组`}
            />
          </CardContent>
        </Card>
      </header>

      <nav className="flex flex-wrap gap-1 rounded-md border border-[var(--color-border)] bg-[var(--color-muted)] p-1">
        {PANELS.map((item) => {
          const Icon = item.icon;
          return (
            <Button
              key={item.id}
              variant={panel === item.id ? "default" : "ghost"}
              size="sm"
              onClick={() => setPanel(item.id)}
              aria-current={panel === item.id ? "page" : undefined}
            >
              <Icon className="size-3.5" aria-hidden />
              {item.label}
            </Button>
          );
        })}
      </nav>

      <main className="grid gap-4">
        {panel === "about" ? <Intro /> : null}

        {panel === "hello" ? (
          <div className="grid gap-4">
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_20rem] lg:items-start">
              <ClientList clients={page.clients} selfLinkId={page.selfLinkId} onGreet={page.greet} />
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
          </div>
        ) : null}

        {panel === "file" ? (
          <div className="grid gap-4">
            <ProfileCard
              draft={page.draft}
              notice={page.savedNotice}
              onChange={page.setDraft}
              onSave={(next) => {
                // The group decides who can see these files, so it is stated
                // here rather than only in the message panel's card.
                void page.save(next);
              }}
            />
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
          </div>
        ) : null}

        {panel === "status" ? <StatusPanel status={page.status} ownGroupKey={ownGroupKey} /> : null}

        {panel === "perf" ? <PerfPanel /> : null}
      </main>

      <Separator />

      <Card>
        <CardHeader className="pb-3">
          <CardTitle>这个页面在做什么</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap items-start justify-between gap-4 p-5 pt-0 text-xs text-[var(--color-muted-foreground)]">
          <p className="min-w-[16rem] flex-1">
            这个页面运行的是真实的 conex-host 进程：所有客户端、往返延迟与文件都来自实际的连接，
            不是模拟数据。分组由你自己填写，经 Host 哈希成隔离键，只在同组内可见。
            每个标签页是一个独立客户端，各自持有自己的链接。
          </p>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void navigator.clipboard?.writeText(location.href)}
          >
            <Copy className="size-3.5" aria-hidden />
            复制本页链接
          </Button>
        </CardContent>
      </Card>

      {/* Corner notifications, outside the layout: a hello must never move the
          thing the visitor is about to click. */}
      <ToastStack
        toasts={page.toasts}
        onAnswer={page.answerToast}
        onDismiss={page.dismissToast}
      />
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: typeof Users;
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <div className="grid gap-0.5">
      <span className="flex items-center gap-1.5 text-xs text-[var(--color-muted-foreground)]">
        <Icon className="size-3.5" aria-hidden />
        {label}
      </span>
      <span className="truncate text-base font-semibold" title={value}>
        {value}
      </span>
      <span className="truncate font-mono text-xs text-[var(--color-muted-foreground)]">{hint}</span>
    </div>
  );
}
