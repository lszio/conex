// Anytype API 类型定义 — 基于实际 API v2025-11-08 响应格式
//
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

/** 获取 done 状态 */
export function isDone(obj: AnytypeObject): boolean {
  return getProperty(obj, "done")?.checkbox || false;
}

/** 获取截止日期 */
export function getDeadline(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "due_date")?.date || undefined;
}

/** 获取优先级的 tag name */
export function getPriorityName(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "priority")?.select?.name || undefined;
}

/** 获取状态 tag name */
export function getStatusName(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "status")?.select?.name || undefined;
}

/** 获取状态 tag key (用于更新 select 属性) */
export function getStatusTagKey(obj: AnytypeObject): string | undefined {
  return getProperty(obj, "status")?.select?.key || undefined;
}

/** Status tag key 常量 */
export const STATUS_TAGS = {
  TODO: "63454ad0c493f68e301890db",
  DONE: "63454af7c493f68e301890dd",
  WILL: "will",
  PEND: "pend",
  QUIT: "quit",
} as const;

/**
 * 根据 status select 属性判断任务是否完成。
 * 只有 status == DONE 才算完成。
 * 如果没有 status 属性，视为 TODO（未完成）。
 */
export function isCompletedByStatus(obj: AnytypeObject): boolean {
  const status = getStatusName(obj);
  return status === "DONE";
}

/** 获取标签列表 */
export function getTags(obj: AnytypeObject): string[] {
  return (getProperty(obj, "tag")?.multi_select || []).map(t => t.name);
}

/** 获取类型 key (小写) */
export function getTypeKey(obj: AnytypeObject): string {
  if (typeof obj.type === "string") return obj.type.toLowerCase();
  return obj.type?.key || "unknown";
}

/** 获取描述内容 (优先 snippet，其次 body) */
export function getDescription(obj: AnytypeObject): string | undefined {
  return obj.snippet || undefined;
}

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
