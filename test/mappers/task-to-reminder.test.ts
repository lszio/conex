// Task → Reminder 映射器单元测试

import { expect, describe, test } from "bun:test";
import { TaskToReminderMapper, computeContentHash, extractConexIdFromNotes } from "../../src/mappers/task-to-reminder.js";
import type { AnytypeObject } from "../../src/adapters/anytype/types.js";
import type { ReminderData } from "../../src/adapters/apple/types.js";

describe("TaskToReminderMapper", () => {
    const mapper = new TaskToReminderMapper("forward");

    test("toTarget: 基本映射 — 标题、描述、完成状态", () => {
        const task: AnytypeObject = {
            id: "tx123",
            name: "买牛奶",
            type: "Task",
            description: "记得买全脂牛奶",
            done: false,
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const reminder = mapper.toTarget(task);

        expect(reminder.title).toBe("买牛奶");
        expect(reminder.notes).toContain("记得买全脂牛奶");
        expect(reminder.isCompleted).toBe(false);
        expect(reminder.conexId).toBe("tx123");
    });

    test("toTarget: 截止日期映射", () => {
        const task: AnytypeObject = {
            id: "tx456",
            name: "提交报告",
            type: "Task",
            deadline: "2026-06-15T17:00:00Z",
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const reminder = mapper.toTarget(task);

        expect(reminder.dueDate).toBeInstanceOf(Date);
        expect(reminder.dueDate!.toISOString()).toContain("2026-06-15");
    });

    test("toTarget: 优先级映射 (Anytype 3 → Apple 1)", () => {
        const task: AnytypeObject = {
            id: "tx789",
            name: "紧急任务",
            type: "Task",
            priority: 3,
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const reminder = mapper.toTarget(task);
        expect(reminder.priority).toBe(1); // high
    });

    test("toTarget: 优先级映射 (Anytype 1 → Apple 9)", () => {
        const task: AnytypeObject = {
            id: "tx012",
            name: "低优先级",
            type: "Task",
            priority: 1,
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const reminder = mapper.toTarget(task);
        expect(reminder.priority).toBe(9); // low
    });

    test("toTarget: 备注中写入 conexId 标签", () => {
        const task: AnytypeObject = {
            id: "tx999",
            name: "追踪测试",
            type: "Task",
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const reminder = mapper.toTarget(task);
        expect(reminder.notes).toContain("[conex:tx999]");
    });

    test("toSource: 基本逆向映射", () => {
        const reminder: ReminderData = {
            title: "买牛奶",
            notes: "记得买全脂牛奶\n[conex:tx123]",
            isCompleted: true,
            priority: 1,
            conexId: "tx123",
        };

        const task = mapper.toSource(reminder);

        expect(task.name).toBe("买牛奶");
        expect(task.done).toBe(true);
        expect(task.priority).toBe(3); // Apple 1 → Anytype 3
    });

    test("toSource: 从备注中提取 conexId", () => {
        const reminder: ReminderData = {
            title: "测试",
            notes: "Some text\n[conex:tx555]",
        };

        const conexId = extractConexIdFromNotes(reminder.notes);
        expect(conexId).toBe("tx555");
    });

    test("computeContentHash: 相同内容产生相同哈希", () => {
        const a: AnytypeObject = {
            id: "t1",
            name: "任务",
            type: "Task",
            done: false,
            priority: 2,
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const b: AnytypeObject = {
            id: "t2", // ID 不同
            name: "任务",
            type: "Task",
            done: false,
            priority: 2,
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        expect(computeContentHash(a)).toBe(computeContentHash(b));
    });

    test("computeContentHash: 不同内容产生不同哈希", () => {
        const a: AnytypeObject = {
            id: "t1",
            name: "任务A",
            type: "Task",
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const b: AnytypeObject = {
            id: "t2",
            name: "任务B",
            type: "Task",
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        expect(computeContentHash(a)).not.toBe(computeContentHash(b));
    });

    test("detectConflict: 内容一致时无冲突", () => {
        const a: AnytypeObject = {
            id: "t1",
            name: "任务",
            type: "Task",
            done: false,
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const b: ReminderData = {
            title: "任务",
            isCompleted: false,
        };

        expect(mapper.detectConflict(a, b)).toHaveLength(0);
    });

    test("detectConflict: 标题不一致时报告冲突", () => {
        const a: AnytypeObject = {
            id: "t1",
            name: "任务A",
            type: "Task",
            lastModifiedDate: "2026-06-10T10:00:00Z",
            createdDate: "2026-06-09T10:00:00Z",
        };

        const b: ReminderData = {
            title: "任务B",
        };

        const conflicts = mapper.detectConflict(a, b);
        expect(conflicts.some((c: { field: string }) => c.field === "title")).toBe(true);
    });
});