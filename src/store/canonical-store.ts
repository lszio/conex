// Canonical Store — 统一任务存储
// 建立新 tasks 表，保留旧表（id_map/sync_state/conflict_log/config）用于过渡
// 数据库位置: ~/.conex/sync.db

import { Database, type SQLQueryBindings } from "bun:sqlite";
import { existsSync, mkdirSync } from "fs";
import { homedir } from "os";
import { join } from "path";
import type { CanonicalTask } from "./schema.js";
import { CREATE_TASKS_TABLE, CREATE_TASKS_INDEXES } from "./schema.js";

const CONEX_DIR = join(homedir(), ".conex");
const DB_PATH = join(CONEX_DIR, "sync.db");

// ── 条件构建 ──
interface WhereClause {
  sql: string;
  values: SQLQueryBindings[];
}

function buildWhere(conditions: Partial<CanonicalTask>): WhereClause {
  const parts: string[] = [];
  const values: SQLQueryBindings[] = [];

  for (const [key, value] of Object.entries(conditions)) {
    if (value === undefined || value === null) continue;
    // Map camelCase → snake_case for SQL columns
    const col = key.replace(/[A-Z]/g, (c) => "_" + c.toLowerCase());
    parts.push(`${col} = ?`);
    values.push(value);
  }

  return {
    sql: parts.length > 0 ? `WHERE ${parts.join(" AND ")}` : "",
    values,
  };
}

const TASK_COLS = [
  "id", "name", "description", "due_date", "is_completed", "priority",
  "completion_date", "is_archived", "alarm_date", "recurrence", "status_select", "status_tag_key",
  "done_checkbox", "tags", "schedule_date", "source", "anytype_id",
  "apple_id", "space_id", "created_at", "updated_at", "version",
  "last_synced", "raw_json",
];

const TASK_COLS_LIST = TASK_COLS.join(", ");
const TASK_PLACEHOLDERS = TASK_COLS.map(() => "?").join(", ");
const TASK_UPDATE_SET = TASK_COLS
  .filter((c) => c !== "id")
  .map((c) => `${c} = ?`)
  .join(", ");

function taskToRow(t: CanonicalTask): SQLQueryBindings[] {
  return TASK_COLS.map((col) => {
    // CanonicalTask uses snake_case field names matching SQL column names
    const val = (t as any)[col];
    return val ?? null;
  });
}

// ──────────────────────────────────────────
// Canonical Store
// ──────────────────────────────────────────

export class CanonicalStore {
  private db: Database;

  constructor() {
    if (!existsSync(CONEX_DIR)) {
      mkdirSync(CONEX_DIR, { recursive: true });
    }

    this.db = new Database(DB_PATH);
    this.db.run("PRAGMA journal_mode=WAL");
    this.migrate();
  }

  private migrate(): void {
    this.db.run(CREATE_TASKS_TABLE);
    for (const idx of CREATE_TASKS_INDEXES) {
      this.db.run(idx);
    }
    // Add new columns for existing databases (safe: no-op if already present)
    try { this.db.run(`ALTER TABLE tasks ADD COLUMN recurrence TEXT`); } catch (_) {}
  }

  close(): void {
    this.db.close();
  }

  // ──────────────────────────────────────
  // Tasks CRUD
  // ──────────────────────────────────────

  /** Insert a new canonical task */
  insert(task: CanonicalTask): void {
    this.db
      .prepare(`INSERT INTO tasks (${TASK_COLS_LIST}) VALUES (${TASK_PLACEHOLDERS})`)
      .run(...taskToRow(task));
  }

  /** Insert or replace */
  upsert(task: CanonicalTask): void {
    const row = taskToRow(task);
    // UPSERT needs: INSERT bindings (23) + UPDATE bindings (22, excluding id)
    const updateRow = row.filter((_, i) => TASK_COLS[i] !== "id");
    this.db
      .prepare(`
        INSERT INTO tasks (${TASK_COLS_LIST})
        VALUES (${TASK_PLACEHOLDERS})
        ON CONFLICT(id) DO UPDATE SET
          ${TASK_UPDATE_SET},
          version = excluded.version
      `)
      .run(...row, ...updateRow);
  }

  /** Get by canonical ID */
  get(id: string): CanonicalTask | null {
    const row = this.db
      .prepare(`SELECT * FROM tasks WHERE id = ?`)
      .get(id) as Record<string, unknown> | undefined;
    return row ? rowToTask(row) : null;
  }

  /** Get by anytype_id */
  getByAnytypeId(anytypeId: string): CanonicalTask | null {
    const row = this.db
      .prepare(`SELECT * FROM tasks WHERE anytype_id = ?`)
      .get(anytypeId) as Record<string, unknown> | undefined;
    return row ? rowToTask(row) : null;
  }

  /** Get by apple_id */
  getByAppleId(appleId: string): CanonicalTask | null {
    const row = this.db
      .prepare(`SELECT * FROM tasks WHERE apple_id = ?`)
      .get(appleId) as Record<string, unknown> | undefined;
    return row ? rowToTask(row) : null;
  }

  /** List all tasks with optional filtering */
  list(params?: {
    is_completed?: number;
    priority?: number;
    source?: string;
    limit?: number;
    offset?: number;
    orderBy?: string;
    orderDir?: "ASC" | "DESC";
  }): CanonicalTask[] {
    const where: string[] = [];
    const values: SQLQueryBindings[] = [];

    if (params?.is_completed !== undefined) {
      where.push("is_completed = ?");
      values.push(params.is_completed);
    }
    if (params?.priority !== undefined) {
      where.push("priority = ?");
      values.push(params.priority);
    }
    if (params?.source) {
      where.push("source = ?");
      values.push(params.source);
    }

    const whereClause = where.length > 0 ? `WHERE ${where.join(" AND ")}` : "";
    const orderBy = params?.orderBy
      ? `ORDER BY ${params.orderBy} ${params.orderDir || "DESC"}`
      : "ORDER BY updated_at DESC";
    const limit = params?.limit ?? 100;
    const offset = params?.offset ?? 0;

    const rows = this.db
      .prepare(`SELECT * FROM tasks ${whereClause} ${orderBy} LIMIT ? OFFSET ?`)
      .all(...values, limit, offset) as Record<string, unknown>[];

    return rows.map(rowToTask);
  }

  /** Update specific fields by canonical ID */
  update(id: string, fields: Partial<CanonicalTask>): void {
    const parts: string[] = [];
    const values: SQLQueryBindings[] = [];

    if (fields.name !== undefined) { parts.push("name = ?"); values.push(fields.name); }
    if (fields.description !== undefined) { parts.push("description = ?"); values.push(fields.description); }
    if (fields.due_date !== undefined) { parts.push("due_date = ?"); values.push(fields.due_date); }
    if (fields.is_completed !== undefined) { parts.push("is_completed = ?"); values.push(fields.is_completed); }
    if (fields.priority !== undefined) { parts.push("priority = ?"); values.push(fields.priority); }
    if (fields.completion_date !== undefined) { parts.push("completion_date = ?"); values.push(fields.completion_date); }
    if (fields.is_archived !== undefined) { parts.push("is_archived = ?"); values.push(fields.is_archived); }
    if (fields.alarm_date !== undefined) { parts.push("alarm_date = ?"); values.push(fields.alarm_date); }
    if (fields.recurrence !== undefined) { parts.push("recurrence = ?"); values.push(fields.recurrence); }
    if (fields.status_select !== undefined) { parts.push("status_select = ?"); values.push(fields.status_select); }
    if (fields.status_tag_key !== undefined) { parts.push("status_tag_key = ?"); values.push(fields.status_tag_key); }
    if (fields.done_checkbox !== undefined) { parts.push("done_checkbox = ?"); values.push(fields.done_checkbox); }
    if (fields.tags !== undefined) { parts.push("tags = ?"); values.push(fields.tags); }
    if (fields.schedule_date !== undefined) { parts.push("schedule_date = ?"); values.push(fields.schedule_date); }
    if (fields.source !== undefined) { parts.push("source = ?"); values.push(fields.source); }
    if (fields.anytype_id !== undefined) { parts.push("anytype_id = ?"); values.push(fields.anytype_id); }
    if (fields.apple_id !== undefined) { parts.push("apple_id = ?"); values.push(fields.apple_id); }
    if (fields.space_id !== undefined) { parts.push("space_id = ?"); values.push(fields.space_id); }
    if (fields.last_synced !== undefined) { parts.push("last_synced = ?"); values.push(fields.last_synced); }
    if (fields.raw_json !== undefined) { parts.push("raw_json = ?"); values.push(fields.raw_json); }

    // Always bump version and updated_at
    parts.push("version = version + 1");
    parts.push("updated_at = ?");
    values.push(new Date().toISOString());

    // last_synced is only updated if explicitly passed in fields, or auto-set to now if not provided
    if (fields.last_synced === undefined) {
      parts.push("last_synced = ?");
      values.push(new Date().toISOString());
    }

    this.db
      .prepare(`UPDATE tasks SET ${parts.join(", ")} WHERE id = ?`)
      .run(...values, id);
  }

  /** Delete by canonical ID */
  delete(id: string): void {
    this.db.prepare("DELETE FROM tasks WHERE id = ?").run(id);
  }

  /** Count tasks */
  count(): number {
    const row = this.db.prepare("SELECT COUNT(*) as count FROM tasks").get() as { count: number };
    return row.count;
  }

  /** List tasks updated since a given timestamp */
  listUpdatedSince(since: string, limit: number = 200): CanonicalTask[] {
    const rows = this.db
      .prepare("SELECT * FROM tasks WHERE updated_at > ? ORDER BY updated_at DESC LIMIT ?")
      .all(since, limit) as Record<string, unknown>[];
    return rows.map(rowToTask);
  }
}

// ── DB row → CanonicalTask ──
function rowToTask(row: Record<string, unknown>): CanonicalTask {
  return {
    id: row.id as string,
    name: row.name as string,
    description: row.description as string,
    due_date: row.due_date as string | null,
    is_completed: row.is_completed as number,
    priority: row.priority as number,
    completion_date: row.completion_date as string | null,
    is_archived: row.is_archived as number,
    alarm_date: row.alarm_date as string | null,
    recurrence: row.recurrence as string | null,
    status_select: row.status_select as string | null,
    status_tag_key: row.status_tag_key as string | null,
    done_checkbox: row.done_checkbox as number | null,
    tags: row.tags as string | null,
    schedule_date: row.schedule_date as string | null,
    source: row.source as string,
    anytype_id: row.anytype_id as string | null,
    apple_id: row.apple_id as string | null,
    space_id: row.space_id as string | null,
    created_at: row.created_at as string,
    updated_at: row.updated_at as string,
    version: row.version as number,
    last_synced: row.last_synced as string | null,
    raw_json: row.raw_json as string | null,
  };
}

// ── 单例 ──
let _store: CanonicalStore | null = null;

export function getCanonicalStore(): CanonicalStore {
  if (!_store) {
    _store = new CanonicalStore();
  }
  return _store;
}
