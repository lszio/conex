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

import { listFiles, uploadFile, downloadFileUrl, removeFile, setCsrfToken, setLinkId, type SharedFile } from "@/lib/files";

const CACHE_KEY = "conex.profile.v1";
const POLL_MS = 3000;
const LOG_LIMIT = 40;
const TOAST_LIMIT = 6;

export interface LogEntry {
  id: number;
  at: number;
  text: string;
  tone: "out" | "in" | "system";
}

/**
 * A notification in the corner, not a line in a transcript.
 *
 * A hello is an event, not a conversation: it arrives, it is read, it is gone.
 * `replyId` is present only while the notification is a question waiting for
 * this tab to answer it — that is what lets a toast own a confirm button
 * without the page having to track which greeting it belongs to.
 */
export interface Toast {
  id: number;
  at: number;
  title: string;
  body: string;
  tone: "info" | "success" | "error";
  /** Present when this toast is a question the host is still waiting on. */
  replyId?: string;
  /** The fields the sender asked about, echoed in the confirm prompt. */
  fields?: string[];
  /** How long before the toast leaves on its own; questions never expire. */
  ttlMs: number;
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
  toasts: Toast[];
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
  greet: (target: ClientSummary, ask?: string) => void;
  /** Answer a question the host pushed, with a value per asked field. */
  answerToast: (toast: Toast, values: Record<string, string>) => void;
  dismissToast: (id: number) => void;
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
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [draft, setDraft] = useState<CachedProfile>(cached.current);
  const [savedNotice, setSavedNotice] = useState("");
  const [status, setStatus] = useState<StatusState>(EMPTY_STATUS);
  const [files, setFiles] = useState<SharedFile[]>([]);
  const [shareNotice, setShareNotice] = useState("");
  const [uploading, setUploading] = useState(false);

  const clientRef = useRef<ConexWsClient | undefined>(undefined);
  const logId = useRef(0);
  const toastId = useRef(0);
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

  const dismissToast = useCallback((id: number) => {
    setToasts((previous) => previous.filter((toast) => toast.id !== id));
  }, []);

  const pushToast = useCallback((toast: Omit<Toast, "id" | "at">) => {
    toastId.current += 1;
    const entry: Toast = { ...toast, id: toastId.current, at: Date.now() };
    // Newest last so the stack reads top-down; the cap keeps a chatty group
    // from filling the corner, and a question outranks a notice when it does.
    setToasts((previous) => [...previous, entry].slice(-TOAST_LIMIT));
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
      // The share routes resolve the caller from this header, and the host
      // refuses a link the session does not own. The SDK's ticket link is the
      // same one the host registered for this socket, so they always agree.
      setLinkId(client.linkId ?? self.linkId);
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
        if (!hello.ask) {
          // A greeting is already acknowledged by the SDK, so this tab has
          // nothing to decide: it arrives as a notice and goes away on its own.
          pushToast({
            title: `${from} 向你打招呼`,
            body: hello.text || "hello",
            tone: "info",
            ttlMs: 5000,
          });
          log(`收到 ${from} 的 hello`, "in");
          return;
        }
        // A question is not answered by this client — only by the person
        // looking at it, so it stays until they confirm or dismiss it. The
        // sender is timing out on a real human either way.
        const fields = Object.keys(hello.payload);
        pushToast({
          title: `${from} 提问`,
          body: hello.text || "请回答",
          tone: "info",
          replyId: hello.replyId,
          fields,
          ttlMs: 0,
        });
        log(`收到 ${from} 的提问：${fields.join("、") || "无字段"}`, "in");
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
  }, [log, pushToast, refresh, refreshFiles]);

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
    (target: ClientSummary, ask?: string) => {
      const client = clientRef.current;
      const linkId = target.linkId;
      const name = label(target);
      if (!client || !linkId) return;
      // The question's field name *is* the callback argument: whatever the
      // sender typed is the key the target's confirm form is asked to fill.
      const field = ask?.trim();
      const payload = field ? { [field]: "" } : undefined;
      void (async () => {
        try {
          // A question carries a payload, so the host waits for a real answer
          // instead of the target's automatic acknowledgement.
          const result = await client.sendHello(
            linkId,
            payload ? "请回答这个问题" : "hello",
            payload,
          );
          const measured = Number(result.roundTripMs ?? "0");
          // A sub-millisecond result is real but reads as zero; say so rather
          // than rounding a valid measurement into a fake one.
          const shown = measured < 1 ? "<1 ms" : `${measured} ms`;
          const answered = Object.entries(result.answer ?? {});
          pushToast({
            title: payload ? `${name} 的回答` : `已发送 hello 给 ${name}`,
            body: answered.length
              ? answered.map(([field, value]) => `${field}：${String(value)}`).join("，")
              : `${name} 已确认，${shown}`,
            tone: "success",
            ttlMs: payload ? 0 : 5000,
          });
          log(`→ ${name}：${shown}，回复「${result.reply}」`, "out");
        } catch (error) {
          pushToast({
            title: `发送给 ${name} 失败`,
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
            ttlMs: 8000,
          });
          log(`→ ${name} 失败：${error instanceof Error ? error.message : String(error)}`, "out");
        }
        await refresh();
      })();
    },
    [label, log, pushToast, refresh],
  );

  const answerToast = useCallback(
    (toast: Toast, values: Record<string, string>) => {
      const client = clientRef.current;
      if (!client || !toast.replyId) return;
      const answer: Record<string, string> = {};
      for (const field of toast.fields ?? Object.keys(values)) {
        const value = values[field]?.trim();
        // An empty field is omitted rather than sent as "": the sender gets a
        // shorter answer instead of a field that looks answered and is not.
        if (value) answer[field] = value;
      }
      if (Object.keys(answer).length === 0) return;
      if (!client.answerHello(toast.replyId, answer, "已回答")) {
        pushToast({
          title: "回答未送达",
          body: "连接已断开，这条提问没有发出去",
          tone: "error",
          ttlMs: 8000,
        });
        return;
      }
      dismissToast(toast.id);
      log(`已回答 ${toast.title}：${Object.keys(answer).join("、")}`, "out");
    },
    [dismissToast, log, pushToast],
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
    toasts,
    draft,
    savedNotice,
    status,
    files,
    shareNotice,
    uploading,
    setDraft,
    save,
    greet,
    answerToast,
    dismissToast,
    shareFiles,
    withdrawFile,
    fileUrl: downloadFileUrl,
    refreshFiles,
    refreshPeers: refreshFiles,
  };
}
