/**
 * 私有产品客户端（P5/P7）：hub 会话与目录、direct grant、节点一次性票据和
 * 直连 WSS JSON-RPC 通道。
 *
 * 边界（docs/runbooks/private-network.md）：
 * - 引导码只出现在 HTTPS 请求正文；cookie 由浏览器持有，CSRF 存内存。
 * - grant 由 hub 签发（签名人/租户/epoch/代际都来自 hub 与节点自己的声明，
 *   客户端不能自报 principal）；purpose 固定，direct grant 不能经中心转发。
 * - 节点直连必须带精确批准的 Origin；票据单次消费、不进 URL；每帧由节点
 *   重查撤权与代际。
 * - 一个直连 WSS 绑定一份 grant（scope 含单个模块代际），因此一个通道只调
 *   一个模块的方法；跨模块的业务各开通道。
 */

import { ConexError } from "./errors";
import { ErrorCode } from "./generated/conex/common";
import { mintUlid } from "./client";
import { isRecord } from "./s1-wire";

/** 与 ws-client 的 WsFetch 一致的取数签名；浏览器/Bun 的 fetch 类型差异由
 * 调用方传入的实现满足，不把某个运行时的 fetch 类型写进公共接口。 */
export type PrivateFetch = (url: string | URL, init?: RequestInit) => Promise<Response>;

/** 服务端选择的真实 JSON profile；与 direct_api.rs PROFILE_JSON 一致。 */
export const DIRECT_PROFILE = "conex-jsonrpc2-wss";

export type DirectChannelState =
  | "idle"
  | "connecting"
  | "ready"
  | "reconnecting"
  | "closed";

export interface PrivateSessionInfo {
  principalId: string;
  tenantId: string;
  role: string;
  csrf: string;
  expiresAtMs: string;
}

export interface DirectoryNode {
  principal: string;
  nodeId: string;
  nodeEpoch: string;
  registeredAtMs: string;
  instances: DirectoryInstance[];
}

export interface DirectoryInstance {
  id: string;
  appId: string;
  state: string;
  modules: DirectoryModule[];
}

export interface DirectoryModule {
  name: string;
  moduleKey: string;
  generation: string;
  publication: string;
  processId?: number | null;
  contract?: { contractId?: string; methods?: string[] } | undefined;
}

export interface Directory {
  nodes: DirectoryNode[];
}

export interface GrantScopeBody {
  spaceId: string;
  providerEndpointId: string;
  contractId: string;
  moduleGeneration: string;
  methods: string[];
  states: string[];
  workspaceIds: string[];
  conversationIds: string[];
  purpose: "direct" | "relay";
}

export interface Grant {
  token: string;
  jti: string;
  nodeId: string;
  nodeEpoch: string;
  expiresAtMs: string;
  scope: GrantScopeBody;
}

export interface DirectInvokeInput {
  instanceId: string;
  module: string;
  generation: string;
  method: string;
  argument: unknown;
}

export interface DirectClientOptions {
  /** 节点直连入口的 http(s) origin（例如 http://127.0.0.1:7451）。 */
  nodeOrigin: string;
  /** 每次建立连接前取一份 direct grant（含正确代际与方法清单）。 */
  grantProvider: (module: string, generation: string) => Promise<Grant>;
  /** 页面 origin；票据与 WSS 升级的 Origin 校验用它逐字比对。 */
  fetch?: PrivateFetch;
  WebSocket?: typeof WebSocket;
  timeoutMs?: number;
  wsPath?: string;
  onState?: (state: DirectChannelState, detail?: string) => void;
}

function directError(message: string, code: number, retry: "safe" | "never" = "safe"): ConexError {
  return new ConexError(message, { code, retry });
}

async function readError(response: Response): Promise<ConexError> {
  let message = `HTTP ${response.status}`;
  try {
    const body = (await response.json()) as unknown;
    if (isRecord(body) && typeof body.message === "string") message = body.message;
    else if (isRecord(body) && isRecord(body.error) && typeof body.error.message === "string") {
      message = body.error.message;
    }
  } catch {
    // 非 JSON 错误体保留状态码消息。
  }
  const code = response.status === 401
    ? ErrorCode.ERROR_CODE_UNAUTHORIZED
    : response.status === 403
      ? ErrorCode.ERROR_CODE_FORBIDDEN
      : ErrorCode.ERROR_CODE_INTERNAL;
  // status 必须随错误透传：入口分发靠它区分"私有未登录"与"非私有部署"。
  return new ConexError(message, { code, retry: "safe", status: response.status });
}

function wsUrl(origin: string, path: string): string {
  const url = new URL(path, origin);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

/** hub 侧私有入口（同源部署时 origin 留空走相对路径）。cookie 由浏览器
 * 自动携带；CSRF 由 loginPrivate 返回并由调用方保存，随后随每个写请求回传。
 * 注意：浏览器把 Origin 列为禁止手设的头，这里不手动携带——同源请求由
 * 浏览器按需自动附加，票据/升级的 Origin 校验由真实页面 origin 满足。 */
export class PrivateHubClient {
  readonly origin: string;
  private readonly fetchImpl: PrivateFetch;
  csrf: string | null = null;

  constructor(origin: string, fetchImpl?: PrivateFetch) {
    this.origin = origin;
    this.fetchImpl = fetchImpl ?? ((url, init) => fetch(url, init));
  }

  private url(path: string): string {
    return this.origin ? new URL(path, this.origin).toString() : path;
  }

  private async post(path: string, body: unknown, withCsrf: boolean): Promise<Response> {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (withCsrf) {
      if (!this.csrf) throw directError("no private session", ErrorCode.ERROR_CODE_UNAUTHORIZED);
      headers["x-csrf-token"] = this.csrf;
    }
    return this.fetchImpl(this.url(path), {
      method: "POST",
      credentials: "include",
      headers,
      body: JSON.stringify(body),
      redirect: "error",
    });
  }

  /** 引导码换私有会话；成功后 csrf 保存在实例里（不落 localStorage）。 */
  async login(code: string): Promise<PrivateSessionInfo> {
    const response = await this.post("/private/session", { code }, false);
    if (!response.ok) throw await readError(response);
    const body = (await response.json()) as Record<string, unknown>;
    if (typeof body.csrf !== "string" || body.csrf === "") {
      throw directError("session response is missing csrf", ErrorCode.ERROR_CODE_INTERNAL, "never");
    }
    this.csrf = body.csrf;
    return {
      principalId: String(body.principalId ?? ""),
      tenantId: String(body.tenantId ?? ""),
      role: String(body.role ?? ""),
      csrf: body.csrf,
      expiresAtMs: String(body.expiresAtMs ?? "0"),
    };
  }

  async directory(): Promise<Directory> {
    const response = await this.fetchImpl(this.url("/private/directory"), {
      credentials: "include",
      redirect: "error",
    });
    if (!response.ok) throw await readError(response);
    const body = (await response.json()) as unknown;
    if (!isRecord(body) || !Array.isArray(body.nodes)) {
      throw directError("directory response is malformed", ErrorCode.ERROR_CODE_INTERNAL, "never");
    }
    return body as unknown as Directory;
  }

  /** 账号路由 A：是否已存在本地管理员账号（登录页据此选表单）。 */
  async accountStatus(): Promise<{ accountExists: boolean }> {
    const response = await this.fetchImpl(this.url("/private/account/status"), {
      credentials: "include",
      redirect: "error",
    });
    if (!response.ok) throw await readError(response);
    const body = (await response.json()) as Record<string, unknown>;
    return { accountExists: body.accountExists === true };
  }

  /** 账号密码登录（仅已建号的部署）；失败不区分用户名与密码错误。 */
  async loginPassword(username: string, password: string): Promise<PrivateSessionInfo> {
    const response = await this.post(
      "/private/account/password-session",
      { username, password },
      false,
    );
    if (!response.ok) throw await readError(response);
    const body = (await response.json()) as Record<string, unknown>;
    if (typeof body.csrf !== "string" || body.csrf === "") {
      throw directError("session response is missing csrf", ErrorCode.ERROR_CODE_INTERNAL, "never");
    }
    this.csrf = body.csrf;
    return {
      principalId: String(body.principalId ?? ""),
      tenantId: String(body.tenantId ?? ""),
      role: String(body.role ?? ""),
      csrf: body.csrf,
      expiresAtMs: String(body.expiresAtMs ?? "0"),
    };
  }

  /** 首次建号：仅引导码会话且账号表为空时成功（服务端强校验）。 */
  async createAccount(username: string, password: string): Promise<void> {
    const response = await this.post("/private/account", { username, password }, true);
    if (!response.ok) throw await readError(response);
  }

  /** 设置/重置本地账号密码；需要已认证的私有会话（引导码恢复路径同样适用）。 */
  async setAccountPassword(username: string, password: string): Promise<void> {
    const response = await this.post("/private/account/password", { username, password }, true);
    if (!response.ok) throw await readError(response);
  }

  async grant(nodeId: string, scope: GrantScopeBody): Promise<Grant> {
    const response = await this.post("/private/grant", { nodeId, scope }, true);
    if (!response.ok) throw await readError(response);
    const body = (await response.json()) as Record<string, unknown>;
    if (typeof body.token !== "string" || body.token === "") {
      throw directError("grant response is missing token", ErrorCode.ERROR_CODE_INTERNAL, "never");
    }
    return {
      token: body.token,
      jti: String(body.jti ?? ""),
      nodeId: String(body.nodeId ?? nodeId),
      nodeEpoch: String(body.nodeEpoch ?? ""),
      expiresAtMs: String(body.expiresAtMs ?? "0"),
      scope,
    };
  }
}

/** 节点直连通道：一份 grant → 一张一次性票据 → 一条 WSS。断线自动重建
 * （重新取 grant——节点重启后旧 epoch 授权必然失效）。invoke 按帧 id 关联，
 * 服务端单帧顺序处理，无并发回复乱序问题。 */
export class NodeDirectClient {
  readonly module: string;
  readonly generation: string;
  state: DirectChannelState = "idle";
  private readonly options: DirectClientOptions;
  private readonly fetchImpl: PrivateFetch;
  private readonly WebSocketImpl: typeof WebSocket;
  private readonly timeoutMs: number;
  private socket: WebSocket | null = null;
  private readonly pending = new Map<string, {
    resolve: (value: unknown) => void;
    reject: (error: unknown) => void;
    timer: ReturnType<typeof setTimeout>;
  }>();
  private generation_ = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private connectPromise: Promise<void> | null = null;
  private closed = false;

  constructor(module: string, generation: string, options: DirectClientOptions) {
    this.module = module;
    this.generation = generation;
    this.options = options;
    this.fetchImpl = options.fetch ?? ((url, init) => fetch(url, init));
    const ws = options.WebSocket ?? globalThis.WebSocket;
    if (!ws) throw directError("WebSocket is not available", ErrorCode.ERROR_CODE_INTERNAL, "never");
    this.WebSocketImpl = ws;
    this.timeoutMs = options.timeoutMs ?? 120_000;
  }

  private setState(state: DirectChannelState, detail?: string): void {
    this.state = state;
    this.options.onState?.(state, detail);
  }

  /** 连接（或取现有在途连接）。失败不自动放弃：退避后整体重建。 */
  connect(): Promise<void> {
    if (this.closed) return Promise.reject(directError("client is closed", ErrorCode.ERROR_CODE_BAD_REQUEST, "never"));
    if (this.state === "ready") return Promise.resolve();
    if (this.connectPromise) return this.connectPromise;
    this.setState(this.generation_ === 0 ? "connecting" : "reconnecting");
    this.connectPromise = this.establish()
      .then(() => {
        this.generation_ += 1;
        this.connectPromise = null;
      })
      .catch((error) => {
        this.connectPromise = null;
        if (this.closed) return;
        this.setState("reconnecting", error instanceof Error ? error.message : String(error));
        this.scheduleReconnect();
        throw error;
      });
    return this.connectPromise;
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer || this.closed) return;
    const delay = Math.min(15000, 1000 * 2 ** Math.min(this.generation_, 4));
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect().catch(() => { /* 状态已上报；下一轮退避继续 */ });
    }, delay);
  }

  private async establish(): Promise<void> {
    const grant = await this.options.grantProvider(this.module, this.generation);
    const response = await this.fetchImpl(new URL("/private/tickets", this.options.nodeOrigin), {
      method: "POST",
      headers: {
        // 票据端点要求精确批准的 Origin；浏览器 fetch 禁止手设 Origin 头，
        // 会以页面真实 origin 自动附加（必须在节点批准清单里）。
        Authorization: `Bearer ${grant.token}`,
      },
      redirect: "error",
    });
    if (!response.ok) throw await readError(response);
    const body = (await response.json()) as Record<string, unknown>;
    if (typeof body.ticket !== "string" || body.ticket === "") {
      throw directError("ticket response is missing ticket", ErrorCode.ERROR_CODE_INTERNAL, "never");
    }
    const profile = typeof body.profile === "string" ? body.profile : DIRECT_PROFILE;
    if (profile !== DIRECT_PROFILE) {
      throw directError(`node selected unexpected profile ${profile}`, ErrorCode.ERROR_CODE_INTERNAL, "never");
    }
    const socket = new this.WebSocketImpl(
      wsUrl(this.options.nodeOrigin, this.options.wsPath ?? "/wss"),
      [DIRECT_PROFILE, `conex-ticket.${body.ticket}`],
    );
    this.socket = socket;
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(directError("direct websocket open timed out", ErrorCode.ERROR_CODE_TIMEOUT)),
        this.timeoutMs,
      );
      socket.onopen = () => { clearTimeout(timer); resolve(); };
      socket.onerror = () => {
        clearTimeout(timer);
        reject(directError("direct websocket failed", ErrorCode.ERROR_CODE_UNAVAILABLE));
      };
      socket.onclose = () => {
        clearTimeout(timer);
        reject(directError("direct websocket closed during open", ErrorCode.ERROR_CODE_UNAVAILABLE));
      };
    });
    socket.onmessage = (event) => this.handleFrame(String(event.data));
    socket.onclose = () => this.handleClose("closed");
    socket.onerror = () => this.handleClose("error");
    this.setState("ready");
  }

  private handleClose(_why: string): void {
    for (const entry of this.pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(directError("direct channel closed", ErrorCode.ERROR_CODE_UNAVAILABLE));
    }
    this.pending.clear();
    this.socket = null;
    if (this.closed) {
      this.setState("closed");
      return;
    }
    this.setState("reconnecting", "connection lost");
    this.scheduleReconnect();
  }

  private handleFrame(text: string): void {
    let frame: unknown;
    try {
      frame = JSON.parse(text);
    } catch {
      return;
    }
    if (!isRecord(frame)) return;
    const id = typeof frame.id === "string" ? frame.id : undefined;
    if (id === undefined) return;
    const entry = this.pending.get(id);
    if (!entry) return;
    this.pending.delete(id);
    clearTimeout(entry.timer);
    if (frame.error !== undefined && frame.error !== null) {
      const error = isRecord(frame.error) ? frame.error : {};
      entry.reject(new ConexError(String(error.message ?? "direct invoke failed"), {
        code: typeof error.code === "number" ? error.code : ErrorCode.ERROR_CODE_INTERNAL,
        retry: error.retry === "never" ? "never" : "safe",
        execution: typeof error.execution === "string" ? error.execution : undefined,
      }));
      return;
    }
    entry.resolve(frame.result);
  }

  /** node/invoke；结果即该方法的 result 值。 */
  async invoke<I, O>(input: DirectInvokeInput): Promise<O> {
    if (this.state !== "ready" || !this.socket || this.socket.readyState !== 1) {
      throw directError("direct channel is not ready", ErrorCode.ERROR_CODE_UNAVAILABLE);
    }
    const id = mintUlid();
    const frame = {
      jsonrpc: "2.0",
      id,
      method: "node/invoke",
      params: {
        context: { plane: "broker" },
        input,
        timeoutBudgetMs: this.timeoutMs,
      },
    };
    const socket = this.socket;
    return new Promise<O>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(directError("direct invoke timed out", ErrorCode.ERROR_CODE_TIMEOUT));
      }, this.timeoutMs);
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject, timer });
      try {
        socket.send(JSON.stringify(frame));
      } catch (error) {
        this.pending.delete(id);
        clearTimeout(timer);
        reject(directError(error instanceof Error ? error.message : "send failed", ErrorCode.ERROR_CODE_UNAVAILABLE));
      }
    });
  }

  close(): void {
    this.closed = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.handleClose("client closed");
    try {
      this.socket?.close();
    } catch {
      // 浏览器在升级竞态下 close 可能抛错；通道状态已收敛。
    }
    this.socket = null;
    this.setState("closed");
  }
}
