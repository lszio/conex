// CONEX Canonical Store — 统一任务模型
//
// 所有适配器读写这个模型。Anytype ↔ Canonical ↔ Apple 两条映射路径。
// 优先级编码: 0=none, 1=Q2(重要不紧急), 2=Q3(紧急不重要), 3=Q1(紧急重要)
//   bit1=紧急, bit0=重要
//    00=Q4(0)  01=Q2(1)  10=Q3(2)  11=Q1(3)

export interface CanonicalTask {
  id: string;                // UUID v4

  // ── 核心字段 ──
  name: string;
  description: string;
  due_date: string | null;   // ISO 8601 datetime
  is_completed: number;      // 0 | 1
  priority: number;          // 0 | 1 | 2 | 3 （四象限）
  completion_date: string | null; // ISO 8601
  is_archived: number;       // 0 | 1
  alarm_date: string | null; // ISO 8601 — Apple Reminder 闹钟时间

  // ── 周期任务 ──
  recurrence: string | null; // JSON: {frequency, interval, days_of_week?, end_date?, ...}
                             // 详见 https://developer.apple.com/documentation/eventkit/ekrecurrencerule

  // ── Anytype 原始字段（合并进来，不另开表）──
  status_select: string | null;   // 'TODO' | 'DONE' | 'WILL' | 'PEND' | 'QUIT'
  status_tag_key: string | null;  // 用于 PATCH 的原始 tag key
  done_checkbox: number | null;   // Anytype done checkbox 原始值 0|1|null
  tags: string | null;            // JSON array: '["Work","BSPA"]'
  schedule_date: string | null;   // Anytype schedule 字段

  // ── 源追踪 ──
  source: string;            // 'anytype' | 'apple' | 'manual'
  anytype_id: string | null;
  apple_id: string | null;
  space_id: string | null;

  // ── 系统字段 ──
  created_at: string;        // ISO 8601
  updated_at: string;        // ISO 8601
  version: number;           // 乐观锁
  last_synced: string | null;

  // ── 完整 round-trip ──
  raw_json: string | null;   // Anytype 对象完整 JSON
}

// ──────────────────────────────────────────
// 优先级映射函数
// ──────────────────────────────────────────

const QUADRANT_LABELS = ["Q4", "Q2", "Q3", "Q1"] as const;

/** Canonical 值 → 四象限标签 */
export function priorityToQuadrantLabel(p: number): string {
  return QUADRANT_LABELS[p] ?? "Q4";
}

/** 四象限标签 → Canonical 值 */
export function quadrantLabelToPriority(label: string): number {
  const idx = QUADRANT_LABELS.indexOf(label as typeof QUADRANT_LABELS[number]);
  return idx >= 0 ? idx : 0;
}

/**
 * Canonical priority (0-3) → Apple Reminders priority (0/1/5/9)
 *   Canonical 0 (Q4) → Apple 0 (none)
 *   Canonical 1 (Q2) → Apple 9 (low)
 *   Canonical 2 (Q3) → Apple 5 (medium)
 *   Canonical 3 (Q1) → Apple 1 (high)
 */
export function canonicalToApplePriority(p: number): number {
  if (p >= 3) return 1;
  if (p >= 2) return 5;
  if (p >= 1) return 9;
  return 0;
}

/**
 * Apple Reminders priority (0/1/5/9) → Canonical priority (0-3)
 */
export function appleToCanonicalPriority(ap: number): number {
  if (ap <= 0) return 0;
  if (ap >= 9) return 1;  // low → Q2
  if (ap >= 5) return 2;  // medium → Q3
  return 3;                // high → Q1
}

// ──────────────────────────────────────────
// 优先级四象限名（用于 Anytype select options）
// ──────────────────────────────────────────
export const PRIORITY_OPTIONS = [
  { name: "Q4", key: "q4" },
  { name: "Q2", key: "q2" },
  { name: "Q3", key: "q3" },
  { name: "Q1", key: "q1" },
] as const;

// ──────────────────────────────────────────
// 状态常量
// ──────────────────────────────────────────

export const STATUS_VALUES = ["TODO", "DONE", "WILL", "PEND", "QUIT"] as const;

export type StatusValue = typeof STATUS_VALUES[number];

// ──────────────────────────────────────────
// SQL Schema（用于 migration）
// ──────────────────────────────────────────

export const CREATE_TASKS_TABLE = `
  CREATE TABLE IF NOT EXISTS tasks (
    id              TEXT PRIMARY KEY,

    -- 核心字段
    name            TEXT NOT NULL DEFAULT '',
    description     TEXT NOT NULL DEFAULT '',
    due_date        TEXT,
    is_completed    INTEGER NOT NULL DEFAULT 0,
    priority        INTEGER NOT NULL DEFAULT 0,
    completion_date TEXT,
    is_archived     INTEGER NOT NULL DEFAULT 0,
    alarm_date      TEXT,
    recurrence      TEXT,

    -- Anytype 原始字段
    status_select   TEXT,
    status_tag_key  TEXT,
    done_checkbox   INTEGER,
    tags            TEXT,           -- JSON array
    schedule_date   TEXT,

    -- 源追踪
    source          TEXT NOT NULL DEFAULT 'manual',
    anytype_id      TEXT,
    apple_id        TEXT,
    space_id        TEXT,

    -- 系统字段
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    version         INTEGER NOT NULL DEFAULT 1,
    last_synced     TEXT,

    raw_json        TEXT
  )
`;

export const CREATE_TASKS_INDEXES = [
  `CREATE INDEX IF NOT EXISTS idx_tasks_anytype_id ON tasks(anytype_id)`,
  `CREATE INDEX IF NOT EXISTS idx_tasks_apple_id ON tasks(apple_id)`,
  `CREATE INDEX IF NOT EXISTS idx_tasks_is_completed ON tasks(is_completed)`,
  `CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority)`,
  `CREATE INDEX IF NOT EXISTS idx_tasks_updated_at ON tasks(updated_at)`,
];

// 旧表保留用于迁移期间兼容
export const LEGACY_TABLES = ["id_map", "sync_state", "conflict_log", "config"];

// ──────────────────────────────────────────
// 规范层 default 工厂
// ──────────────────────────────────────────

export function createTask(overrides?: Partial<CanonicalTask>): CanonicalTask {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    name: "",
    description: "",
    due_date: null,
    is_completed: 0,
    priority: 0,
    completion_date: null,
    is_archived: 0,
    alarm_date: null,
    recurrence: null,
    status_select: null,
    status_tag_key: null,
    done_checkbox: null,
    tags: null,
    schedule_date: null,
    source: "manual",
    anytype_id: null,
    apple_id: null,
    space_id: null,
    created_at: now,
    updated_at: now,
    version: 1,
    last_synced: null,
    raw_json: null,
    ...overrides,
  };
}
