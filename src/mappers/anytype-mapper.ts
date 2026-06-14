// Anytype Task ↔ Canonical Task 映射器
//
// AnytypeObject (properties[] array) ──→ CanonicalTask (规范层)
// CanonicalTask ──→ PATCH request (写回 Anytype)

import type { AnytypeObject } from "../adapters/anytype/types.js";
import {
  getProperty,
  isCompletedByStatus,
  getStatusName,
  getStatusTagKey,
  getDeadline,
  getDescription,
  getTags,
  getCanonicalPriority,
  getScheduleDate,
  getCompletionDate,
  isArchived,
  isDone,
  getPriorityTagKey,
  canonicalToAnytypePriority,
  getRecurrence,
} from "../adapters/anytype/types.js";
import { STATUS_TAGS } from "../adapters/anytype/types.js";
import type { CanonicalTask } from "../store/schema.js";
import { createTask } from "../store/schema.js";

// ──────────────────────────────────────────
// Anytype → Canonical
// ──────────────────────────────────────────

export function anytypeToCanonical(obj: AnytypeObject): CanonicalTask {
  const statusName = getStatusName(obj);
  const statusTagKey = getStatusTagKey(obj);
  const tags = getTags(obj);
  const priority = getCanonicalPriority(obj);

  return createTask({
    name: obj.name || "(未命名任务)",
    description: getDescription(obj) || "",
    due_date: getDeadline(obj) || null,
    is_completed: isCompletedByStatus(obj) ? 1 : 0,
    priority,
    completion_date: getCompletionDate(obj) || null,
    is_archived: isArchived(obj) ? 1 : 0,

    status_select: statusName || null,
    status_tag_key: statusTagKey || null,
    done_checkbox: isDone(obj) ? 1 : 0,
    tags: tags.length > 0 ? JSON.stringify(tags) : null,
    schedule_date: getScheduleDate(obj) || null,
    recurrence: getRecurrence(obj) || null,

    source: "anytype",
    anytype_id: obj.id,
    space_id: obj.space_id,

    raw_json: JSON.stringify(obj),
  });
}

// ──────────────────────────────────────────
// Canonical → Anytype (PATCH properties)
// ──────────────────────────────────────────

export interface AnytypePatchOp {
  key: string;
  [format: string]: unknown;
}

/**
 * 将 CanonicalTask 的变更转为 Anytype PATCH properties 列表。
 * 只返回有变更的字段。
 *
 * @param task 当前 canonical 状态
 * @param refObj 引用对象（可选，用于比对只推送变更）
 */
export function canonicalToAnytypePatch(
  task: CanonicalTask,
  refObj?: AnytypeObject,
): AnytypePatchOp[] {
  const props: AnytypePatchOp[] = [];

  // name
  if (task.name && (!refObj || refObj.name !== task.name)) {
    props.push({ key: "name", text: task.name });
  }

  // description (只在有内容时写)
  if (task.description && (!refObj || getDescription(refObj) !== task.description)) {
    props.push({ key: "description", text: task.description });
  }

  // due_date
  if (task.due_date !== undefined) {
    const current = refObj ? getDeadline(refObj) || null : null;
    if (!refObj || current !== task.due_date) {
      props.push({ key: "due_date", date: task.due_date });
    }
  }

  // status — compare with refObj before pushing
  const currentStatusName = refObj ? getStatusName(refObj) : null;
  const currentTagKey = refObj ? getStatusTagKey(refObj) : null;
  const currentDone = refObj ? isCompletedByStatus(refObj) : null;

  if (!refObj) {
    // No ref: always push
    if (task.is_completed) {
      props.push({ key: "status", select: STATUS_TAGS.DONE });
    } else if (task.status_select && task.status_tag_key) {
      props.push({ key: "status", select: task.status_tag_key });
    } else {
      props.push({ key: "status", select: STATUS_TAGS.TODO });
    }
  } else if (task.is_completed && !currentDone) {
    // Need to mark DONE
    props.push({ key: "status", select: STATUS_TAGS.DONE });
  } else if (!task.is_completed && currentDone) {
    // Need to unmark DONE — keep the status tag if available
    if (task.status_select && task.status_tag_key && currentStatusName !== task.status_select) {
      props.push({ key: "status", select: task.status_tag_key });
    } else {
      props.push({ key: "status", select: STATUS_TAGS.TODO });
    }
  } else if (!task.is_completed && task.status_select && task.status_tag_key) {
    // Both not-completed: check if status need to change
    if (currentStatusName !== task.status_select) {
      props.push({ key: "status", select: task.status_tag_key });
    }
  }

  // done checkbox (同步设置保持兼容)
  if (task.done_checkbox !== null && task.done_checkbox !== undefined) {
    const refChecked = refObj ? isDone(refObj) : null;
    if (refChecked === null || refChecked !== (task.is_completed === 1)) {
      props.push({ key: "done", checkbox: task.is_completed === 1 });
    }
  }

  // priority (四象限值) — 只推送已有 priority 的对象，避免推不存在的选项
  if (task.priority !== undefined && task.priority >= 0) {
    const refPriority = refObj ? getCanonicalPriority(refObj) : null;
    if (refPriority !== null && refPriority !== 0 && refPriority !== task.priority) {
      const anytypeVal = canonicalToAnytypePriority(task.priority);
      props.push({ key: "priority", select: anytypeVal });
    }
  }

  // completion_date
  if (task.completion_date) {
    const refDate = refObj ? getCompletionDate(refObj) : null;
    if (!refDate || refDate !== task.completion_date) {
      props.push({ key: "completion_date", date: task.completion_date });
    }
  }

  // recurrence
  if (task.recurrence) {
    const refRecurrence = refObj ? getRecurrence(refObj) || null : null;
    if (!refObj || refRecurrence !== task.recurrence) {
      props.push({ key: "recurrence", text: task.recurrence });
    }
  } else if (refObj && getRecurrence(refObj)) {
    // Recurrence was removed — clear it
    props.push({ key: "recurrence", text: "" });
  }

  return props;
}

// ──────────────────────────────────────────
// 哈希 (用于变更检测)
// ──────────────────────────────────────────

/**
 * 计算 Anytype task 的内容哈希，用于跳过无变更同步。
 * 与 canonical 模型对齐: 使用 canonical 关心的字段。
 */
export function computeAnytypeContentHash(obj: AnytypeObject): string {
  const fields = [
    obj.name,
    String(isCompletedByStatus(obj)),
    getDeadline(obj) || "",
    getCanonicalPriority(obj).toString(),
    getDescription(obj) || "",
    getStatusName(obj) || "",
    getCompletionDate(obj) || "",
    getRecurrence(obj) || "",
  ];
  const str = fields.join("|");
  let hash = 5381;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) + hash) + str.charCodeAt(i);
    hash = hash & hash;
  }
  return Math.abs(hash).toString(36);
}
