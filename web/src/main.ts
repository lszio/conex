import { ConexWsClient, type EndpointSummary, type UiLinkSummary } from "@conex/sdk";
import {
  appendEvent,
  bindLogin,
  bindLogout,
  bindOperationTabs,
  renderCatalog,
  renderConnections,
  renderError,
  renderResult,
  renderSelected,
  setHostStatus,
  setStage,
  showApp,
  showLogin,
  type Operation,
} from "./view";

let csrfToken = "";
let client: ConexWsClient | undefined;
let endpoints: EndpointSummary[] = [];
let selected: EndpointSummary | undefined;
let operation: Operation = "list";
let pollTimer = 0;
let pollInFlight = false;
let connectionsPollTimer = 0;
let connectionsPollInFlight = false;
let connectionRows: UiLinkSummary[] = [];

function log(phase: string, message: string, method?: string): void {
  appendEvent({ at: Date.now(), phase, message, method });
}

async function readError(response: Response): Promise<string> {
  try {
    const body = await response.json() as { message?: string };
    return body.message || `${response.status} ${response.statusText}`;
  } catch {
    return `${response.status} ${response.statusText}`;
  }
}

async function session(): Promise<boolean> {
  const response = await fetch("/web/session", { credentials: "same-origin" });
  if (!response.ok) return false;
  const body = await response.json() as { csrf?: string; principalId?: string };
  csrfToken = body.csrf || "";
  log("认证", `已恢复会话 ${body.principalId || ""}`);
  return Boolean(csrfToken);
}

async function login(token: string): Promise<void> {
  setStage("auth", "正在验证访问凭据…");
  setHostStatus("认证中", "pending");
  const response = await fetch("/web/login", {
    method: "POST",
    credentials: "same-origin",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!response.ok) throw new Error(await readError(response));
  if (!(await session())) throw new Error("登录成功但会话不可用");
  showApp();
  await connect();
}

async function connect(): Promise<void> {
  client?.close();
  setStage("transport", "正在建立安全连接…");
  setHostStatus("连接中", "pending");
  client = new ConexWsClient({ origin: location.origin, csrfToken, reconnect: true });
  client.onEvent((event) => {
    const phase = event.phase || "连接";
    const message = event.summary || "状态更新";
    log(phase, message, event.method);
    if (phase === "ticket" || phase === "authenticating") {
      setStage("auth", "正在获取本次连接票据…");
    } else if (phase === "connecting" || phase === "negotiating") {
      setStage("transport", "正在建立安全连接…");
    } else if (phase === "hello") {
      setStage("hello", "Host hello 握手已发送…");
    } else if (phase === "reconnecting") {
      setHostStatus("重连中", "pending");
      setStage("transport", "连接中断，正在重新认证并建立连接…");
    } else if (phase === "ready") {
      if (message === "ready") {
        setStage("done", "连接就绪，端点目录将自动更新");
        setHostStatus("已就绪", "ready");
        void refreshCatalog();
      } else {
        setStage("ready", "等待 Host 确认 ready…");
      }
    } else if (phase === "failed" || phase === "error") {
      setHostStatus("连接异常", "error");
    }
  });
  try {
    await client.connect();
    setStage("done", "连接就绪，端点目录将自动更新");
    setHostStatus("已就绪", "ready");
    await refreshCatalog();
    startPolling();
  } catch (error) {
    setHostStatus("连接失败", "error");
    setStage("transport", error instanceof Error ? error.message : "连接失败");
    log("错误", error instanceof Error ? error.message : "连接失败");
    throw error;
  }
}

async function refreshCatalog(): Promise<void> {
  if (!client || pollInFlight || document.hidden) return;
  pollInFlight = true;
  try {
    const result = await client.listEndpoints({ limit: 100 });
    endpoints = result.endpoints || [];
    renderCatalog(endpoints, selectEndpoint);
    if (selected) selected = endpoints.find((item) => item.endpointId === selected?.endpointId);
    renderSelected(selected, operation, invoke);
    log("目录", `已更新 ${endpoints.length} 个端点`);
  } catch (error) {
    log("错误", error instanceof Error ? error.message : "目录获取失败", "endpoint/list");
    setHostStatus("目录获取失败", "error");
  } finally {
    pollInFlight = false;
  }
}

function startPolling(): void {
  window.clearInterval(pollTimer);
  pollTimer = window.setInterval(() => { void refreshCatalog(); }, 3000);
  window.clearInterval(connectionsPollTimer);
  connectionsPollTimer = window.setInterval(() => { void refreshConnections(); }, 1000);
  void refreshConnections();
}

async function refreshConnections(): Promise<void> {
  if (!client || connectionsPollInFlight || document.hidden) return;
  connectionsPollInFlight = true;
  try {
    const result = await client.listConnections({});
    const next = (result.browserLinks ?? []) as UiLinkSummary[];
    connectionRows = next;
    renderConnections(next);
  } catch (err) {
    // Connection panel is best-effort; do not log each tick to avoid
    // overwhelming the timeline when the link is not yet ready.
  } finally {
    connectionsPollInFlight = false;
  }
}

function selectEndpoint(endpoint: EndpointSummary): void {
  selected = endpoint;
  renderSelected(selected, operation, invoke);
  log("目录", `已选择 ${endpoint.displayName || endpoint.endpointId || "端点"}`);
}

async function invoke(kind: Operation, input: Record<string, string>): Promise<void> {
  if (!client || !selected?.endpointId) return;
  renderError("");
  const method = `source/${kind}`;
  const started = performance.now();
  try {
    let result: unknown;
    if (kind === "list") result = await client.list(selected.endpointId, { root: input.root || "", limit: 50 });
    else if (kind === "read") result = await client.read(selected.endpointId, { resourceId: input.resourceId || "" });
    else result = await client.search(selected.endpointId, { root: "", query: input.query || "", limit: 50 });
    renderResult(result);
    log("调用成功", `${Math.round(performance.now() - started)} ms`, method);
  } catch (error) {
    const message = error instanceof Error ? error.message : "调用失败";
    renderError(message);
    log("调用失败", message, method);
  }
}

async function logout(): Promise<void> {
  window.clearInterval(pollTimer);
  window.clearInterval(connectionsPollTimer);
  client?.close();
  client = undefined;
  await fetch("/web/logout", { method: "POST", credentials: "same-origin", headers: { "x-csrf-token": csrfToken } });
  csrfToken = "";
  endpoints = [];
  selected = undefined;
  connectionRows = [];
  renderConnections([]);
  showLogin();
  setHostStatus("未连接", "");
  log("认证", "已退出登录");
}

bindLogin((token) => { void login(token).catch((error) => showLogin(error instanceof Error ? error.message : "登录失败")); });
bindLogout(() => { void logout(); });
bindOperationTabs((next) => {
  operation = next;
  renderSelected(selected, operation, invoke);
});
document.addEventListener("visibilitychange", () => {
  if (document.hidden) {
    window.clearInterval(pollTimer);
    window.clearInterval(connectionsPollTimer);
  } else if (client) {
    void refreshCatalog();
    void refreshConnections();
    startPolling();
  }
});

void session().then((active) => {
  if (active) { showApp(); return connect().catch((error) => showLogin(error instanceof Error ? error.message : "连接失败")); }
  showLogin();
}).catch((error) => showLogin(error instanceof Error ? error.message : "会话检查失败"));
