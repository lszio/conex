// Page shell: what conex is, then the two scenes it demonstrates.
//
// Tabs exist because these are different questions. "介绍" is a claim about
// the project, "hello" and "文件" are two live scenes that need a real
// connection, "状态" is what the host currently sees, and the performance
// report is reference material rather than something to read between two
// greetings.

import { useState } from "react";

import { Badge, Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ClientList } from "@/components/client-list";
import { FileScene } from "@/components/file-scene";
import { Intro } from "@/components/intro";
import { MessageBubbles } from "@/components/message-bubbles";
import { MessageLog } from "@/components/message-log";
import { PerfPanel } from "@/components/perf-panel";
import { ProfileCard } from "@/components/profile-card";
import { StatusPanel } from "@/components/status-panel";
import { useHelloPage } from "@/hooks/use-hello-page";

const TABS = [
  { id: "about", label: "介绍" },
  { id: "hello", label: "hello 场景" },
  { id: "file", label: "文件场景" },
  { id: "status", label: "状态" },
  { id: "perf", label: "性能" },
] as const;

type TabId = (typeof TABS)[number]["id"];

const STATUS_VARIANT = {
  connecting: "secondary",
  ready: "success",
  reconnecting: "secondary",
  error: "destructive",
} as const;

export function App() {
  const page = useHelloPage();
  // Every login lands on the hello scene: it is the one that proves the
  // connection is real without asking the visitor to pick a file.
  const [tab, setTab] = useState<TabId>("hello");
  const self = page.clients.find((row) => row.linkId === page.selfLinkId);
  const selfName = self?.profile?.displayName || page.draft.displayName || "你";
  const ownGroupKey = self?.groupKey ?? "";

  return (
    <div className="mx-auto grid w-full max-w-3xl gap-4 px-4 py-8">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="text-xs font-bold uppercase tracking-[0.14em] text-[var(--color-primary)]">
            conex
          </p>
          <h1 className="text-3xl font-semibold tracking-tight">双向能力路由内核</h1>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-xs text-[var(--color-muted-foreground)]" role="status">
            {page.detail}
          </span>
          <Badge variant={STATUS_VARIANT[page.connection]}>
            {page.connection === "ready" ? "已连接" : page.connection === "error" ? "异常" : "连接中"}
          </Badge>
        </div>
      </header>

      <nav className="flex flex-wrap gap-1 rounded-md border border-[var(--color-border)] bg-[var(--color-muted)] p-1">
        {TABS.map((item) => (
          <Button
            key={item.id}
            variant={tab === item.id ? "default" : "ghost"}
            size="sm"
            onClick={() => setTab(item.id)}
            aria-current={tab === item.id ? "page" : undefined}
          >
            {item.label}
          </Button>
        ))}
      </nav>

      {tab === "about" ? <Intro /> : null}

      {tab === "hello" ? (
        <div className="grid gap-4">
          <MessageBubbles bubbles={page.bubbles} selfLabel={selfName} peerCount={page.clients.length - 1} />
          <ProfileCard
            draft={page.draft}
            notice={page.savedNotice}
            onChange={page.setDraft}
            onSave={(next) => {
              void page.save(next);
            }}
          />
          <ClientList clients={page.clients} selfLinkId={page.selfLinkId} onGreet={page.greet} />
          <MessageLog messages={page.messages} />
        </div>
      ) : null}

      {tab === "file" ? (
        <div className="grid gap-4">
          <ProfileCard
            draft={page.draft}
            notice={page.savedNotice}
            onChange={page.setDraft}
            onSave={(next) => {
              // The group decides who can see these files, so it is stated
              // here rather than only in the hello scene's card.
              void page.save(next);
            }}
          />
          <FileScene
            files={page.files}
            selfLinkId={page.selfLinkId}
            groupLabel={self?.profile?.group ?? page.draft.group}
            groupKey={ownGroupKey}
            notice={page.shareNotice}
            uploading={page.uploading}
            onShare={(picked) => void page.shareFiles(picked)}
            onWithdraw={(file) => void page.withdrawFile(file)}
            fileUrl={page.fileUrl}
          />
        </div>
      ) : null}

      {tab === "status" ? <StatusPanel status={page.status} ownGroupKey={ownGroupKey} /> : null}

      {tab === "perf" ? <PerfPanel /> : null}

      <Card>
        <CardContent className="p-5 text-xs text-[var(--color-muted-foreground)]">
          这个页面运行的是真实的 conex-host 进程：所有客户端、往返延迟与文件都来自实际的
          连接，不是模拟数据。分组由你自己填写，经 Host 哈希成隔离键，只在同组内可见。
        </CardContent>
      </Card>
    </div>
  );
}
