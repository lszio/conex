// Page shell: status, tabbed panels, and the connection lifecycle. Tabs exist
// because the performance report is reference material, not something to read
// between two hellos.

import { useState } from "react";

import { Badge, Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ClientList } from "@/components/client-list";
import { Intro } from "@/components/intro";
import { MessageLog } from "@/components/message-log";
import { PerfPanel } from "@/components/perf-panel";
import { ProfileCard } from "@/components/profile-card";
import { useHelloPage } from "@/hooks/use-hello-page";

const TABS = [
  { id: "clients", label: "客户端" },
  { id: "about", label: "介绍" },
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
  const [tab, setTab] = useState<TabId>("clients");

  return (
    <div className="mx-auto grid w-full max-w-3xl gap-4 px-4 py-8">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="text-xs font-bold uppercase tracking-[0.14em] text-[var(--color-primary)]">
            conex
          </p>
          <h1 className="text-3xl font-semibold tracking-tight">hello</h1>
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

      <nav className="flex gap-1 rounded-md border border-[var(--color-border)] bg-[var(--color-muted)] p-1">
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

      {tab === "clients" ? (
        <div className="grid gap-4">
          <ProfileCard
            draft={page.draft}
            notice={page.savedNotice}
            onChange={page.setDraft}
            onSave={(next) => {
              void page.save(next);
            }}
          />
          <ClientList clients={page.clients} selfLinkId={page.selfLinkId} onGreet={(row) => void page.greet(row)} />
          <MessageLog messages={page.messages} />
        </div>
      ) : null}

      {tab === "about" ? (
        <div className="grid gap-4">
          <Intro />
          <Card>
            <CardContent className="p-5 text-sm text-[var(--color-muted-foreground)]">
              <p className="grid gap-2">
                这里的每个客户端就是一条浏览器连接。你看到的名字由你自己填写，Host
                会保证同屏不重名；隐藏之后你既不出现在别人的列表里，也无法被别人问候。
              </p>
              <p className="mt-2 grid gap-2">
                这台 Host 由中心进程统一认证与路由。页面不提供写入、命令执行或任何改变
                远端状态的能力，只有只读的问与答。
              </p>
              <p className="mt-3">
                <a
                  className="font-medium text-[var(--color-primary)] underline underline-offset-4"
                  href="/llms.txt"
                >
                  llms.txt
                </a>{" "}
                是同一站点的机器可读说明。
              </p>
            </CardContent>
          </Card>
        </div>
      ) : null}

      {tab === "perf" ? <PerfPanel /> : null}
    </div>
  );
}
