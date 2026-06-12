// Anytype REST API 客户端
// 基于 Anytype OpenAPI v2025-11-08 的实际端点
//
// 关键端点:
//   GET  /v1/spaces                            — 列出空间
//   GET  /v1/spaces/{space_id}                 — 获取空间详情
//   GET  /v1/spaces/{space_id}/objects         — 列出对象
//   GET  /v1/spaces/{space_id}/objects/{id}    — 获取对象详情
//   PATCH /v1/spaces/{space_id}/objects/{id}   — 更新对象
//   POST /v1/search                            — 全局搜索
//   POST /v1/spaces/{space_id}/search          — 空间内搜索
//   GET  /v1/spaces/{space_id}/types           — 列出类型

import { loadCredentials, hasApiKey } from "./auth.js";
import type {
  AnytypeObject,
  AnytypeSpace,
  AnytypeListResponse,
} from "./types.js";
import type { AdapterStatus } from "../types.js";

export class AnytypeAdapterError extends Error {
  status: number;
  code: string;
  constructor(message: string, status: number, code: string) {
    super(message);
    this.name = "AnytypeAdapterError";
    this.status = status;
    this.code = code;
  }
}

export class AnytypeAdapter {
  private baseUrl: string;
  private apiKey: string;
  private apiVersion: string;
  private configured: boolean = false;

  private constructor() {
    this.baseUrl = "";
    this.apiKey = "";
    this.apiVersion = "";
  }

  /** 异步工厂方法 */
  static async create(): Promise<AnytypeAdapter> {
    const instance = new AnytypeAdapter();
    const creds = await loadCredentials();
    instance.baseUrl = creds.apiBaseUrl;
    instance.apiKey = creds.apiKey;
    instance.apiVersion = creds.apiVersion;
    instance.configured = !!creds.apiKey;
    return instance;
  }

  isConfigured(): boolean {
    return this.configured;
  }

  private get headers(): Record<string, string> {
    return {
      Authorization: `Bearer ${this.apiKey}`,
      "Anytype-Version": this.apiVersion,
      "Content-Type": "application/json",
    };
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    params?: Record<string, string | number>,
  ): Promise<T> {
    const url = new URL(path, this.baseUrl);
    if (params) {
      for (const [key, value] of Object.entries(params)) {
        url.searchParams.set(key, String(value));
      }
    }

    const opts: RequestInit = {
      method,
      headers: this.headers,
    };
    if (body !== undefined) {
      opts.body = JSON.stringify(body);
    }

    const res = await fetch(url.toString(), opts);

    if (!res.ok) {
      let errMsg: string;
      try {
        const errBody = JSON.parse(await res.text()) as { message?: string };
        errMsg = errBody.message || res.statusText;
      } catch {
        errMsg = res.statusText;
      }
      throw new AnytypeAdapterError(
        `${method} ${path}: ${res.status} — ${errMsg}`,
        res.status,
        String(res.status),
      );
    }

    // 204 No Content
    if (res.status === 204) return undefined as T;

    return res.json() as Promise<T>;
  }

  // ──────────────────────────────────────────
  // 公共 API
  // ──────────────────────────────────────────

  /** 健康检查: 尝试获取空间列表来验证 API 连通性 */
  async healthCheck(): Promise<AdapterStatus> {
    const start = Date.now();
    try {
      const res = await fetch(
        new URL("/v1/spaces", this.baseUrl).toString(),
        { method: "GET", headers: this.headers },
      );
      const ok = res.ok;
      return {
        connected: ok,
        lastCheck: new Date(),
        version: ok ? this.apiVersion : undefined,
      };
    } catch (e) {
      return {
        connected: false,
        lastCheck: new Date(),
        error: String(e),
      };
    }
  }

  /** 列出所有空间 */
  async listSpaces(): Promise<AnytypeSpace[]> {
    const data = await this.request<AnytypeListResponse<AnytypeSpace>>("GET", "/v1/spaces");
    return data.data || [];
  }

  /** 查询对象 — 使用 GET /v1/spaces/{spaceId}/objects (简单查询) 或 POST /v1/search (全文搜索) */
  async queryObjects(params: {
    type?: string;
    spaceId?: string;
    limit?: number;
    offset?: number;
    lastSync?: string;    // ISO 8601，增量同步用
    sort?: string;        // "lastModifiedDesc" | "createdDate" | "name"
  } = {}): Promise<AnytypeObject[]> {
    const searchBody: Record<string, unknown> = {};
    const sortFieldMap: Record<string, string> = {
      lastModifiedDesc: "last_modified_date",
      createdDate: "created_date",
      name: "name",
    };

    // 类型过滤
    if (params.type) {
      searchBody.types = [params.type.toLowerCase()];
    }

    // 分页
    searchBody.limit = params.limit ?? 50;
    if (params.offset !== undefined) searchBody.offset = params.offset;

    // 排序
    if (params.sort) {
      const field = sortFieldMap[params.sort] || "last_modified_date";
      searchBody.sort = { direction: "desc", property_key: field };
    }

    // 搜索端点
    const endpoint = params.spaceId
      ? `/v1/spaces/${params.spaceId}/search`
      : "/v1/search";

    try {
      const data = await this.request<AnytypeListResponse<AnytypeObject>>(
        "POST",
        endpoint,
        searchBody,
      );
      return data.data || [];
    } catch (e) {
      // 如果 spaces 内搜索失败，降级到全局搜索
      if (params.spaceId) {
        const data = await this.request<AnytypeListResponse<AnytypeObject>>(
          "POST",
          "/v1/search",
          searchBody,
        );
        return data.data || [];
      }
      throw e;
    }
  }

  /** 根据 ID 获取单个对象详情 */
  async getObject(spaceId: string, id: string): Promise<AnytypeObject> {
    const data = await this.request<{ object: AnytypeObject }>(
      "GET",
      `/v1/spaces/${encodeURIComponent(spaceId)}/objects/${encodeURIComponent(id)}`,
    );
    return data.object;
  }

  /**
   * Update object properties using the correct API format.
   *
   *  Examples:
   *    setProperties(spaceId, objId, [{ key: "status", select: "63454af7..." }])
   *    setProperties(spaceId, objId, [{ key: "done", checkbox: true }])
   */
  async setProperties(
    spaceId: string,
    id: string,
    props: Array<{ key: string; [k: string]: unknown }>,
  ): Promise<void> {
    await this.request(
      "PATCH",
      `/v1/spaces/${encodeURIComponent(spaceId)}/objects/${encodeURIComponent(id)}`,
      { properties: props },
    );
  }

  /** Convenience: set task status to DONE (uses status select property only) */
  async setTaskDone(spaceId: string, taskId: string): Promise<void> {
    await this.setProperties(spaceId, taskId, [
      { key: "status", select: "63454af7c493f68e301890dd" }, // DONE tag key
    ]);
  }

  /** Convenience: set task status to TODO (uses status select property only) */
  async setTaskTodo(spaceId: string, taskId: string): Promise<void> {
    await this.setProperties(spaceId, taskId, [
      { key: "status", select: "63454ad0c493f68e301890db" }, // TODO tag key
    ]);
  }

  /** 列出空间内的所有对象类型 */
  async listTypes(spaceId: string): Promise<{ id: string; key: string; name: string }[]> {
    const data = await this.request<{ data: { id: string; key: string; name: string }[] }>(
      "GET",
      `/v1/spaces/${encodeURIComponent(spaceId)}/types`,
      undefined,
      { limit: 100 },
    );
    return data.data || [];
  }
}

/** 单例工厂 */
let _instance: AnytypeAdapter | null = null;

export async function getAnytypeAdapter(): Promise<AnytypeAdapter> {
  if (!_instance) {
    _instance = await AnytypeAdapter.create();
  }
  return _instance;
}
