// Anytype REST API 客户端
// 基于 Anytype OpenAPI 规范（本地 /docs/openapi.json）
// API 路径结构: /v1/spaces/{space_id}/objects (对象必须在 space 下查询)

import { loadCredentials, hasApiKey } from "./auth.js";
import type { AnytypeObject, AnytypeSpace, AnytypeListResponse, AnytypeObjectResponse } from "./types.js";
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

  private async get<T>(path: string, params?: Record<string, string | number>): Promise<T> {
    const url = new URL(path, this.baseUrl);
    if (params) {
      for (const [key, value] of Object.entries(params)) {
        url.searchParams.set(key, String(value));
      }
    }

    const res = await fetch(url.toString(), {
      method: "GET",
      headers: this.headers,
    });

    if (!res.ok) {
      const body = await res.text().catch(() => "unknown");
      throw new AnytypeAdapterError(
        `GET ${path}: ${res.status} ${res.statusText} — ${body.slice(0, 200)}`,
        res.status,
        String(res.status),
      );
    }

    return res.json() as Promise<T>;
  }

  private async patch<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(new URL(path, this.baseUrl).toString(), {
      method: "PATCH",
      headers: this.headers,
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const text = await res.text().catch(() => "unknown");
      throw new AnytypeAdapterError(
        `PATCH ${path}: ${res.status} ${res.statusText} — ${text.slice(0, 200)}`,
        res.status,
        String(res.status),
      );
    }

    return res.json() as Promise<T>;
  }

  // ──────────────────────────────────────────
  // 公共 API
  // ──────────────────────────────────────────

  /** 健康检查 */
  async healthCheck(): Promise<AdapterStatus> {
    const start = Date.now();
    try {
      // Anytype API 在 31009 端口响应，但没有 /health 端点
      // 用 spaces 列表做健康检查（无认证返回 401，有认证返回数据）
      const res = await fetch(new URL("/v1/spaces", this.baseUrl).toString(), {
        method: "GET",
        headers: this.headers,
      });
      const ok = res.status < 400;
      return {
        connected: res.status !== 0,
        lastCheck: new Date(),
        version: ok ? String(res.status) : undefined,
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
    const data = await this.get<{ data: AnytypeSpace[] }>("/v1/spaces");
    return data.data || [];
  }

  /** 查询对象列表 — 必须在 space 下查 */
  async queryObjects(params: {
    type?: string;
    spaceId?: string;
    limit?: number;
    offset?: number;
    lastSync?: string; // ISO 8601
    sort?: string;
  } = {}): Promise<AnytypeObject[]> {
    if (!params.spaceId) {
      throw new AnytypeAdapterError("queryObjects: spaceId is required", 400, "missing_space");
    }

    const queryParams: Record<string, string | number> = {};

    if (params.type) queryParams.type = params.type;
    if (params.limit !== undefined) queryParams.limit = params.limit;
    if (params.offset !== undefined) queryParams.offset = params.offset;
    if (params.sort) queryParams.sort = params.sort;
    if (params.lastSync) {
      queryParams.lastModifiedAfter = params.lastSync;
    }

    const data = await this.get<{ data: AnytypeObject[] }>(
      `/v1/spaces/${encodeURIComponent(params.spaceId)}/objects`,
      queryParams,
    );

    return data.data || [];
  }

  /** 根据 ID 获取单个对象 */
  async getObject(spaceId: string, id: string): Promise<AnytypeObject> {
    const data = await this.get<{ object: AnytypeObject }>(
      `/v1/spaces/${encodeURIComponent(spaceId)}/objects/${encodeURIComponent(id)}`,
    );
    return data.object;
  }

  /** 更新对象字段 */
  async updateObject(spaceId: string, id: string, fields: Partial<AnytypeObject>): Promise<void> {
    await this.patch(`/v1/spaces/${encodeURIComponent(spaceId)}/objects/${encodeURIComponent(id)}`, fields);
  }

  /** 列出空间内的类型 */
  async listTypes(spaceId?: string): Promise<{ id: string; name: string }[]> {
    if (!spaceId) {
      throw new AnytypeAdapterError("listTypes: spaceId is required", 400, "missing_space");
    }
    const data = await this.get<{ types: { id: string; name: string; name?: string }[] }>(
      `/v1/spaces/${encodeURIComponent(spaceId)}/types`,
    );
    return data.types || [];
  }

  /** 搜索（全局） */
  async search(query: string, limit: number = 10): Promise<AnytypeObject[]> {
    const data = await this.get<{ objects: AnytypeObject[] }>("/v1/search", { query, limit });
    return data.objects || [];
  }
}

/** 单例工厂 */
let _instance: AnytypeAdapter | null = null;

export function getAnytypeAdapter(): AnytypeAdapter {
  if (!_instance) {
    _instance = new AnytypeAdapter();
  }
  return _instance;
}