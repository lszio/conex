// Apple Reminder ↔ Canonical Task 映射器
//
// ReminderData (AppleScript) ──→ CanonicalTask (规范层)
// CanonicalTask ──→ ReminderData (写回 Apple)
//
// Apple Reminder notes 结构:
//   {纯用户描述}
//
//   ─── CONEX ───
//   ID: {anytype_id}
//   URI: anytype://{anytype_id}
//   Status: TODO | DONE | ...
//   Priority: Q1 | Q2 | Q3 | Q4
//   Tags: {tag1}, {tag2}
//   Schedule: {schedule_date}
//   Recurrence: {frequency} ...
//   Space: {space_id}
//
// 元数据段由 "─── CONEX ───" 分隔，读回时完整剥离。
// 纯用户描述部分才被写回 Anytype 的 description 字段。

import type { ReminderData } from "../adapters/apple/types.js";
import type { CanonicalTask } from "../store/schema.js";
import { createTask, canonicalToApplePriority, appleToCanonicalPriority, priorityToQuadrantLabel } from "../store/schema.js";

// ──────────────────────────────────────────
// Apple 备注中的元数据段常量
// ──────────────────────────────────────────

const METADATA_SEPARATOR = "─── CONEX ───";

/**
 * 从 Apple 备注中提取 conexId。
 * 支持新旧两种格式:
 *   新: ID: bafyreidyy...
 *   旧: [conex:bafyreidyy...]
 */
export function extractConexIdFromNotes(notes: string | undefined): string | null {
  if (!notes) return null;

  // 新格式: "ID: bafyreidyy..."
  const newMatch = notes.match(new RegExp(`^ID:\\s*(\\S+)`, "m"));
  if (newMatch) return newMatch[1];

  // 旧格式兼容: "[conex:bafyreidyy...]"
  const oldMatch = notes.match(/\[conex:([^\]]+)\]/);
  if (oldMatch) return oldMatch[1];

  return null;
}

// ──────────────────────────────────────────
// 构建 Apple 备注（含元数据段）
// ──────────────────────────────────────────

/** 格式化 recurrrence JSON 为人类可读字符串 */
function formatRecurrence(recJson: string | null): string | null {
  if (!recJson) return null;
  try {
    const r = JSON.parse(recJson);
    const parts: string[] = [];
    parts.push(r.frequency);
    if (r.interval > 1) parts.unshift(`every ${r.interval}`);
    if (r.days_of_week) {
      const dayNames = ["", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
      parts.push(`(${r.days_of_week.map((d: number) => dayNames[d] || d).join(", ")})`);
    }
    if (r.end_date) parts.push(`until ${r.end_date.slice(0, 10)}`);
    if (r.occurrence_count) parts.push(`×${r.occurrence_count}`);
    return parts.join(" ");
  } catch {
    return recJson;
  }
}

/** 解析 tags JSON */
function formatTags(tags: string | null): string | null {
  if (!tags) return null;
  try {
    const t = JSON.parse(tags);
    return Array.isArray(t) && t.length > 0 ? t.join(", ") : null;
  } catch {
    return tags;
  }
}

/**
 * 构建包含元数据段的 Apple 备注字符串。
 * 纯用户描述在上，元数据段在 "─── CONEX ───" 分隔线之后。
 * 读回时完整剥离元数据段，只保留纯描述。
 */
export function buildNotes(task: CanonicalTask): string {
  const lines: string[] = [];

  // 用户描述（无任何 CONEX 元数据）
  if (task.description) {
    lines.push(task.description);
  }

  // 元数据段（Apple 侧专属展示，不会被回写到 Anytype）
  if (task.anytype_id || task.status_select || task.priority || task.tags || task.schedule_date || task.recurrence || task.space_id) {
    if (lines.length > 0 && lines[lines.length - 1] !== "") {
      lines.push(""); // 空行分隔
    }
    lines.push(METADATA_SEPARATOR);

    if (task.anytype_id) {
      lines.push(`ID: ${task.anytype_id}`);
      lines.push(`URI: anytype://${task.anytype_id}`);
    }
    if (task.status_select) {
      lines.push(`Status: ${task.status_select}`);
    }
    if (task.priority && task.priority > 0) {
      lines.push(`Priority: ${priorityToQuadrantLabel(task.priority)}`);
    }
    if (task.schedule_date) {
      lines.push(`Schedule: ${task.schedule_date.slice(0, 10)}`);
    }
    const formattedRec = formatRecurrence(task.recurrence);
    if (formattedRec) {
      lines.push(`Recurrence: ${formattedRec}`);
    }
    if (task.space_id) {
      lines.push(`Space: ${task.space_id.slice(0, 16)}…`);
    }

    // ── 标签行（Apple 原生 tag 系统识别 #tagname ──
    const tagWords: string[] = [];
    tagWords.push("conex"); // 标记为 CONEX 管理
    const parsedTags = formatTags(task.tags);
    if (parsedTags) {
      for (const t of parsedTags.split(", ")) {
        const clean = t.trim().replace(/\s+/g, "-").toLowerCase();
        if (clean) tagWords.push(clean);
      }
    }
    if (tagWords.length > 0) {
      lines.push(""); // 空行
      lines.push(tagWords.map(t => `#${t}`).join(" "));
    }
  }

  return lines.join("\n");
}

// ──────────────────────────────────────────
// Apple Reminder → Canonical
// ──────────────────────────────────────────

export function reminderToCanonical(reminder: ReminderData): CanonicalTask {
  const conexId = reminder.conexId || extractConexIdFromNotes(reminder.notes);

  // 从备注中剥离完整元数据段，得到纯用户内容
  const cleanNotes = stripMetadataFromNotes(reminder.notes);

  return createTask({
    name: reminder.title || "",
    description: cleanNotes,
    due_date: reminder.dueDate ? reminder.dueDate.toISOString() : null,
    is_completed: reminder.isCompleted ? 1 : 0,
    priority: appleToCanonicalPriority(reminder.priority ?? 0),
    alarm_date: reminder.alarmDate ? reminder.alarmDate.toISOString() : null,
    recurrence: reminder.recurrence || null,
    completion_date: null, // AppleScript 无法可靠读取 completion date
    is_archived: 0,
    source: "apple",
    apple_id: reminder.id || null,
    anytype_id: conexId,
  });
}

/**
 * 从 Apple 备注中剥离完整元数据段。
 *
 * 剥离规则:
 * 1. "─── CONEX ───" 分隔行及之后的所有内容
 * 2. 旧的独立行格式: anytype://... 和 [conex:...]
 *
 * 只返回纯用户描述文本。
 */
function stripMetadataFromNotes(notes: string | undefined): string {
  if (!notes) return "";

  // 新格式: 分隔符之后整段剥离
  const sepIdx = notes.indexOf(METADATA_SEPARATOR);
  if (sepIdx >= 0) {
    return notes.slice(0, sepIdx).trim();
  }

  // 旧格式兼容: 逐行过滤
  const lines = notes.split("\n");
  const filtered = lines.filter((line) => {
    if (line.startsWith("anytype://")) return false;
    if (line.startsWith("[conex:") && line.endsWith("]")) return false;
    return true;
  });
  return filtered.join("\n").trim();
}

// ──────────────────────────────────────────
// Canonical → Apple Reminder
// ──────────────────────────────────────────

export function canonicalToReminder(task: CanonicalTask): ReminderData {
  const result: ReminderData = {
    title: task.name || "(未命名任务)",
    notes: buildNotes(task),
    isCompleted: task.is_completed === 1,
    priority: canonicalToApplePriority(task.priority),
    conexId: task.anytype_id || undefined,
  };

  if (task.due_date) {
    const d = new Date(task.due_date);
    if (!isNaN(d.getTime())) {
      result.dueDate = d;
    }
  }

  // 紧急任务（Q1/Q3）设置闹钟提醒
  const computedAlarm = computeAlarmDate(task);
  if (computedAlarm) {
    result.alarmDate = computedAlarm;
  }

  // 周期任务
  if (task.recurrence) {
    result.recurrence = task.recurrence;
  }

  return result;
}

/** 计算任务应该有的闹钟时间（紧急任务 = 截止时间前 15 分钟） */
export function computeAlarmDate(task: CanonicalTask): Date | null {
  if (task.priority < 2) return null;
  if (task.alarm_date) {
    const d = new Date(task.alarm_date);
    return isNaN(d.getTime()) ? null : d;
  }
  if (task.due_date) {
    const d = new Date(task.due_date);
    if (!isNaN(d.getTime())) {
      d.setMinutes(d.getMinutes() - 15);
      return d;
    }
  }
  return null;
}

// ──────────────────────────────────────────
// 变更检测 (用于 Canonical → Apple push)
// ──────────────────────────────────────────

export function hasReminderChanged(
  task: CanonicalTask,
  currentReminder: ReminderData,
): boolean {
  if (task.name !== currentReminder.title) return true;
  if (task.description !== stripMetadataFromNotes(currentReminder.notes)) return true;
  if ((task.is_completed === 1) !== !!currentReminder.isCompleted) return true;
  if (canonicalToApplePriority(task.priority) !== (currentReminder.priority ?? 0)) return true;

  // 日期比较
  const taskDate = task.due_date ? new Date(task.due_date).toISOString().slice(0, 10) : null;
  const remDate = currentReminder.dueDate ? currentReminder.dueDate.toISOString().slice(0, 10) : null;
  if (taskDate !== remDate) return true;

  // 闹钟比较: 计算 task 应该有的闹钟，和当前提醒的闹钟比对
  const computedAlarm = computeAlarmDate(task);
  const taskAlarm = computedAlarm ? computedAlarm.toISOString().slice(0, 16) : null;
  const remAlarm = currentReminder.alarmDate ? currentReminder.alarmDate.toISOString().slice(0, 16) : null;
  if (taskAlarm !== remAlarm) return true;

  // 周期规则比较
  if ((task.recurrence || null) !== (currentReminder.recurrence || null)) return true;

  // 备注元数据格式升级: 旧格式 (anytype://...) → 新格式 (─── CONEX ───)
  if (currentReminder.notes) {
    const hasOldFormat = currentReminder.notes.includes("anytype://") || currentReminder.notes.includes("[conex:");
    const hasNewFormat = currentReminder.notes.includes(METADATA_SEPARATOR);
    if (hasOldFormat && !hasNewFormat) return true;
  }

  return false;
}
