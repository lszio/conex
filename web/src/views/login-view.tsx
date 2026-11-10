// 私有登录（路由 A）：账号密码为主入口，一次性引导码保留为首次建号与
// 恢复凭证。引导码不落 localStorage、不出现在 URL；错误原样呈现，不猜测
// 原因。失败提示对"用户名不存在"与"密码错误"在服务端就不区分，这里同样
// 只透传。

import { useEffect, useState } from "react";

import type { PrivateHubClient } from "@conex/sdk";
import { saveCsrf } from "@/domain/session";

type Stage =
  | "checking"
  | "password"
  | "code"
  | "provision"
  | "reset";

export function LoginView({
  hub,
  onReady,
}: {
  hub: PrivateHubClient;
  onReady: (hub: PrivateHubClient) => void;
}) {
  const [stage, setStage] = useState<Stage>("checking");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    hub
      .accountStatus()
      .then((status) => {
        if (!cancelled) setStage(status.accountExists ? "password" : "code");
      })
      .catch(() => {
        // 状态不可知时保守退回引导码表单：它永远可用。
        if (!cancelled) setStage("code");
      });
    return () => {
      cancelled = true;
    };
  }, [hub]);

  const run = async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const submitPassword = (event: React.FormEvent) => {
    event.preventDefault();
    if (!username.trim() || !password || busy) return;
    void run(async () => {
      await hub.loginPassword(username.trim(), password);
      saveCsrf(hub);
      onReady(hub);
    });
  };

  const submitCode = (event: React.FormEvent) => {
    event.preventDefault();
    if (!code.trim() || busy) return;
    void run(async () => {
      await hub.login(code.trim());
      saveCsrf(hub);
      const status = await hub.accountStatus();
      setCode("");
      setPassword("");
      setStage(status.accountExists ? "reset" : "provision");
    });
  };

  const submitProvision = (event: React.FormEvent) => {
    event.preventDefault();
    if (!username.trim() || !password || busy) return;
    if (password !== confirm) {
      setError("两次输入的密码不一致");
      return;
    }
    void run(async () => {
      await hub.createAccount(username.trim(), password);
      setPassword("");
      setConfirm("");
      onReady(hub);
    });
  };

  const submitReset = (event: React.FormEvent) => {
    event.preventDefault();
    if (!username.trim() || !password || busy) return;
    if (password !== confirm) {
      setError("两次输入的密码不一致");
      return;
    }
    void run(async () => {
      await hub.setAccountPassword(username.trim(), password);
      setPassword("");
      setConfirm("");
      onReady(hub);
    });
  };

  if (stage === "checking") {
    return (
      <div className="grid min-h-screen place-items-center px-4">
        <p className="text-xs text-[var(--color-muted-foreground)]">正在检查私有入口…</p>
      </div>
    );
  }

  if (stage === "provision" || stage === "reset") {
    const provisioning = stage === "provision";
    return (
      <div className="grid min-h-screen place-items-center px-4">
        <form
          onSubmit={provisioning ? submitProvision : submitReset}
          className="grid w-full max-w-sm gap-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-6"
        >
          <header className="grid gap-1">
            <h1 className="text-lg font-semibold tracking-tight">
              <span className="text-[var(--color-primary)]">conex</span>{" "}
              {provisioning ? "设置管理员账号" : "重置管理员密码"}
            </h1>
            <p className="text-xs text-[var(--color-muted-foreground)]">
              {provisioning
                ? "首次部署：引导码验证通过。设置日常登录用的管理员账号与密码，此后不再需要引导码。"
                : "引导码验证通过：为本地账号设置新密码（忘记密码时的恢复路径）。"}
            </p>
          </header>
          <label className="grid gap-1.5">
            <span className="text-sm font-medium">用户名</span>
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              autoFocus
              autoComplete="username"
              spellCheck={false}
              className="rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)]"
              placeholder="字母、数字、_.-，最长 64"
            />
          </label>
          <label className="grid gap-1.5">
            <span className="text-sm font-medium">新密码</span>
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              autoComplete="new-password"
              className="rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)]"
              placeholder="至少 8 个字符"
            />
          </label>
          <label className="grid gap-1.5">
            <span className="text-sm font-medium">确认新密码</span>
            <input
              type="password"
              value={confirm}
              onChange={(event) => setConfirm(event.target.value)}
              autoComplete="new-password"
              className="rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)]"
            />
          </label>
          {error ? (
            <p role="alert" className="text-xs text-[var(--color-destructive)]">
              {error}
            </p>
          ) : null}
          <button
            type="submit"
            disabled={busy || !username.trim() || !password || !confirm}
            className="rounded-md bg-[var(--color-primary)] px-3 py-2 text-sm font-medium text-[var(--color-primary-foreground)] disabled:opacity-50"
          >
            {busy ? "提交中…" : provisioning ? "创建并进入" : "重置并进入"}
          </button>
        </form>
      </div>
    );
  }

  const passwordStage = stage === "password";
  return (
    <div className="grid min-h-screen place-items-center px-4">
      <form
        onSubmit={passwordStage ? submitPassword : submitCode}
        className="grid w-full max-w-sm gap-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-6"
      >
        <header className="grid gap-1">
          <h1 className="text-lg font-semibold tracking-tight">
            <span className="text-[var(--color-primary)]">conex</span> 私有工作台
          </h1>
          <p className="text-xs text-[var(--color-muted-foreground)]">
            {passwordStage
              ? "使用管理员账号登录。"
              : "输入管理员在服务端生成的引导码（5 分钟内有效、单次消费）。首次部署用它创建管理员账号。"}
          </p>
        </header>
        {passwordStage ? (
          <>
            <label className="grid gap-1.5">
              <span className="text-sm font-medium">用户名</span>
              <input
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                autoFocus
                autoComplete="username"
                spellCheck={false}
                className="rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)]"
              />
            </label>
            <label className="grid gap-1.5">
              <span className="text-sm font-medium">密码</span>
              <input
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete="current-password"
                className="rounded-md border border-[var(--color-input)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)]"
              />
            </label>
          </>
        ) : (
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
        )}
        {error ? (
          <p role="alert" className="text-xs text-[var(--color-destructive)]">
            {error}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={busy || (passwordStage ? !username.trim() || !password : !code.trim())}
          className="rounded-md bg-[var(--color-primary)] px-3 py-2 text-sm font-medium text-[var(--color-primary-foreground)] disabled:opacity-50"
        >
          {busy ? "登录中…" : "登录"}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setError(null);
            setCode("");
            setStage(passwordStage ? "code" : "password");
          }}
          className="text-xs text-[var(--color-muted-foreground)] underline-offset-2 hover:underline disabled:opacity-50"
        >
          {passwordStage ? "使用引导码登录" : "已有账号？使用账号密码登录"}
        </button>
      </form>
    </div>
  );
}
