// SQLite 状态存储 — 管理同步状态、ID 映射、冲突日志和配置
// 数据库位置: ~/.conex/sync.db (WAL 模式)

import { Database, type SQLQueryBindings } from "bun:sqlite";
import { existsSync, mkdirSync } from "fs";
import { homedir } from "os";
import { join } from "path";

const CONEX_DIR = join(homedir(), ".conex");
const DB_PATH = join(CONEX_DIR, "sync.db");

// ──────────────────────────────────────────
// 行类型定义
// ──────────────────────────────────────────

export interface IdMapRow {
    anytype_id: string;
    apple_id: string;
    apple_type: string;        // 'reminder' | 'event' | 'note'
    last_sync_at: string;      // ISO 8601
    last_hash: string | null;  // 上一次同步的内容哈希
    direction: string;         // 'forward' | 'bidirectional'
}

export interface SyncStateRow {
    id: number;
    last_poll_at: string;      // ISO 8601
    total_synced: number;
    last_error: string | null;
    config_hash: string | null;
}

export interface ConflictLogRow {
    id: number;
    anytype_id: string;
    apple_id: string;
    conflict_type: string;     // 'content_mismatch' | 'deleted_on_one_side'
    resolution: string | null; // 'use_anytype' | 'use_apple' | 'manual'
    resolved_at: string | null;
    created_at: string;
}

export interface ConfigRow {
    key: string;
    value: string;
}

// ──────────────────────────────────────────
// 数据库管理器
// ──────────────────────────────────────────

export class SyncStore {
    private db: Database;

    constructor() {
        if (!existsSync(CONEX_DIR)) {
            mkdirSync(CONEX_DIR, { recursive: true });
        }

        this.db = new Database(DB_PATH);

        // 启用 WAL 模式
        this.db.run("PRAGMA journal_mode=WAL");

        // 初始化表结构
        this.migrate();
    }

    /** Schema 迁移 */
    private migrate(): void {
        this.db.run(`
            CREATE TABLE IF NOT EXISTS id_map (
                anytype_id    TEXT PRIMARY KEY,
                apple_id      TEXT NOT NULL,
                apple_type    TEXT NOT NULL,
                last_sync_at  TEXT NOT NULL,
                last_hash     TEXT,
                direction     TEXT NOT NULL DEFAULT 'forward'
            )
        `);

        this.db.run(`
            CREATE TABLE IF NOT EXISTS sync_state (
                id            INTEGER PRIMARY KEY DEFAULT 1,
                last_poll_at  TEXT NOT NULL,
                total_synced  INTEGER DEFAULT 0,
                last_error    TEXT,
                config_hash   TEXT
            )
        `);

        this.db.run(`
            CREATE TABLE IF NOT EXISTS conflict_log (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                anytype_id    TEXT NOT NULL,
                apple_id      TEXT NOT NULL,
                conflict_type TEXT NOT NULL,
                resolution    TEXT,
                resolved_at   TEXT,
                created_at    TEXT NOT NULL DEFAULT (datetime('now'))
            )
        `);

        this.db.run(`
            CREATE TABLE IF NOT EXISTS config (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )
        `);

        // 确保 sync_state 至少有一行
        const existing = this.db
            .prepare("SELECT id FROM sync_state WHERE id = 1")
            .get() as { id: number } | undefined;
        if (!existing) {
            this.db
                .prepare("INSERT INTO sync_state (id, last_poll_at, total_synced) VALUES (1, datetime('now'), 0)")
                .run();
        }
    }

    /** 关闭数据库连接 */
    close(): void {
        this.db.close();
    }

    // ──────────────────────────────────────
    // ID 映射
    // ──────────────────────────────────────

    /** 获取 ID 映射 */
    getIdMap(anytypeId: string): IdMapRow | null {
        const row = this.db
            .prepare("SELECT * FROM id_map WHERE anytype_id = ?")
            .get(anytypeId) as IdMapRow | undefined;
        return row || null;
    }

    /** 按 Apple ID 查找映射 */
    findByAppleId(appleId: string): IdMapRow | null {
        const row = this.db
            .prepare("SELECT * FROM id_map WHERE apple_id = ?")
            .get(appleId) as IdMapRow | undefined;
        return row || null;
    }

    /** 获取所有类型为某值的映射 */
    listIdMaps(appleType?: string): IdMapRow[] {
        if (appleType) {
            return this.db
                .prepare("SELECT * FROM id_map WHERE apple_type = ?")
                .all(appleType) as IdMapRow[];
        }
        return this.db.prepare("SELECT * FROM id_map").all() as IdMapRow[];
    }

    /** 创建或更新 ID 映射 */
    upsertIdMap(row: Omit<IdMapRow, "last_sync_at"> & { last_sync_at?: string }): void {
        this.db
            .prepare(`
                INSERT INTO id_map (anytype_id, apple_id, apple_type, last_sync_at, last_hash, direction)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(anytype_id) DO UPDATE SET
                    apple_id = excluded.apple_id,
                    apple_type = excluded.apple_type,
                    last_sync_at = excluded.last_sync_at,
                    last_hash = excluded.last_hash,
                    direction = excluded.direction
            `)
            .run(
                row.anytype_id,
                row.apple_id,
                row.apple_type,
                row.last_sync_at || new Date().toISOString(),
                row.last_hash || null,
                row.direction || "forward",
            );
    }

    /** 删除 ID 映射 */
    deleteIdMap(anytypeId: string): void {
        this.db.prepare("DELETE FROM id_map WHERE anytype_id = ?").run(anytypeId);
    }

    /** 获取映射数量 */
    countIdMaps(): number {
        const row = this.db
            .prepare("SELECT COUNT(*) as count FROM id_map")
            .get() as { count: number };
        return row.count;
    }

    // ──────────────────────────────────────
    // 同步状态
    // ──────────────────────────────────────

    /** 获取同步状态 */
    getSyncState(): SyncStateRow {
        return this.db
            .prepare("SELECT * FROM sync_state WHERE id = 1")
            .get() as SyncStateRow;
    }

    /** 更新同步状态 */
    updateSyncState(updates: Partial<Pick<SyncStateRow, "last_poll_at" | "total_synced" | "last_error" | "config_hash">>): void {
        const parts: string[] = [];
        const values: SQLQueryBindings[] = [];

        if (updates.last_poll_at !== undefined) {
            parts.push("last_poll_at = ?");
            values.push(updates.last_poll_at);
        }
        if (updates.total_synced !== undefined) {
            parts.push("total_synced = ?");
            values.push(updates.total_synced);
        }
        if (updates.last_error !== undefined) {
            parts.push("last_error = ?");
            values.push(updates.last_error);
        }
        if (updates.config_hash !== undefined) {
            parts.push("config_hash = ?");
            values.push(updates.config_hash);
        }

        if (parts.length > 0) {
            this.db
                .prepare(`UPDATE sync_state SET ${parts.join(", ")} WHERE id = 1`)
                .run(...values);
        }
    }

    // ──────────────────────────────────────
    // 冲突日志
    // ──────────────────────────────────────

    /** 记录冲突 */
    logConflict(conflict: {
        anytype_id: string;
        apple_id: string;
        conflict_type: string;
    }): void {
        this.db
            .prepare("INSERT INTO conflict_log (anytype_id, apple_id, conflict_type) VALUES (?, ?, ?)")
            .run(conflict.anytype_id, conflict.apple_id, conflict.conflict_type);
    }

    /** 获取冲突历史 */
    listConflicts(limit: number = 50): ConflictLogRow[] {
        return this.db
            .prepare("SELECT * FROM conflict_log ORDER BY created_at DESC LIMIT ?")
            .all(limit) as ConflictLogRow[];
    }

    /** 标记冲突已解决 */
    resolveConflict(conflictId: number, resolution: string): void {
        this.db
            .prepare("UPDATE conflict_log SET resolution = ?, resolved_at = datetime('now') WHERE id = ?")
            .run(resolution, conflictId);
    }

    // ──────────────────────────────────────
    // 配置
    // ──────────────────────────────────────

    /** 获取配置值 */
    getConfig(key: string): string | null {
        const row = this.db
            .prepare("SELECT value FROM config WHERE key = ?")
            .get(key) as { value: string } | undefined;
        return row?.value || null;
    }

    /** 设置配置值 */
    setConfig(key: string, value: string): void {
        this.db
            .prepare("INSERT OR REPLACE INTO config (key, value) VALUES (?, ?)")
            .run(key, value);
    }

    /** 列出所有配置 */
    listConfig(): ConfigRow[] {
        return this.db.prepare("SELECT * FROM config ORDER BY key").all() as ConfigRow[];
    }

    /** 删除配置 */
    deleteConfig(key: string): void {
        this.db.prepare("DELETE FROM config WHERE key = ?").run(key);
    }
}

/** 单例 */
let _store: SyncStore | null = null;

export function getSyncStore(): SyncStore {
    if (!_store) {
        _store = new SyncStore();
    }
    return _store;
}