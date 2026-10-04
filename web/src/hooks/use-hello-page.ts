// The page's whole state: one connection, one identity, three read paths.
//
// The cache exists because a visitor's name and group are *theirs*, not the
// link's: the server mints a new link on every reconnect, so without a local
// cache a refresh would drop you back to "client-xxxxxx" and lose your name.
// It is a convenience, never a source of truth — the host still owns the
// displayed name, including the uniqueness suffix it assigns, and the group
// hash it derives.

import { useCallback, useEffect, useRef, useState } from "react";
import { ConexWsClient, type ClientSummary } from "@conex/sdk";

import { listFiles, uploadFile, downloadFileUrl, removeFile, setCsrfToken, type SharedFile } from "@/lib/files";

const CACHE_KEY = "conex.profile.v1";
const POLL_MS = 3000;
const LOG_LIMIT = 40;
const BUBBLE_LIMIT = 24;

export interface LogEntry {
  id: number;
  at: number;
  text: string;
  tone: "out" | "in" | "system";
}

/** A hello shown as a bubble on the target site, newest last. */
export interface Bubble {
  id: number;
  at: number;
  /** Display name of the peer, resolved by the host. */
  from: string;
  text: string;
  direction: "in" | "out";
  /** Round-trip milliseconds; only present for an outgoing hello. */
  roundTripMs?: number;
}

/** One row of `client/status`. */
export interface GroupRow {
  groupKey: string;
  label: string;
  clientsOnline: number;
  lastRoundTripMs: number;
  avgRoundTripMs: number;
  roundTrips: number;
  filesShared: number;
  sharedBytes: number;
}

export interface StatusState {
  clientsOnline: number;
  groupsOnline: number;
  groups: GroupRow[];
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
    if (typeof parsed !== "object" || parsed === null) return undefined;
    return {
      displayName: typeof parsed.displayName === "string" ? parsed.displayName : "",
      group: typeof parsed.group === "string" ? parsed.group : "",
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

/** Which scene the page is showing. `hello` is the default every login lands on. */
export type Scene = "hello" | "file";

export interface HelloPageState {
  connection: ConnectionState;
  detail: string;
  clients: ClientSummary[];
  selfLinkId: string;
  messages: LogEntry[];
  bubbles: Bubble[];
  draft: CachedProfile;
  savedNotice: string;
  status: StatusState;
  files: SharedFile[];
  shareNotice: string;
  uploading: boolean;
  setDraft: (next: CachedProfile) => void;
  /** Takes the profile to save: setState is async, so reading `draft` here
   *  after onChange would persist the previous values. */
  save: (next: CachedProfile) => Promise<void>;
  greet: (target: ClientSummary) => void;
  shareFiles: (files: FileList) => Promise<void>;
  withdrawFile: (file: SharedFile) => Promise<void>;
  fileUrl: (file: SharedFile) => string;
  refreshFiles: () => Promise<void>;
  /** Rerun after a group change, so the new group's peers load. */
  refreshPeers: () => Promise<void>;
}

const EMPTY_STATUS: StatusState = { clientsOnline: 0, groupsOnline: 0, groups: [] };

export function useHelloPage(): HelloPageState {
  const cached = useRef<CachedProfile>(readCachedProfile() ?? { displayName: "", group: "", visible: true });
  const [connection, setConnection] = useState<ConnectionState>("connecting");
  const [detail, setDetail] = useState("正在取得会话…");
  const [clients, setClients] = useState<ClientSummary[]>([]);
  const [selfLinkId, setSelfLinkId] = useState("");
  const [messages, setMessages] = useState<LogEntry[]>([]);
  const [bubbles, setBubbles] = useState<Bubble[]>([]);
  const [draft, setDraft] = useState<CachedProfile>(cached.current);
  const [savedNotice, setSavedNotice] = useState("");
  const [status, setStatus] = useState<StatusState>(EMPTY_STATUS);
  const [files, setFiles] = useState<SharedFile[]>([]);
  const [shareNotice, setShareNotice] = useState("");
  const [uploading, setUploading] = useState(false);

  const clientRef = useRef<ConexWsClient | undefined>(undefined);
  const logId = useRef(0);
  const bubbleId = useRef(0);
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

  const pushBubble = useCallback((bubble: Omit<Bubble, "id" | "at">) => {
    bubbleId.current += 1;
    const entry: Bubble = { ...bubble, id: bubbleId.current, at: Date.now() };
    // Bubbles read as a conversation, so the newest goes last; the cap keeps
    // a long session from growing the DOM without bound.
    setBubbles((previous) => [...previous, entry].slice(-BUBBLE_LIMIT));
  }, []);

  const refresh = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const [result, counters] = await Promise.all([client.listClients(), client.listClientStatus()]);
      setClients(result.clients ?? []);
      setStatus({
        clientsOnline: Number(counters.clientsOnline ?? "0"),
        groupsOnline: Number(counters.groupsOnline ?? "0"),
        groups: (counters.groups ?? []).map((group) => ({
          groupKey: group.groupKey ?? "",
          label: group.label ?? "",
          clientsOnline: Number(group.clientsOnline ?? "0"),
          lastRoundTripMs: Number(group.lastRoundTripMs ?? "0"),
          avgRoundTripMs: Number(group.avgRoundTripMs ?? "0"),
          roundTrips: Number(group.roundTrips ?? "0"),
          filesShared: Number(group.filesShared ?? "0"),
          sharedBytes: Number(group.sharedBytes ?? "0"),
        })),
      });
    } catch (error) {
      setDetail(`读取列表失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }, []);

  const refreshFiles = useCallback(async () => {
    try {
      const listing = await listFiles();
      setFiles(listing.files);
    } catch (error) {
      setShareNotice(`读取文件失败：${error instanceof Error ? error.message : String(error)}`);
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
      // The share routes are plain HTTP, not the SDK's WSS path, so they need
      // the same nonce the ticket request uses.
      setCsrfToken(body.csrf);

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
      client.onHello((hello) => {
        const from = hello.fromName || "某个客户端";
        // The bubble is the point of the hello scene: the greeting appears
        // on the target's own page, with the sender's name resolved by the
        // host rather than by the sender's page.
        pushBubble({ from, text: hello.text || "hello", direction: "in" });
        log(`收到 ${from} 的 hello`, "in");
      });
      await client.connect();
      setConnection("ready");
      setDetail("已连接");
      await applyProfile(client, cached.current);
      await refresh();
      await refreshFiles();
      poll = window.setInterval(() => {
        // `client/list` is skipped for a hidden tab to spare it work, but the
        // shared files must not be: a visitor watching one tab in the
        // background has to see what peers shared when they come back, and
        // that is the whole point of the file scene.
        if (!document.hidden) void refresh();
        void refreshFiles();
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
  }, [log, pushBubble, refresh, refreshFiles]);

  const save = useCallback(
    async (profile: CachedProfile) => {
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
        // A group change moves this client to a different peer set and a
        // different file scope, so both lists have to be read again.
        await Promise.all([refresh(), refreshFiles()]);
      } catch (error) {
        setSavedNotice(`保存失败：${error instanceof Error ? error.message : String(error)}`);
      }
    },
    [log, refresh, refreshFiles],
  );

  const greet = useCallback(
    (target: ClientSummary) => {
      const client = clientRef.current;
      const linkId = target.linkId;
      const name = label(target);
      if (!client || !linkId) return;
      // The bubble goes in before the round trip finishes so the greeting is
      // visible immediately; the measured latency fills in when it answers.
      pushBubble({ from: name, text: "hello", direction: "out" });
      void (async () => {
        try {
          // The host attaches the sender's display name; the text stays plain so
          // the recipient is not told twice who is greeting them.
          const result = await client.sendHello(linkId, "hello");
          const measured = Number(result.roundTripMs ?? "0");
          // A sub-millisecond result is real but reads as zero; say so rather
          // than rounding a valid measurement into a fake one.
          const shown = measured < 1 ? "<1 ms" : `${measured} ms`;
          setBubbles((previous) => {
            const last = previous[previous.length - 1];
            if (!last || last.direction !== "out" || last.roundTripMs !== undefined) return previous;
            return [...previous.slice(0, -1), { ...last, roundTripMs: measured }];
          });
          log(`→ ${name}：${shown}，回复「${result.reply}」`, "out");
        } catch (error) {
          log(`→ ${name} 失败：${error instanceof Error ? error.message : String(error)}`, "out");
        }
        await refresh();
      })();
    },
    [label, log, pushBubble, refresh],
  );

  const shareFiles = useCallback(
    async (picked: FileList) => {
      if (picked.length === 0) return;
      setUploading(true);
      setShareNotice("");
      let done = 0;
      for (const file of Array.from(picked)) {
        try {
          await uploadFile(file);
          done += 1;
        } catch (error) {
          setShareNotice(
            `${file.name} 上传失败：${error instanceof Error ? error.message : String(error)}`,
          );
        }
      }
      setUploading(false);
      if (done > 0) {
        setShareNotice(`已共享 ${done} 个文件给同组客户端`);
        await Promise.all([refreshFiles(), refresh()]);
      }
    },
    [refresh, refreshFiles],
  );

  const withdrawFile = useCallback(
    async (file: SharedFile) => {
      try {
        await removeFile(file.id);
        setShareNotice(`已撤回「${file.name}」`);
        await Promise.all([refreshFiles(), refresh()]);
      } catch (error) {
        setShareNotice(`撤回失败：${error instanceof Error ? error.message : String(error)}`);
      }
    },
    [refresh, refreshFiles],
  );

  return {
    connection,
    detail,
    clients,
    selfLinkId,
    messages,
    bubbles,
    draft,
    savedNotice,
    status,
    files,
    shareNotice,
    uploading,
    setDraft,
    save,
    greet,
    shareFiles,
    withdrawFile,
    fileUrl: downloadFileUrl,
    refreshFiles,
    refreshPeers: refreshFiles,
  };
}
