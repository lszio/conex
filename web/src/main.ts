// The hello page: connect, list online clients, greet one, and keep this
// visitor's own name/group/visibility. Everything is a real call over the
// same WSS link the protocol defines; there is no local mock state.

import { ConexWsClient, type ClientProfile, type ClientSummary } from "@conex/sdk";

const POLL_MS = 2000;
const LOG_LIMIT = 30;

const el = <T extends HTMLElement>(selector: string): T =>
  document.querySelector<T>(selector)!;

let csrfToken = "";
let client: ConexWsClient | undefined;
/** This visitor's own link id, used to render the "you" row. */
let selfLinkId = "";
let rows: ClientSummary[] = [];
let pollTimer = 0;
let polling = false;

const logEl = el<HTMLOListElement>("#log");
const clientsEl = el<HTMLUListElement>("#clients");
const statusEl = el<HTMLParagraphElement>("#status");
const countEl = el<HTMLSpanElement>("#client-count");
const profileState = el<HTMLParagraphElement>("#profile-state");
const nameInput = el<HTMLInputElement>("#profile-name");
const groupInput = el<HTMLInputElement>("#profile-group");
const visibleInput = el<HTMLInputElement>("#profile-visible");

function setStatus(label: string, kind: "" | "ready" | "error" = ""): void {
  statusEl.textContent = label;
  statusEl.className = `status-pill ${kind}`.trim();
}

function log(message: string): void {
  const item = document.createElement("li");
  const time = document.createElement("span");
  time.className = "log-time";
  time.textContent = new Date().toLocaleTimeString();
  const text = document.createElement("span");
  text.textContent = message;
  item.append(time, text);
  logEl.prepend(item);
  while (logEl.children.length > LOG_LIMIT) logEl.lastElementChild?.remove();
}

function labelOf(row: ClientSummary): string {
  return row.profile?.displayName || row.linkId?.slice(0, 6) || "?";
}

function ago(ms: string | undefined): string {
  const value = Number(ms ?? "0");
  if (!Number.isFinite(value) || value <= 0) return "—";
  const seconds = Math.max(0, Math.round((Date.now() - value) / 1000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  return `${Math.round(seconds / 3600)}h`;
}

function render(): void {
  countEl.textContent = `${rows.length} 个`;
  clientsEl.replaceChildren();
  if (rows.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "还没有其他可见的客户端。开一个页面就能互相 hello。";
    clientsEl.append(empty);
    return;
  }
  // Group first, then name, so the visitor's own grouping is what they see.
  const sorted = [...rows].sort((a, b) => {
    const groupA = a.profile?.group ?? "";
    const groupB = b.profile?.group ?? "";
    if (groupA !== groupB) return groupA.localeCompare(groupB);
    return labelOf(a).localeCompare(labelOf(b));
  });
  for (const row of sorted) {
    const linkId = row.linkId;
    // A row without an id cannot be addressed or compared, so it is not a
    // usable client; the host always sets it, so this only guards decoding.
    if (!linkId) continue;
    const item = document.createElement("li");
    item.className = "client";

    const identity = document.createElement("div");
    identity.className = "client-id";
    const name = document.createElement("strong");
    name.textContent = labelOf(row);
    const meta = document.createElement("span");
    meta.className = "muted";
    const group = row.profile?.group || "未分组";
    meta.textContent = `${group} · 活跃于 ${ago(row.lastSeenAtMs)} 前`;
    identity.append(name, meta);
    if (linkId === selfLinkId) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = "你";
      identity.append(badge);
    }

    const hello = document.createElement("button");
    hello.type = "button";
    hello.className = "secondary";
    hello.textContent = linkId === selfLinkId ? "hello 给自己" : "发送 hello";
    hello.addEventListener("click", () => { void greet(row); });

    item.append(identity, hello);
    clientsEl.append(item);
  }
}

async function greet(target: ClientSummary): Promise<void> {
  const linkId = target.linkId;
  if (!client || !linkId) return;
  const name = labelOf(target);
  const self = rows.find((row) => row.linkId === selfLinkId);
  const sender = self?.profile?.displayName || nameInput.value.trim() || "我";
  try {
    const result = await client.sendHello(linkId, `hello ${sender}`);
    // The host measures the full round trip. Sub-millisecond results are real
    // but read as "0 ms" and say nothing useful, so they are shown as such
    // rather than rounded into a fake number.
    const measured = Number(result.roundTripMs ?? "0");
    const shown = measured < 1 ? "<1 ms" : `${measured} ms`;
    log(`→ ${name}：${shown}，回复「${result.reply}」`);
  } catch (error) {
    log(`→ ${name} 失败：${error instanceof Error ? error.message : String(error)}`);
  }
}

async function refresh(): Promise<void> {
  if (!client || polling) return;
  polling = true;
  try {
    const result = await client.listClients();
    rows = result.clients ?? [];
    render();
  } catch (error) {
    setStatus(`列表失败：${error instanceof Error ? error.message : String(error)}`, "error");
  } finally {
    polling = false;
  }
}

async function saveProfile(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  if (!client) return;
  const profile: ClientProfile = {
    displayName: nameInput.value.trim(),
    group: groupInput.value.trim(),
    visible: visibleInput.checked,
  };
  try {
    const result = await client.setProfile(profile);
    const self = result.self;
    if (self?.linkId) selfLinkId = self.linkId;
    profileState.textContent = `已保存 · 你的链接 ${selfLinkId.slice(0, 8)}`;
    log(`保存资料：${self?.profile?.displayName || "未命名"}${profile.visible ? "（可见）" : "（已隐藏）"}`);
    await refresh();
  } catch (error) {
    profileState.textContent = `保存失败：${error instanceof Error ? error.message : String(error)}`;
  }
}

async function session(): Promise<boolean> {
  const response = await fetch("/web/session", { credentials: "same-origin" });
  if (!response.ok) return false;
  const body = (await response.json()) as { csrf?: string; principalId?: string };
  csrfToken = body.csrf ?? "";
  return Boolean(csrfToken);
}

async function connect(): Promise<void> {
  setStatus("连接中…");
  client = new ConexWsClient({ origin: location.origin, csrfToken, reconnect: true });
  client.onEvent((event) => {
    if (event.phase === "ready") {
      setStatus("已连接", "ready");
    } else if (event.phase === "reconnecting") {
      setStatus("重连中…");
    } else if (event.phase === "failed" || event.phase === "error") {
      setStatus("连接异常", "error");
    }
  });
  // Greetings pushed by other clients: the SDK already replied with a pong,
  // so this only records that it happened.
  client.onHello((hello) => {
    log(`← 收到 hello：${hello.text}`);
  });
  await client.connect();
  // Push the current form values so the visitor appears with a usable name.
  await client.setProfile({
    displayName: nameInput.value.trim(),
    group: groupInput.value.trim(),
    visible: visibleInput.checked,
  });
  await refresh();
  window.clearInterval(pollTimer);
  pollTimer = window.setInterval(() => { void refresh(); }, POLL_MS);
}

el<HTMLFormElement>("#profile-form").addEventListener("submit", (event) => {
  void saveProfile(event as SubmitEvent);
});

void session()
  .then((active) => {
    if (!active) {
      setStatus("会话不可用，请刷新页面", "error");
      return undefined;
    }
    return connect();
  })
  .catch((error) => {
    setStatus(`连接失败：${error instanceof Error ? error.message : String(error)}`, "error");
  });
