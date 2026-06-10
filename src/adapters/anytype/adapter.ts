// Anytype REST API 客户端
// 基于 Anytype OpenAPI v2025-11-08 规范的轻量 HTTP 客户端

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

    constructor() {
        const creds = loadCredentials();
        this.baseUrl = creds.apiBaseUrl;
        this.apiKey = creds.apiKey;
        this.apiVersion = creds.apiVersion;
    }

    /** 检查是否有可用的 API Key */
    isConfigured(): boolean {
        return hasApiKey();
    }

    /** 获取认证用的请求头 */
    private get headers(): Record<string, string> {
        return {
            Authorization: `Bearer ${this.apiKey}`,
            "Anytype-Version": this.apiVersion,
            "Content-Type": "application/json",
        };
    }

    /** HTTP GET 请求 */
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

    /** HTTP PATCH 请求（用于更新对象） */
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
            const res = await fetch(new URL("/api/v1/health", this.baseUrl).toString(), {
                method: "GET",
                headers: this.headers,
            });
            const ok = res.status < 400;
            return {
                connected: ok,
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
        const data = await this.get<{ spaces: AnytypeSpace[] }>("/api/v1/spaces");
        return data.spaces || [];
    }

    /** 查询对象列表 */
    async queryObjects(params: {
        type?: string;
        spaceId?: string;
        limit?: number;
        offset?: number;
        lastSync?: string;    // ISO 8601，增量同步用
        sort?: string;        // "lastModifiedDesc" | "createdDate" | "name"
    } = {}): Promise<AnytypeObject[]> {
        const queryParams: Record<string, string | number> = {};

        if (params.type) queryParams.type = params.type;
        if (params.spaceId) queryParams.spaceId = params.spaceId;
        if (params.limit !== undefined) queryParams.limit = params.limit;
        if (params.offset !== undefined) queryParams.offset = params.offset;
        if (params.sort) queryParams.sort = params.sort;
        if (params.lastSync) {
            // Anytype API 支持 lastModifiedAfter 参数进行增量查询
            queryParams.lastModifiedAfter = params.lastSync;
        }

        const data = await this.get<AnytypeListResponse<AnytypeObject>>(
            "/api/v1/objects",
            queryParams,
        );

        return data.objects || [];
    }

    /** 根据 ID 获取单个对象 */
    async getObject(id: string): Promise<AnytypeObject> {
        const data = await this.get<AnytypeObjectResponse>(`/api/v1/objects/${encodeURIComponent(id)}`);
        return data.object;
    }

    /** 更新对象字段 */
    async updateObject(id: string, fields: Partial<AnytypeObject>): Promise<void> {
        await this.patch(`/api/v1/objects/${encodeURIComponent(id)}`, fields);
    }

    /** 列出对象的所有类型 */
    async listTypes(): Promise<{ id: string; name: string }[]> {
        const data = await this.get<{ types: { id: string; name: string }[] }>("/api/v1/types");
        return data.types || [];
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