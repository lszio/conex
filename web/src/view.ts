import type { EndpointSummary, UiLinkSummary } from "@conex/sdk";

export type Operation = "list" | "read" | "search";
export type EventItem = { at: number; phase: string; message: string; method?: string };
export type ConnectionRow = UiLinkSummary;

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;

function connectionStateName(state: unknown): string {
  if (state === 1 || state === "CONNECTION_STATE_NOT_APPLICABLE") return "not_applicable";
  if (state === 2 || state === "CONNECTION_STATE_OFFLINE") return "offline";
  if (state === 3 || state === "CONNECTION_STATE_READY") return "ready";
  return String(state ?? "unspecified").toLowerCase();
}
export function showLogin(error = ""): void {
  $("#login-panel").hidden = false;
  $("#app-panel").hidden = true;
  const node = $("#login-error");
  node.textContent = error;
  node.hidden = !error;
}

export function showApp(): void {
  $("#login-panel").hidden = true;
  $("#app-panel").hidden = false;
}

export function setHostStatus(label: string, kind: string): void {
  const node = $("#host-status");
  node.textContent = label;
  node.className = `status-pill ${kind}`;
}

export function setStage(stage: string, detail: string): void {
  const order = ["auth", "transport", "hello", "ready", "done"];
  const current = Math.max(0, order.indexOf(stage));
  document.querySelectorAll<HTMLElement>("#stages li").forEach((node, index) => {
    node.classList.toggle("active", index === current);
    node.classList.toggle("complete", index < current);
  });
  $("#connection-detail").textContent = detail;
}

function endpointLabel(endpoint: EndpointSummary): string {
  return endpoint.displayName || endpoint.endpointId || "未命名端点";
}

export function renderCatalog(endpoints: EndpointSummary[], onSelect: (endpoint: EndpointSummary) => void): void {
  const list = $("#endpoint-list");
  const graph = $("#topology-graph");
  list.replaceChildren();
  graph.replaceChildren();
  $("#endpoint-count").textContent = `${endpoints.length} 个端点`;
  $("#catalog-state").textContent = endpoints.length ? "已更新" : "没有可见端点";

  const host = document.createElement("div");
  host.className = "topology-node host-node";
  host.textContent = "中心 Host";
  graph.append(host);
  const groups = new Map<string, HTMLElement>();
  for (const endpoint of endpoints) {
    const provider = endpoint.agentId || endpoint.providerId || "本地服务";
    let group = groups.get(provider);
    if (!group) {
      group = document.createElement("div");
      group.className = "topology-group";
      const heading = document.createElement("strong");
      heading.textContent = provider;
      group.append(heading);
      graph.append(group);
      groups.set(provider, group);
    }
    const node = document.createElement("span");
    const topologyState = connectionStateName(endpoint.connectionState);
    node.className = `topology-endpoint ${topologyState}`;
    node.textContent = endpointLabel(endpoint);
    group.append(node);

    const card = document.createElement("button");
    card.type = "button";
    card.className = "endpoint-card";
    const title = document.createElement("strong");
    title.textContent = endpointLabel(endpoint);
    const meta = document.createElement("span");
    meta.textContent = `${provider} · ${endpoint.region || "未标注地域"}`;
    const state = document.createElement("span");
    const connectionState = connectionStateName(endpoint.connectionState);
    const online = connectionState === "ready" || connectionState === "not_applicable";
    state.className = `state ${online ? "online" : "offline"}`;
    state.textContent = online ? "可调用" : "离线（可查看授权）";
    card.append(title, meta, state);
    card.addEventListener("click", () => onSelect(endpoint));
    list.append(card);
  }
  if (!endpoints.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "当前身份没有获准端点，或获准端点均未上线。";
    list.append(empty);
  }
}

export function renderSelected(endpoint: EndpointSummary | undefined, operation: Operation, submit: (operation: Operation, input: Record<string, string>) => void): void {
  $("#selected-endpoint").textContent = endpoint ? `${endpointLabel(endpoint)} · ${endpoint.endpointId || ""}` : "选择一个端点查看可用操作";
  const form = $("#operation-form");
  form.replaceChildren();
  const connectionState = connectionStateName(endpoint?.connectionState);
  const offline = Boolean(endpoint) && connectionState !== "ready" && connectionState !== "not_applicable";
  const scopes = endpoint?.authorizedScopes || [];
  const allowed = (method: string) => endpoint?.availableMethods?.includes(method) && scopes.some((scope) => scope.method === method);
  const method = `source/${operation}`;
  const submitButton = document.createElement("button");
  submitButton.type = "submit";
  submitButton.textContent = offline ? "端点离线" : `执行 ${method}`;
  submitButton.disabled = Boolean(offline) || !allowed(method);
  if (offline) submitButton.title = "端点离线，恢复在线后可调用";
  const root = document.createElement("input");
  root.name = "root";
  root.placeholder = "授权根目录（默认空）";
  root.value = scopes.find((scope) => scope.method === method)?.root || "";
  if (operation === "read") {
    root.name = "resourceId";
    root.placeholder = "资源名，例如 notes/readme.md";
  }
  if (operation === "search") {
    root.name = "query";
    root.placeholder = "搜索词";
  }
  form.append(root, submitButton);
  form.onsubmit = (event: SubmitEvent) => {
    event.preventDefault();
    submit(operation, { [root.name]: root.value.trim() });
  };
  document.querySelectorAll<HTMLButtonElement>("[data-operation]").forEach((button) => {
    button.setAttribute("aria-selected", button.dataset.operation === operation ? "true" : "false");
  });
}

export function renderResult(value: unknown): void {
  $("#operation-result").textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function shortLinkId(id: string | undefined): string {
  if (!id) return "";
  return id.length > 8 ? `${id.slice(0, 6)}…${id.slice(-2)}` : id;
}

function formatTimestamp(ms: string | number | undefined): string {
  if (!ms) return "—";
  const value = typeof ms === "string" ? Number(ms) : ms;
  if (!Number.isFinite(value) || value <= 0) return "—";
  return new Date(value).toLocaleTimeString();
}

function diffNew(prev: Set<string> | null, next: Set<string>): Set<string> {
  if (!prev) return new Set();
  const added = new Set<string>();
  for (const id of next) {
    if (!prev.has(id)) added.add(id);
  }
  return added;
}

export function renderConnections(rows: readonly ConnectionRow[]): void {
  const list = $("#connection-list");
  const counter = $("#connection-count");
  const stamp = $("#connection-stamp");
  if (!list || !counter) return;
  list.replaceChildren();
  const ids = rows
    .map((row) => row.linkId ?? "")
    .filter((id) => id.length > 0);
  counter.textContent = `${rows.length} 个连接`;
  if (stamp) stamp.textContent = new Date().toLocaleTimeString();
  if (rows.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "本主体尚无浏览器 UI 链接。";
    list.append(empty);
    return;
  }
  const knownIds = new Set(ids);
  // Mark each row that just appeared with .flash; consumers can use the
  // existing flash animation defined in style.css.
  for (const row of rows) {
    const card = document.createElement("div");
    card.className = "connection-card";
    card.dataset.connectionId = row.linkId ?? "";
    const id = document.createElement("strong");
    id.textContent = shortLinkId(row.linkId);
    const meta = document.createElement("span");
    const principal = row.principalId ?? "—";
    const tenant = row.tenantId ?? "—";
    meta.textContent = `${principal} · ${tenant}`;
    const times = document.createElement("span");
    times.className = "muted";
    times.textContent = `connected ${formatTimestamp(row.connectedAtMs)} · seen ${formatTimestamp(row.lastSeenAtMs)}`;
    const stats = document.createElement("span");
    const total = Number(row.callsTotal ?? "0");
    const inflight = Number(row.callsInFlight ?? "0");
    const tickets = Number(row.ticketsIssued ?? "0");
    stats.className = "stats";
    stats.textContent = `calls ${total} · in-flight ${inflight} · tickets ${tickets}`;
    card.append(id, meta, times, stats);
    list.append(card);
  }
  // Diff tracking is performed by the caller; this renderer is pure.
  void diffNew(null, knownIds);
}

export function renderError(message: string): void {
  const node = $("#operation-error");
  node.textContent = message;
  node.hidden = !message;
}

export function appendEvent(event: EventItem): void {
  const list = $("#events");
  const item = document.createElement("li");
  const time = new Date(event.at).toLocaleTimeString();
  item.textContent = `${time} · ${event.phase}${event.method ? ` · ${event.method}` : ""} · ${event.message}`;
  list.prepend(item);
  while (list.children.length > 200) list.lastElementChild?.remove();
}

export function bindLogin(handler: (token: string) => void): void {
  $("#login-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const input = $("#token") as HTMLInputElement;
    const token = input.value;
    input.value = "";
    handler(token);
  });
}

export function bindLogout(handler: () => void): void { $("#logout").addEventListener("click", handler); }
export function bindOperationTabs(handler: (operation: Operation) => void): void {
  document.querySelectorAll<HTMLButtonElement>("[data-operation]").forEach((button) => button.addEventListener("click", () => handler(button.dataset.operation as Operation)));
}
