// Private product entry (MVP-P7).
//
// The host decides the shape, not the bundle: a `[private]` deployment never
// serves the guest landing, and a guest host has no private session route.
// The page probes `/private/directory` once — 404 means the legacy guest
// landing, 401 means the private login, 200 means the workbench. No fake
// states: every branch below is the host's own answer.

import { useEffect, useState } from "react";

import { PrivateHubClient } from "@conex/sdk";
import { HelloApp } from "@/legacy-hello";
import { LoginView } from "@/views/login-view";
import { WorkbenchView } from "@/views/workbench-view";

type Entry = "checking" | "legacy" | "login" | "ready";

export function App() {
  const [entry, setEntry] = useState<Entry>("checking");
  const [hub, setHub] = useState<PrivateHubClient | null>(null);

  useEffect(() => {
    const client = new PrivateHubClient("");
    let cancelled = false;
    client
      .directory()
      .then(() => {
        if (!cancelled) {
          setHub(client);
          setEntry("ready");
        }
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        const status = (error as { status?: number }).status;
        // 401：私有入口在等引导码；404/网络错误：这不是私有部署。
        if (status === 401 || status === 403) {
          setHub(client);
          setEntry("login");
        } else {
          setEntry("legacy");
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (entry === "checking") {
    return (
      <div className="grid min-h-screen place-items-center text-sm text-[var(--color-muted-foreground)]">
        正在检查入口…
      </div>
    );
  }
  if (entry === "legacy" || !hub) return <HelloApp />;
  if (entry === "login") {
    return (
      <LoginView
        hub={hub}
        onReady={(client) => {
          setHub(client);
          setEntry("ready");
        }}
      />
    );
  }
  return <WorkbenchView hub={hub} />;
}
