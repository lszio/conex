// 私有登录：一次性引导码换 cookie 会话。引导码不落 localStorage，不出现在
// URL；错误原样呈现（错误码 401 = 无效或已消费），不猜测原因。

import { useState } from "react";

import type { PrivateHubClient } from "@conex/sdk";
import { saveCsrf } from "@/domain/session";

export function LoginView({
  hub,
  onReady,
}: {
  hub: PrivateHubClient;
  onReady: (hub: PrivateHubClient) => void;
}) {
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!code.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await hub.login(code.trim());
      saveCsrf(hub);
      onReady(hub);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid min-h-screen place-items-center px-4">
      <form
        onSubmit={submit}
        className="grid w-full max-w-sm gap-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-6"
      >
        <header className="grid gap-1">
          <h1 className="text-lg font-semibold tracking-tight">
            <span className="text-[var(--color-primary)]">conex</span> 私有工作台
          </h1>
          <p className="text-xs text-[var(--color-muted-foreground)]">
            输入管理员在服务端生成的引导码（5 分钟内有效、单次消费）。
          </p>
        </header>
        <label className="grid gap-1.5">
          <span className="text-sm font-medium">引导码</span>
          <input
            type="password"
            value={code}
            onChange={(event) => setCode(event.target.value)}
            autoFocus
            autoComplete="off"
            spellCheck={false}
            className="rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 font-mono text-sm outline-none focus:border-[var(--color-primary)]"
            placeholder="43 位 base64url"
          />
        </label>
        {error ? (
          <p role="alert" className="text-xs text-[var(--color-destructive)]">
            {error}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={busy || !code.trim()}
          className="rounded-md bg-[var(--color-primary)] px-3 py-2 text-sm font-medium text-[var(--color-primary-foreground)] disabled:opacity-50"
        >
          {busy ? "登录中…" : "登录"}
        </button>
      </form>
    </div>
  );
}
