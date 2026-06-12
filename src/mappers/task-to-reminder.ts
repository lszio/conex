// Task (Anytype) → Reminder (Apple Reminders) 映射器
// 定义 Anytype 对象与 Apple Reminders 之间的字段映射规则
//
// Anytype Task 属性 (properties 数组):
//   done       — checkbox: boolean
//   due_date   — date: "ISO 8601"
//   priority   — select: { name: "High" | "Medium" | "Low" | ... }
//   status     — select: { name: "TODO" | "DONE" | "WILL" | ... }
//   tag        — multi_select: [{ name: "..." }]
//   assignee   — objects: [participant_id]
//   description— 在 object.snippet 中

import type { AnytypeObject } from "../adapters/anytype/types.js";
import {
  isDone,
  getDeadline,
  getPriorityName,
  getDescription,
  getStatusName,
  isCompletedByStatus,
  getStatusTagKey,
} from "../adapters/anytype/types.js";
import type { ReminderData } from "../adapters/apple/types.js";
import type { SyncMapper, Conflict } from "../adapters/types.js";
import { mapperRegistry } from "./registry.js";

export type SyncMode = "forward" | "backward" | "bidirectional";

/** Anytype iOS 深度链接前缀 */
export const ANYTYPE_DEEP_LINK_PREFIX = "anytype://";

function makeDeepLink(objectId: string): string {
  return `${ANYTYPE_DEEP_LINK_PREFIX}${objectId}`;
}

/**
 * Anytype priority (tag name) → Apple Reminders priority (0/1/5/9)。
 *
 * Anytype: "High" / "Medium" / "Low" / "Urgent"
 * Apple:  0=none, 1=high, 5=medium, 9=low
 */
function anytypePriorityToApple(priorityName: string | undefined): number {
  if (!priorityName) return 0;
  const name = priorityName.toLowerCase();
  if (name === "urgent" || name === "high") return 1;
  if (name === "medium") return 5;
  if (name === "low") return 9;
  return 0;
}

function applePriorityToAnytype(ap: number | undefined): string {
  if (ap === undefined || ap === 0) return "";
  if (ap >= 9) return "Low";
  if (ap >= 5) return "Medium";
  if (ap >= 1) return "High";
  return "";
}

/**
 * 从 Apple Reminder notes 中提取 conexId
 */
export function extractConexIdFromNotes(notes: string | undefined): string | null {
  if (!notes) return null;
  const match = notes.match(/\[conex:([^\]]+)\]/);
  return match ? match[1] : null;
}

/**
 * 计算内容哈希，用于冲突检测
 * 使用 name + completion + due_date + priority + description + status
 */
export function computeContentHash(task: AnytypeObject): string {
  const fields = [
    task.name,
    String(isCompletedByStatus(task)),
    getDeadline(task) || "",
    getPriorityName(task) || "",
    getDescription(task) || "",
    getStatusName(task) || "",
  ];
  const str = fields.join("|");
  let hash = 5381;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) + hash) + str.charCodeAt(i);
    hash = hash & hash;
  }
  return Math.abs(hash).toString(36);
}

export class TaskToReminderMapper implements SyncMapper<AnytypeObject, ReminderData> {
  sourceType = "Task";
  targetType = "Reminder";
  direction: SyncMode;

  constructor(direction: SyncMode = "forward") {
    this.direction = direction;
  }

  toTarget(source: AnytypeObject): ReminderData {
    // 使用 status 属性判断完成状态
    const completed = isCompletedByStatus(source);
    const statusName = getStatusName(source);

    // 构建备注: 描述 + 状态信息 + 深度链接 + conexId
    const notesParts: string[] = [];
    const desc = getDescription(source);
    if (desc) notesParts.push(desc);

    // 状态信息
    if (statusName && statusName !== "TODO") {
      notesParts.push(`📌 ${statusName}`);
    }

    // Anytype 深度链接
    notesParts.push(makeDeepLink(source.id));

    // CONEX 追踪 ID
    notesParts.push(`[conex:${source.id}]`);

    const result: ReminderData = {
      title: source.name || "(未命名任务)",
      notes: notesParts.join("\n"),
      isCompleted: completed,
      priority: anytypePriorityToApple(getPriorityName(source)),
      conexId: source.id,
    };

    // 截止日期
    const deadline = getDeadline(source);
    if (deadline) {
      const d = new Date(deadline);
      if (!isNaN(d.getTime())) {
        result.dueDate = d;
      }
    }

    return result;
  }

  toSource(target: ReminderData): AnytypeObject {
    const conexId = target.conexId || extractConexIdFromNotes(target.notes) || "";

    const obj: AnytypeObject = {
      object: "object",
      id: conexId,
      name: target.title,
      icon: null,
      layout: "basic",
      type: "task",
      space_id: "",
      archived: false,
      snippet: target.notes || undefined,
      properties: [],
    };

    return obj;
  }

  detectConflict(source: AnytypeObject, target: ReminderData): Conflict[] {
    const conflicts: Conflict[] = [];

    // 标题对比
    if (source.name !== target.title) {
      conflicts.push({
        field: "title",
        sourceValue: source.name,
        targetValue: target.title,
        severity: "warning",
      });
    }

    // 完成状态 (使用 status 判断)
    if (isCompletedByStatus(source) !== !!target.isCompleted) {
      conflicts.push({
        field: "isCompleted",
        sourceValue: isCompletedByStatus(source),
        targetValue: !!target.isCompleted,
        severity: "info",
      });
    }

    // 日期
    const sourceDate = getDeadline(source)
      ? new Date(getDeadline(source)!).toISOString().slice(0, 10)
      : null;
    const targetDate = target.dueDate
      ? target.dueDate.toISOString().slice(0, 10)
      : null;
    if (sourceDate !== targetDate) {
      conflicts.push({
        field: "dueDate",
        sourceValue: sourceDate || "(none)",
        targetValue: targetDate || "(none)",
        severity: "info",
      });
    }

    return conflicts;
  }
}

// 注册到全局注册表
mapperRegistry.register(new TaskToReminderMapper());
