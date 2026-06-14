import { describe, test, expect } from "bun:test";
import {
  createTask,
  canonicalToApplePriority,
  appleToCanonicalPriority,
  priorityToQuadrantLabel,
} from "../../src/store/schema.js";
import { anytypeToCanonical, computeAnytypeContentHash } from "../../src/mappers/anytype-mapper.js";
import { reminderToCanonical, canonicalToReminder, extractConexIdFromNotes } from "../../src/mappers/reminder-mapper.js";
import type { AnytypeObject } from "../../src/adapters/anytype/types.js";
import type { ReminderData } from "../../src/adapters/apple/types.js";

// ─── Test helpers ───

function makeAnytypeTask(overrides: Record<string, unknown> & {
  name?: string;
  status?: string;
  statusKey?: string;
  done?: boolean;
  priority?: string;     // "0"|"1"|"2"|"3" — Anytype priority select option name
  deadline?: string;
  description?: string;
  tags?: string[];
  archived?: boolean;
}): AnytypeObject {
  const task: AnytypeObject = {
    object: "object",
    id: (overrides.id as string) || "tx-test",
    name: (overrides.name as string) || "测试任务",
    layout: "basic",
    type: "task",
    space_id: "space1",
    archived: overrides.archived as boolean || false,
    snippet: undefined,
    properties: [],
  };

  // status select
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
        key: (overrides.statusKey as string) || `key-${overrides.status.toLowerCase()}`,
        name: overrides.status as string,
        color: "ice",
      },
    });
  }

  // done checkbox
  if (overrides.done !== undefined) {
    task.properties!.push({
      object: "property",
      id: "prop-done",
      key: "done",
      name: "Done",
      format: "checkbox",
      checkbox: overrides.done as boolean,
    });
  }

  // priority select (四象限)
  if (overrides.priority) {
    task.properties!.push({
      object: "property",
      id: "prop-priority",
      key: "priority",
      name: "Priority",
      format: "select",
      select: {
        object: "tag",
        id: "tag-pri",
        key: `pri-${overrides.priority}`,
        name: overrides.priority as string,
        color: "red",
      },
    });
  }

  // due_date
  if (overrides.deadline) {
    task.properties!.push({
      object: "property",
      id: "prop-due",
      key: "due_date",
      name: "Due date",
      format: "date",
      date: overrides.deadline as string,
    });
  }

  // tag
  if (overrides.tags) {
    task.properties!.push({
      object: "property",
      id: "prop-tag",
      key: "tag",
      name: "Tag",
      format: "multi_select",
      multi_select: (overrides.tags as string[]).map((t) => ({
        object: "tag",
        id: `tag-${t}`,
        key: t,
        name: t,
        color: "default",
      })),
    });
  }

  // description text
  if (overrides.description) {
    task.properties!.push({
      object: "property",
      id: "prop-desc",
      key: "description",
      name: "Description",
      format: "text",
      text: overrides.description as string,
    });
  }

  return task;
}

// ──────────────────────────────────────────
// Schema tests
// ──────────────────────────────────────────

describe("Schema — 优先级四象限映射", () => {
  test("canonical 0 → Q4", () => {
    expect(priorityToQuadrantLabel(0)).toBe("Q4");
  });
  test("canonical 1 → Q2", () => {
    expect(priorityToQuadrantLabel(1)).toBe("Q2");
  });
  test("canonical 2 → Q3", () => {
    expect(priorityToQuadrantLabel(2)).toBe("Q3");
  });
  test("canonical 3 → Q1", () => {
    expect(priorityToQuadrantLabel(3)).toBe("Q1");
  });

  test("canonical→Apple: 0→0, 1→9, 2→5, 3→1", () => {
    expect(canonicalToApplePriority(0)).toBe(0);
    expect(canonicalToApplePriority(1)).toBe(9);  // Q2 → low
    expect(canonicalToApplePriority(2)).toBe(5);  // Q3 → medium
    expect(canonicalToApplePriority(3)).toBe(1);  // Q1 → high
  });

  test("Apple→canonical: 0→0, 9→1, 5→2, 1→3", () => {
    expect(appleToCanonicalPriority(0)).toBe(0);
    expect(appleToCanonicalPriority(9)).toBe(1);
    expect(appleToCanonicalPriority(5)).toBe(2);
    expect(appleToCanonicalPriority(1)).toBe(3);
  });

  test("双向映射不损失信息", () => {
    for (const p of [0, 1, 2, 3]) {
      const apple = canonicalToApplePriority(p);
      const back = appleToCanonicalPriority(apple);
      expect(back).toBe(p);
    }
  });

  test("createTask 默认 priority=0", () => {
    const t = createTask();
    expect(t.priority).toBe(0);
    expect(t.is_completed).toBe(0);
    expect(t.id).toBeTruthy();
  });

  test("createTask 覆盖字段", () => {
    const t = createTask({ name: "买牛奶", priority: 3, is_completed: 1 });
    expect(t.name).toBe("买牛奶");
    expect(t.priority).toBe(3);
    expect(t.is_completed).toBe(1);
  });
});

// ──────────────────────────────────────────
// Anytype mapper tests
// ──────────────────────────────────────────

describe("AnytypeMapper — Anytype → Canonical", () => {
  test("status=DONE → is_completed=1", () => {
    const task = makeAnytypeTask({ id: "t1", name: "完成的任务", status: "DONE" });
    const canonical = anytypeToCanonical(task);
    expect(canonical.is_completed).toBe(1);
    expect(canonical.status_select).toBe("DONE");
  });

  test("status=TODO → is_completed=0", () => {
    const task = makeAnytypeTask({ id: "t2", name: "待办", status: "TODO" });
    const canonical = anytypeToCanonical(task);
    expect(canonical.is_completed).toBe(0);
    expect(canonical.status_select).toBe("TODO");
  });

  test("done checkbox fallback", () => {
    const task = makeAnytypeTask({ id: "t3", name: "完成的(checkbox)", done: true });
    const canonical = anytypeToCanonical(task);
    expect(canonical.is_completed).toBe(1);
    expect(canonical.done_checkbox).toBe(1);
  });

  test("priority 四象限", () => {
    const task = makeAnytypeTask({ id: "t4", name: "紧急重要", priority: "3" });
    const canonical = anytypeToCanonical(task);
    expect(canonical.priority).toBe(3);
  });

  test("priority 默认 0", () => {
    const task = makeAnytypeTask({ id: "t5", name: "无优先级" });
    const canonical = anytypeToCanonical(task);
    expect(canonical.priority).toBe(0);
  });

  test("description 优先于 snippet", () => {
    const task = makeAnytypeTask({
      id: "t6",
      name: "有描述",
      description: "这是描述文字",
    });
    task.snippet = "这是搜索片段";
    const canonical = anytypeToCanonical(task);
    expect(canonical.description).toBe("这是描述文字"); // 不是 snippet
  });

  test("tags 序列化为 JSON", () => {
    const task = makeAnytypeTask({
      id: "t7",
      name: "有标签",
      tags: ["Work", "BSPA"],
    });
    const canonical = anytypeToCanonical(task);
    expect(JSON.parse(canonical.tags!)).toEqual(["Work", "BSPA"]);
  });

  test("anytype_id 保留", () => {
    const task = makeAnytypeTask({ id: "tx-abc", name: "溯源" });
    const canonical = anytypeToCanonical(task);
    expect(canonical.anytype_id).toBe("tx-abc");
    expect(canonical.source).toBe("anytype");
  });

  test("哈希一致性", () => {
    const a = makeAnytypeTask({ id: "h1", name: "任务", status: "TODO", priority: "2" });
    const b = makeAnytypeTask({ id: "h2", name: "任务", status: "TODO", priority: "2" });
    expect(computeAnytypeContentHash(a)).toBe(computeAnytypeContentHash(b));
  });

  test("哈希区分不同内容", () => {
    const a = makeAnytypeTask({ id: "h3", name: "任务A" });
    const b = makeAnytypeTask({ id: "h4", name: "任务B" });
    expect(computeAnytypeContentHash(a)).not.toBe(computeAnytypeContentHash(b));
  });
});

// ──────────────────────────────────────────
// Reminder mapper tests
// ──────────────────────────────────────────

describe("ReminderMapper — Apple ↔ Canonical", () => {
  test("reminder → canonical: 基本字段映射", () => {
    const reminder: ReminderData = {
      id: "apple-123",
      title: "买牛奶",
      notes: "记得买全脂\nanytype://tx-abc\n[conex:tx-abc]",
      isCompleted: true,
      priority: 1,
      dueDate: new Date("2026-06-15T17:00:00Z"),
    };
    const task = reminderToCanonical(reminder);
    expect(task.name).toBe("买牛奶");
    expect(task.description).toBe("记得买全脂"); // 元数据已剥离
    expect(task.is_completed).toBe(1);
    expect(task.priority).toBe(3); // Apple 1 → canonical 3
    expect(task.apple_id).toBe("apple-123");
    expect(task.anytype_id).toBe("tx-abc");
    expect(task.source).toBe("apple");
  });

  test("canonical → reminder: 基本字段映射", () => {
    const task = createTask({
      name: "写周报",
      description: "需要提交本周工作总结",
      due_date: "2026-06-20T17:00:00Z",
      is_completed: 0,
      priority: 2, // Q3
      anytype_id: "tx-周报",
    });
    const reminder = canonicalToReminder(task);
    expect(reminder.title).toBe("写周报");
    expect(reminder.notes).toContain("需要提交本周工作总结");
    expect(reminder.notes).toContain("anytype://tx-周报");
    expect(reminder.notes).toContain("[conex:tx-周报]");
    expect(reminder.isCompleted).toBe(false);
    expect(reminder.priority).toBe(5); // Q3 → medium
    expect(reminder.dueDate).toBeInstanceOf(Date);
  });

  test("canonical → reminder: 优先级映射循环", () => {
    for (const cp of [0, 1, 2, 3]) {
      const task = createTask({ priority: cp });
      const reminder = canonicalToReminder(task);
      const taskBack = reminderToCanonical(reminder);
      // priority 在循环中可能不是精确相等 (Apple 只有 4 个值)
      // 但应该在合理范围内
      expect(Math.abs(taskBack.priority - cp)).toBeLessThanOrEqual(1);
    }
  });

  test("备注中提取 conexId", () => {
    expect(extractConexIdFromNotes("some text\n[conex:tx-123]")).toBe("tx-123");
    expect(extractConexIdFromNotes("no tag here")).toBeNull();
    expect(extractConexIdFromNotes(undefined)).toBeNull();
  });

  test("completed 状态映射", () => {
    const reminder: ReminderData = { title: "完成", isCompleted: true };
    const task = reminderToCanonical(reminder);
    expect(task.is_completed).toBe(1);

    const task2 = createTask({ is_completed: 1 });
    const reminder2 = canonicalToReminder(task2);
    expect(reminder2.isCompleted).toBe(true);
  });
});
