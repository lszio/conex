// Anytype API 类型定义 — 基于实际 API v2025-11-08 响应格式
// 属性以数组形式返回，每个元素包含 format/值字段。

export interface AnytypeProperty {
  object: "property";
  id: string;
  key: string;           // e.g. "done", "due_date", "priority", "status", "tag"
  name: string;          // e.g. "Done", "Due date", "Priority"
  format: string;        // "checkbox" | "date" | "select" | "multi_select" | "text" | "objects" | ...

  // checkbox 格式
  checkbox?: boolean;

  // date 格式
  date?: string;         // ISO 8601

  // select 格式
  select?: {
    object: "tag";
    id: string;
    key: string;
    name: string;
    color: string;
  } | null;

  // multi_select 格式
  multi_select?: Array<{
    object: "tag";
    id: string;
    key: string;
    name: string;
    color: string;
  }>;

  // objects 格式 (relations)
  objects?: string[] | null;

  // text 格式
  text?: string;

  // number 格式
  number?: number;
}

export interface AnytypeType {
  object: "type";
  id: string;
  key: string;           // e.g. "task", "page", "note"
  name: string;          // e.g. "Task", "Page", "Note"
  plural_name: string;
  layout: string;
  icon?: { format: string; name: string; color?: string };
  properties?: AnytypeProperty[];
}

export interface AnytypeObject {
  object: "object";
  id: string;
  name: string;
  icon: any | null;
  archived: boolean;
  space_id: string;
  snippet?: string;      // 简短内容预览（搜索中的内容片段）
  layout: string;        // e.g. "basic", "note", "set", "collection"
  type: AnytypeType | string;  // 完整对象或类型 key 字符串
  properties?: AnytypeProperty[];

  // 获取属性值的便捷方法
  getProp?: (key: string) => AnytypeProperty | undefined;
}

/** 从 properties 数组中获取指定 key 的属性 */
export function getProperty(obj: AnytypeObject, key: string): AnytypeProperty | undefined {
  if (!obj.properties) return undefined;
  return obj.properties.find(p => p.key === key);
}

// ──────────────────────────────────────────
// 完成状态
// ──────────────────────────────────────────

/** 获取 done checkbox 值 */
export function isDone(obj: AnytypeObject): boolean {
  return getProperty(obj, "done")?.checkbox || false;
}

/** 获取状态 tag name */
export function getStatusName(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "status")?.select?.name || undefined;
}

/** 获取状态 tag key (用于更新 select 属性) */
export function getStatusTagKey(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "status")?.select?.key || undefined;
}

/** Status tag key 常量 (labry 空间) */
export const STATUS_TAGS = {
  TODO: "63454ad0c493f68e301890db",
  DONE: "63454af7c493f68e301890dd",
  WILL: "will",
  PEND: "pend",
  QUIT: "quit",
} as const;

/**
 * 判断任务是否完成。
 *
 * 逻辑: status == "DONE" 优先级最高；
 * 如果没有 status 字段或 status 未设置，fallback 到 done checkbox。
 */
export function isCompletedByStatus(obj: AnytypeObject): boolean {
  const status = getStatusName(obj);
  if (status) return status === "DONE";
  // fallback: done checkbox
  return isDone(obj);
}

// ──────────────────────────────────────────
// 优先级 (四象限)
// ──────────────────────────────────────────

/** 优先级的四象限 select option name（Anytype 中对应值） */
export const PRIORITY_OPTIONS = ["0", "1", "2", "3"] as const;

/**
 * 从 Anytype priority select 获取原始值 → canonical 0-3
 * Anytype select option name 直接存 "0","1","2","3"
 */
export function getCanonicalPriority(obj: AnytypeObject): number {
  const name = getProperty(obj, "priority")?.select?.name;
  if (name === "3") return 3;
  if (name === "2") return 2;
  if (name === "1") return 1;
  return 0;
}

/**
 * Canonical priority (0-3) → Anytype priority select value (string)
 */
export function canonicalToAnytypePriority(p: number): string {
  if (p >= 3) return "3";
  if (p >= 2) return "2";
  if (p >= 1) return "1";
  return "0";
}

// ──────────────────────────────────────────
// 日期
// ──────────────────────────────────────────

/** 获取截止日期 */
export function getDeadline(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "due_date")?.date || undefined;
}

/** 获取 schedule 日期 */
export function getScheduleDate(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "schedule")?.date || undefined;
}

/** 获取 completion_date (由 CONEX 写入的新字段) */
export function getCompletionDate(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "completion_date")?.date || undefined;
}

// ──────────────────────────────────────────
// 描述内容
// ──────────────────────────────────────────

/**
 * 获取任务描述。
 * 优先级: description text 属性 > snippet
 */
export function getDescription(obj: AnytypeObject): string | undefined {
  const descProp = getProperty(obj, "description");
  if (descProp?.text) return descProp.text;
  return obj.snippet || undefined;
}

// ──────────────────────────────────────────
// 标签、类型等
// ──────────────────────────────────────────

/** 获取标签列表 */
export function getTags(obj: AnytypeObject): string[] {
  return (getProperty(obj, "tag")?.multi_select || []).map(t => t.name);
}

/** 获取类型 key (小写) */
export function getTypeKey(obj: AnytypeObject): string {
  if (typeof obj.type === "string") return obj.type.toLowerCase();
  return obj.type?.key || "unknown";
}

/** 获取 archive checkbox */
export function isArchived(obj: AnytypeObject): boolean {
  return getProperty(obj, "archive")?.checkbox || obj.archived;
}

/** 获取周期规则（JSON 文本字段） */
export function getRecurrence(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "recurrence")?.text || undefined;
}

/** 获取优先级的 tag name (原始 Anytype select name) */
export function getPriorityName(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "priority")?.select?.name || undefined;
}

/** 获取优先级的 tag key */
export function getPriorityTagKey(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "priority")?.select?.key || undefined;
}

// ──────────────────────────────────────────
// API 类型
// ──────────────────────────────────────────

export interface AnytypeSpace {
  object: string;
  id: string;
  name: string;
  icon: any | null;
  description: string;
  gateway_url: string;
  network_id: string;
}

export interface AnytypePagination {
  total: number;
  offset: number;
  limit: number;
  has_more: boolean;
}

export interface AnytypeListResponse<T> {
  data: T[];
  pagination?: AnytypePagination;
}

export interface AnytypeErrorResponse {
  object: "error";
  status: number;
  code: string;
  message: string;
}
