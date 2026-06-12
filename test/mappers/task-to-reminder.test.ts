// Task → Reminder 映射器单元测试 (基于新版 types)
import { expect, describe, test } from "bun:test";
import { TaskToReminderMapper, computeContentHash, extractConexIdFromNotes } from "../../src/mappers/task-to-reminder.js";
import type { AnytypeObject } from "../../src/adapters/anytype/types.js";
import { getProperty } from "../../src/adapters/anytype/types.js";
import type { ReminderData } from "../../src/adapters/apple/types.js";

/** 创建测试 AnytypeObject 的便捷函数 */
function makeTask(overrides: Partial<AnytypeObject> & {
  done?: boolean;
  deadline?: string;
  priorityName?: string;
  description?: string;
}): AnytypeObject {
  const task: AnytypeObject = {
    object: "object",
    id: overrides.id || "tx123",
    name: overrides.name || "买牛奶",
    layout: "basic",
    type: "task",
    space_id: "space1",
    archived: false,
    snippet: overrides.description,
    properties: [],
  };

  // 添加 done 属性
  if (overrides.done !== undefined) {
    task.properties!.push({
      object: "property",
      id: "prop-done",
      key: "done",
      name: "Done",
      format: "checkbox",
      checkbox: overrides.done,
    });
  }

  // 添加 due_date 属性
  if (overrides.deadline) {
    task.properties!.push({
      object: "property",
      id: "prop-deadline",
      key: "due_date",
      name: "Due date",
      format: "date",
      date: overrides.deadline,
    });
  }

  // 添加 priority 属性
  if (overrides.priorityName) {
    task.properties!.push({
      object: "property",
      id: "prop-priority",
      key: "priority",
      name: "Priority",
      format: "select",
      select: {
        object: "tag",
        id: "tag-pri",
        key: "pri-key",
        name: overrides.priorityName,
        color: "red",
      },
    });
  }

  // 添加 status 属性
  if (overrides.status) {
    task.properties!.push({
      object: "property",
      id: "prop-status",
      key: "status",
      name: "Status",
      format: "select",
      select: {
        object: "tag",
        id: "tag-st",
        key: "st-key",
        name: overrides.status,
        color: "ice",
      },
    });
  }

  return task;
}

describe("TaskToReminderMapper", () => {
  const mapper = new TaskToReminderMapper("forward");

  test("toTarget: status=DONE 映射为已完成的提醒", () => {
    const task = makeTask({
      id: "tx-done",
      name: "已完成任务",
      done: false,         // done checkbox 是 false
      status: "DONE",      // 但 status 是 DONE
    });

    const reminder = mapper.toTarget(task);

    // 即使 done=false，因为 status=DONE，isCompleted=true
    expect(reminder.isCompleted).toBe(true);
    expect(reminder.notes).toContain("anytype://tx-done");
    expect(reminder.notes).toContain("[conex:tx-done]");
  });

  test("toTarget: status=TODO 映射为未完成的提醒", () => {
    const task = makeTask({
      id: "tx-todo",
      name: "待办任务",
      status: "TODO",
    });

    const reminder = mapper.toTarget(task);

    expect(reminder.isCompleted).toBe(false);
    // 状态信息 TODO 不显示（默认状态）
    expect(reminder.notes).not.toContain("📌");
  });

  test("toTarget: status=WILL 显示状态标签", () => {
    const task = makeTask({
      id: "tx-will",
      name: "计划任务",
      status: "WILL",
    });

    const reminder = mapper.toTarget(task);

    expect(reminder.isCompleted).toBe(false);
    expect(reminder.notes).toContain("📌 WILL");
  });

  test("toTarget: 截止日期映射", () => {
    const task = makeTask({
      id: "tx456",
      name: "提交报告",
      deadline: "2026-06-15T17:00:00Z",
    });

    const reminder = mapper.toTarget(task);

    expect(reminder.dueDate).toBeInstanceOf(Date);
    expect(reminder.dueDate!.toISOString()).toContain("2026-06-15");
  });

  test("toTarget: 优先级映射 (Anytype High → Apple 1)", () => {
    const task = makeTask({
      id: "tx789",
      name: "紧急任务",
      priorityName: "High",
    });

    const reminder = mapper.toTarget(task);
    expect(reminder.priority).toBe(1); // high
  });

  test("toTarget: 优先级映射 (Anytype Low → Apple 9)", () => {
    const task = makeTask({
      id: "tx012",
      name: "低优先级",
      priorityName: "Low",
    });

    const reminder = mapper.toTarget(task);
    expect(reminder.priority).toBe(9); // low
  });

  test("toTarget: 备注中写入 conexId 标签", () => {
    const task = makeTask({
      id: "tx999",
      name: "追踪测试",
    });

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
    expect(task.snippet).toContain("记得买全脂牛奶");
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
    const a = makeTask({ id: "t1", name: "任务", done: false, priorityName: "Medium" });
    const b = makeTask({ id: "t2", name: "任务", done: false, priorityName: "Medium" });

    expect(computeContentHash(a)).toBe(computeContentHash(b));
  });

  test("computeContentHash: 不同内容产生不同哈希", () => {
    const a = makeTask({ id: "t1", name: "任务A" });
    const b = makeTask({ id: "t2", name: "任务B" });

    expect(computeContentHash(a)).not.toBe(computeContentHash(b));
  });

  test("detectConflict: 内容一致时无冲突", () => {
    const a = makeTask({ id: "t1", name: "任务", done: false });
    const b: ReminderData = { title: "任务", isCompleted: false };

    expect(mapper.detectConflict(a, b)).toHaveLength(0);
  });

  test("detectConflict: 标题不一致时报告冲突", () => {
    const a = makeTask({ id: "t1", name: "任务A" });
    const b: ReminderData = { title: "任务B" };

    const conflicts = mapper.detectConflict(a, b);
    expect(conflicts.some((c) => c.field === "title")).toBe(true);
  });
});
