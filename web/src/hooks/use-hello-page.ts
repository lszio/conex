// Connection state and the name/group cache.
//
// The cache exists because a visitor's name and group are *theirs*, not the
// link's: the server mints a new link on every reconnect, so without a local
// cache a refresh would drop you back to "client-xxxxxx" and lose your name.
// It is a convenience, never a source of truth — the host still owns the
// displayed name, including the uniqueness suffix it assigns.

import { useCallback, useEffect, useRef, useState } from "react";
import { ConexWsClient, type ClientSummary } from "@conex/sdk";

const CACHE_KEY = "conex.profile.v1";
const POLL_MS = 3000;
const LOG_LIMIT = 40;

export interface LogEntry {
  id: number;
  at: number;
  text: string;
  tone: "out" | "in" | "system";
}

/** What the visitor typed, kept locally so a reload can restore it. */
export interface CachedProfile {
  displayName: string;
  group: string;
  visible: boolean;
}

export function readCachedProfile(): CachedProfile | undefined {
  try {
    const raw = localStorage.getItem(CACHE_KEY);
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as Partial<CachedProfile>;
    if (typeof parsed.displayName !== "string" || typeof parsed.group !== "string") return undefined;
    return {
      displayName: parsed.displayName.slice(0, 40),
      group: parsed.group.slice(0, 24),
      visible: parsed.visible !== false,
    };
  } catch {
    // Private mode, disabled storage, or corrupt JSON: the page still works,
    // it just cannot restore the name.
    return undefined;
  }
}

export function writeCachedProfile(profile: CachedProfile): void {
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(profile));
  } catch {
    // A failed write is not worth interrupting the visitor over.
  }
}

export type ConnectionState = "connecting" | "ready" | "reconnecting" | "error";

export interface HelloPageState {
  connection: ConnectionState;
  detail: string;
  clients: ClientSummary[];
  selfLinkId: string;
  messages: LogEntry[];
  draft: CachedProfile;
  savedNotice: string;
  setDraft: (next: CachedProfile) => void;
  /** Takes the profile to save: setState is async, so reading `draft` here
   *  after onChange would persist the previous values. */
  save: (next: CachedProfile) => Promise<void>;
  greet: (target: ClientSummary) => Promise<void>;
}

export function useHelloPage(): HelloPageState {
  const cached = useRef<CachedProfile>(readCachedProfile() ?? { displayName: "", group: "", visible: true });
  const [connection, setConnection] = useState<ConnectionState>("connecting");
  const [detail, setDetail] = useState("正在取得会话…");
  const [clients, setClients] = useState<ClientSummary[]>([]);
  const [selfLinkId, setSelfLinkId] = useState("");
  const [messages, setMessages] = useState<LogEntry[]>([]);
  const [draft, setDraft] = useState<CachedProfile>(cached.current);
  const [savedNotice, setSavedNotice] = useState("");

  const clientRef = useRef<ConexWsClient | undefined>(undefined);
  const logId = useRef(0);
  const selfLinkRef = useRef("");

  const log = useCallback((text: string, tone: LogEntry["tone"] = "system") => {
    logId.current += 1;
    const entry: LogEntry = { id: logId.current, at: Date.now(), text, tone };
    setMessages((previous) => [entry, ...previous].slice(0, LOG_LIMIT));
  }, []);

  const label = useCallback(
    (row: ClientSummary) => row.profile?.displayName || row.linkId?.slice(0, 6) || "?",
    [],
  );

  const refresh = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const result = await client.listClients();
      setClients(result.clients ?? []);
    } catch (error) {
      setDetail(`读取列表失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let poll = 0;

    const applyProfile = async (client: ConexWsClient, profile: CachedProfile) => {
      const result = await client.setProfile({
        displayName: profile.displayName,
        group: profile.group,
        visible: profile.visible,
      });
      const self = result.self;
      if (!self?.linkId) return;
      selfLinkRef.current = self.linkId;
      setSelfLinkId(self.linkId);
      // The host may suffix a duplicate name; show what it actually assigned
      // rather than what was typed, so the field never lies about the name
      // other visitors will see.
      const assigned = self.profile?.displayName;
      if (assigned && assigned !== profile.displayName && profile.displayName) {
        writeCachedProfile({ ...profile, displayName: assigned });
        setDraft((previous) => ({ ...previous, displayName: assigned }));
        log(`名称「${profile.displayName}」已被占用，改为「${assigned}」`);
      }
    };

    const connect = async () => {
      const session = await fetch("/web/session", { credentials: "same-origin" });
      if (!session.ok) throw new Error("会话不可用");
      const body = (await session.json()) as { csrf?: string };
      if (!body.csrf) throw new Error("会话缺少 CSRF token");

      const client = new ConexWsClient({ origin: location.origin, csrfToken: body.csrf, reconnect: true });
      clientRef.current = client;
      client.onEvent((event) => {
        if (event.phase === "ready") {
          setConnection("ready");
          setDetail("已连接");
        } else if (event.phase === "reconnecting") {
          setConnection("reconnecting");
          setDetail("连接中断，正在重连…");
        } else if (event.phase === "failed" || event.phase === "error") {
          setConnection("error");
          setDetail("连接异常");
        }
      });
      client.onHello((hello) => log(`收到 ${hello.fromName || "某个客户端"} 的 hello`, "in"));
      await client.connect();
      setConnection("ready");
      setDetail("已连接");
      await applyProfile(client, cached.current);
      await refresh();
      poll = window.setInterval(() => {
        if (!document.hidden) void refresh();
      }, POLL_MS);
    };

    void (async () => {
      try {
        await connect();
      } catch (error) {
        if (cancelled) return;
        setConnection("error");
        setDetail(error instanceof Error ? error.message : String(error));
      }
    })();

    return () => {
      cancelled = true;
      window.clearInterval(poll);
      clientRef.current?.close();
    };
  }, [log, refresh]);

  const save = useCallback(async (profile: CachedProfile) => {
    const client = clientRef.current;
    if (!client) return;
    setDraft(profile);
    writeCachedProfile(profile);
    try {
      const result = await client.setProfile({
        displayName: profile.displayName,
        group: profile.group,
        visible: profile.visible,
      });
      const self = result.self;
      if (self?.linkId) {
        selfLinkRef.current = self.linkId;
        setSelfLinkId(self.linkId);
      }
      const assigned = self?.profile?.displayName;
      if (assigned && profile.displayName && assigned !== profile.displayName) {
        const renamed = { ...profile, displayName: assigned };
        writeCachedProfile(renamed);
        setDraft(renamed);
        setSavedNotice(`名称已被占用，当前显示为「${assigned}」`);
      } else {
        setSavedNotice(profile.visible ? "已保存，其他人可以看见你" : "已保存，你已对其他人隐藏");
      }
      log(`已保存资料：${assigned || "未命名"}${profile.visible ? "" : "（已隐藏）"}`);
      await refresh();
    } catch (error) {
      setSavedNotice(`保存失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }, [log, refresh]);

  const greet = useCallback(
    async (target: ClientSummary) => {
      const client = clientRef.current;
      const linkId = target.linkId;
      if (!client || !linkId) return;
      try {
        // The host attaches the sender's display name; the text stays plain so
        // the recipient is not told twice who is greeting them.
        const result = await client.sendHello(linkId, "hello");
        const measured = Number(result.roundTripMs ?? "0");
        // A sub-millisecond result is real but reads as zero; say so rather
        // than rounding a valid measurement into a fake one.
        const shown = measured < 1 ? "<1 ms" : `${measured} ms`;
        log(`→ ${label(target)}：${shown}，回复「${result.reply}」`, "out");
      } catch (error) {
        log(`→ ${label(target)} 失败：${error instanceof Error ? error.message : String(error)}`, "out");
      }
    },
    [label, log],
  );

  return {
    connection,
    detail,
    clients,
    selfLinkId,
    messages,
    draft,
    savedNotice,
    setDraft,
    save,
    greet,
  };
}
