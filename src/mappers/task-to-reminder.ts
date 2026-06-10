// Task (Anytype) → Reminder (Apple Reminders) 映射器
// 定义 Anytype 对象与 Apple Reminders 之间的字段映射规则

import type { AnytypeObject } from "../adapters/anytype/types.js";
import type { ReminderData } from "../adapters/apple/types.js";
import type { SyncMapper, Conflict } from "../adapters/types.js";
import { mapperRegistry } from "./registry.js";

export type SyncMode = "forward" | "backward" | "bidirectional";

/**
 * Anytype 的 priority 字段 (0-4) 映射到 Apple Reminders 的 priority (0/1/5/9)。
 *
 * Anytype: 0=none, 1=low, 2=medium, 3=high, 4=urgent
 * Apple:  0=none, 1=high, 5=medium, 9=low
 */
function anytypePriorityToApple(ap: number | undefined): number {
    if (ap === undefined || ap <= 0) return 0;
    if (ap === 1) return 9;  // low
    if (ap === 2) return 5;  // medium
    if (ap === 3) return 1;  // high
    if (ap >= 4) return 1;   // urgent → high
    return 0;
}

function applePriorityToAnytype(ap: number | undefined): number {
    if (ap === undefined || ap === 0) return 0;
    if (ap >= 9) return 1;   // low
    if (ap >= 5) return 2;   // medium
    if (ap >= 1) return 3;   // high
    return 0;
}

/**
 * 从 Anytype 对象备注中提取 conexId
 */
export function extractConexIdFromNotes(notes: string | undefined): string | null {
    if (!notes) return null;
    const match = notes.match(/\[conex:([^\]]+)\]/);
    return match ? match[1] : null;
}

/**
 * 计算内容哈希，用于冲突检测
 */
export function computeContentHash(task: AnytypeObject): string {
    const fields = [
        task.name,
        task.description,
        task.done ? "done" : "pending",
        task.deadline || "",
        String(task.priority ?? 0),
    ];
    // 简单的字符串哈希 (djb2)
    const str = fields.join("|");
    let hash = 5381;
    for (let i = 0; i < str.length; i++) {
        hash = ((hash << 5) + hash) + str.charCodeAt(i);
        hash = hash & hash; // Convert to 32bit integer
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
        const result: ReminderData = {
            title: source.name || "(未命名任务)",
            notes: source.description || undefined,
            isCompleted: !!source.done,
            priority: anytypePriorityToApple(source.priority),
            conexId: source.id,
        };

        // 处理截止日期
        if (source.deadline) {
            const d = new Date(source.deadline);
            if (!isNaN(d.getTime())) {
                result.dueDate = d;
            }
        }

        // 将任何已有的 conexId 信息拼接到备注中
        if (source.description) {
            const tag = `[conex:${source.id}]`;
            if (!source.description.includes(tag)) {
                result.notes = source.description + `\n${tag}`;
            }
        } else {
            result.notes = `[conex:${source.id}]`;
        }

        return result;
    }

    toSource(target: ReminderData): AnytypeObject {
        const conexId = target.conexId || extractConexIdFromNotes(target.notes) || "";

        const obj: AnytypeObject = {
            id: conexId,
            name: target.title,
            type: "Task",
            description: target.notes,
            done: target.isCompleted || false,
            priority: applePriorityToAnytype(target.priority),
            lastModifiedDate: new Date().toISOString(),
            createdDate: new Date().toISOString(),
        };

        if (target.dueDate) {
            obj.deadline = target.dueDate.toISOString();
        }

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

        // 完成状态
        if (!!source.done !== !!target.isCompleted) {
            conflicts.push({
                field: "isCompleted",
                sourceValue: !!source.done,
                targetValue: !!target.isCompleted,
                severity: "info",
            });
        }

        // 日期
        const sourceDate = source.deadline ? new Date(source.deadline).toISOString().slice(0, 10) : null;
        const targetDate = target.dueDate ? target.dueDate.toISOString().slice(0, 10) : null;
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