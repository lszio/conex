// CONEX Sync Engine v2 — Canonical Store 居中架构
//
// 同步流程:
//   1. Anytype → Canonical Store (拉取 Anytype 变更，写入 tasks 表)
//   2. Apple → Canonical Store (拉取 Apple 变更，写入 tasks 表)
//   3. Canonical Store → Anytype (推送未同步的变更到 Anytype)
//   4. Canonical Store → Apple (推送未同步的变更到 Apple)

import type { AnytypeObject } from "../adapters/anytype/types.js";
import { AnytypeAdapter } from "../adapters/anytype/adapter.js";
import { AppleRemindersAdapter } from "../adapters/apple/reminders.js";
import type { ReminderData } from "../adapters/apple/types.js";
import type { SyncResult, AdapterStatus } from "../adapters/types.js";
import type { CanonicalTask } from "../store/schema.js";
import { getCanonicalStore, type CanonicalStore } from "../store/canonical-store.js";
import {
  anytypeToCanonical,
  canonicalToAnytypePatch,
  computeAnytypeContentHash,
} from "../mappers/anytype-mapper.js";
import {
  reminderToCanonical,
  canonicalToReminder,
  extractConexIdFromNotes,
  hasReminderChanged,
} from "../mappers/reminder-mapper.js";
import { getSyncStore } from "../store/db.js";

export interface SyncConfig {
  spaceId?: string;
  batchSize: number;
  ensureRemindersList: boolean;
  /** 两个方向的变更检测窗口（秒） */
  anytypeWindowSec: number;
  appleWindowSec: number;
}

const DEFAULT_CONFIG: SyncConfig = {
  batchSize: 50,
  ensureRemindersList: true,
  anytypeWindowSec: 120,   // Anytype 往前看 2 分钟
  appleWindowSec: 300,     // Apple 往前看 5 分钟
};

export class SyncEngine {
  private anytype: AnytypeAdapter;
  private reminders: AppleRemindersAdapter;
  private store: CanonicalStore;
  private legacyStore: ReturnType<typeof getSyncStore>;
  private config: SyncConfig;

  private constructor(config?: Partial<SyncConfig>) {
    this.anytype = null as any;
    this.reminders = new AppleRemindersAdapter();
    this.store = getCanonicalStore();
    this.legacyStore = getSyncStore();
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /** Async factory */
  static async create(config?: Partial<SyncConfig>): Promise<SyncEngine> {
    const engine = new SyncEngine(config);
    engine.anytype = await AnytypeAdapter.create();
    return engine;
  }

  // ──────────────────────────────────────────
  // Status
  // ──────────────────────────────────────────

  async checkStatus(): Promise<{
    anytype: AdapterStatus;
    reminders: { available: boolean; error?: string };
    store: { path: string; objects: number; lastSync?: string };
  }> {
    const anytypeStatus = await this.anytype.healthCheck();
    const remindersStatus = await this.reminders.ensureReady();
    const state = this.legacyStore.getSyncState();

    return {
      anytype: anytypeStatus,
      reminders: {
        available: remindersStatus.available,
        error: remindersStatus.error,
      },
      store: {
        path: "~/.conex/sync.db (tasks)",
        objects: this.store.count(),
        lastSync: state.last_poll_at || undefined,
      },
    };
  }

  // ──────────────────────────────────────────
  // Main sync (bidirectional)
  // ──────────────────────────────────────────

  async sync(): Promise<SyncResult> {
    const start = Date.now();
    const result: SyncResult = {
      success: false,
      created: 0,
      updated: 0,
      skipped: 0,
      conflicts: 0,
      errors: [],
      duration: 0,
    };

    try {
      // 1. Ensure reminders list
      if (this.config.ensureRemindersList) {
        const ready = await this.reminders.ensureReady();
        if (!ready.available) {
          throw new Error(`Reminders 不可用: ${ready.error}`);
        }
      }

      // 2. Anytype → Canonical Store
      const anytypeChanges = await this.pollAnytype();
      for (const obj of anytypeChanges) {
        try {
          const action = await this.syncAnytypeToCanonical(obj);
          if (action === "created") result.created += 1;
          else if (action === "updated") result.updated += 1;
          else result.skipped += 1;
        } catch (e: any) {
          result.errors.push(`[Anytype→Canonical] ${obj.name}: ${e.message}`);
        }
      }

      // 3. Apple → Canonical Store
      const appleChanges = await this.pollApple();
      for (const reminder of appleChanges) {
        try {
          await this.syncAppleToCanonical(reminder);
          result.updated += 1;
        } catch (e: any) {
          result.errors.push(`[Apple→Canonical] ${reminder.title}: ${e.message}`);
        }
      }

      // 4. Canonical Store → Anytype (push pending changes)
      const anytypePushCount = await this.pushToAnytype();
      result.updated += anytypePushCount;

      // 5. Canonical Store → Apple (push pending changes)
      const applePushCount = await this.pushToApple();
      result.updated += applePushCount;

      // 6. Update sync state (legacy compat)
      this.legacyStore.updateSyncState({
        last_poll_at: new Date().toISOString(),
        total_synced: this.legacyStore.getSyncState().total_synced + anytypeChanges.length + appleChanges.length,
        last_error: result.errors.length > 0 ? result.errors.join("; ").slice(0, 500) : null,
      });

      result.success = true;
      result.duration = Date.now() - start;
    } catch (e: any) {
      result.errors.push(e.message);
      result.duration = Date.now() - start;
      this.legacyStore.updateSyncState({ last_error: e.message.slice(0, 500) });
    }

    return result;
  }

  // ──────────────────────────────────────────
  // 方向 1: Anytype → Canonical Store
  // ──────────────────────────────────────────

  private async pollAnytype(): Promise<AnytypeObject[]> {
    const since = new Date(Date.now() - this.config.anytypeWindowSec * 1000).toISOString();
    const tasks = await this.anytype.queryObjects({
      type: "Task",
      spaceId: this.config.spaceId,
      limit: this.config.batchSize,
      sort: "lastModifiedDesc",
    });
    // 时间窗口过滤
    return tasks.filter((t) => {
      const modified = t.properties?.find((p) => p.key === "last_modified_date")?.date;
      return modified && modified >= since;
    });
  }

  private async syncAnytypeToCanonical(obj: AnytypeObject): Promise<"created" | "updated" | "skipped"> {
    // 检查是否已有 canonical 记录
    const existing = this.store.getByAnytypeId(obj.id);

    // 计算当前哈希
    const currentHash = computeAnytypeContentHash(obj);

    if (!existing) {
      // 新对象: 创建 canonical 记录
      const task = anytypeToCanonical(obj);
      this.store.insert(task);
      return "created";
    }

    // 检查哈希是否有变化
    // (existing.raw_json 存的是上次同步时的完整对象，这里简单判断: 直接比对)
    try {
      const prevObj = existing.raw_json ? JSON.parse(existing.raw_json) as AnytypeObject : null;
      if (prevObj && computeAnytypeContentHash(prevObj) === currentHash) {
        return "skipped"; // 无变更
      }
    } catch {
      // 解析失败，重新同步
    }

    // 有变更: 更新 canonical 记录
    const task = anytypeToCanonical(obj);
    task.id = existing.id;   // 保留 canonical ID
    task.created_at = existing.created_at; // 保留创建时间
    this.store.upsert(task);

    // 记录冲突日志（如果 Apple 侧也有变更）
    if (existing.apple_id && existing.updated_at > existing.last_synced!) {
      this.legacyStore.logConflict({
        anytype_id: obj.id,
        apple_id: existing.apple_id,
        conflict_type: "content_mismatch",
      });
    }

    return "updated";
  }

  // ──────────────────────────────────────────
  // 方向 2: Apple → Canonical Store
  // ──────────────────────────────────────────

  private async pollApple(): Promise<ReminderData[]> {
    const since = new Date(Date.now() - this.config.appleWindowSec * 1000);
    return this.reminders.getRecentCompletions(since);
  }

  private async syncAppleToCanonical(reminder: ReminderData): Promise<void> {
    const conexId = reminder.conexId || extractConexIdFromNotes(reminder.notes);
    if (!conexId) return; // 不是 CONEX 管理的任务

    // 查找 canonical 记录
    let task = conexId ? this.store.getByAnytypeId(conexId) : null;
    if (!task && reminder.id) {
      task = this.store.getByAppleId(reminder.id);
    }

    if (!task) return; // 没有对应的 canonical 记录

    // 如果已经完成，跳过
    if (task.is_completed) return;

    // 更新 canonical: 标记完成，记录完成时间
    this.store.update(task.id, {
      is_completed: 1,
      completion_date: new Date().toISOString(),
      last_synced: null, // 标记为需要推送回 Anytype
    });

    this.legacyStore.logConflict({
      anytype_id: conexId,
      apple_id: reminder.id || task.apple_id || "",
      conflict_type: "apple_completed",
    });
  }

  // ──────────────────────────────────────────
  // 方向 3: Canonical Store → Anytype
  // ──────────────────────────────────────────

  private async pushToAnytype(): Promise<number> {
    // 查找 last_synced 为 null 的任务（有待推送的变更）
    const pending = this.store.listUpdatedSince(
      new Date(0).toISOString(),
      50,
    ).filter((t) => (t.source === "anytype" || t.source === "apple" || t.source === "manual") && t.last_synced === null);

    let count = 0;
    for (const task of pending) {
      if (!task.anytype_id || !task.space_id) continue;

      try {
        // 获取 Anytype 当前状态做比对
        const currentObj = await this.anytype.getObject(task.space_id, task.anytype_id);

        // 构建 PATCH
        const patches = canonicalToAnytypePatch(task, currentObj);
        if (patches.length === 0) continue;

        // 先更新状态（如果用 status）
        if (task.is_completed) {
          await this.anytype.setTaskDone(task.space_id, task.anytype_id);
        } else if (patches.some((p) => p.key === "status")) {
          const statusPatch = patches.find((p) => p.key === "status")!;
          await this.anytype.setProperties(task.space_id, task.anytype_id, [statusPatch]);
        }

        // 其他字段
        const otherPatches = patches.filter((p) => p.key !== "name" && p.key !== "status" && p.key !== "done");
        if (otherPatches.length > 0) {
          await this.anytype.setProperties(task.space_id, task.anytype_id, otherPatches);
        }

        // 更新 last_synced
        this.store.update(task.id, { last_synced: new Date().toISOString() });
        count += 1;
      } catch (e: any) {
        console.error(`  ⚠️ push Anytype 失败 [${task.name}]: ${e.message}`);
      }
    }
    return count;
  }

  // ──────────────────────────────────────────
  // 方向 4: Canonical Store → Apple
  // ──────────────────────────────────────────

  private async pushToApple(): Promise<number> {
    const pending = this.store.listUpdatedSince(
      new Date(0).toISOString(),
      50,
    ).filter((t) => (t.source === "anytype" || t.source === "manual") && t.last_synced === null);

    let count = 0;
    for (const task of pending) {
      if (!task.anytype_id) continue;

      try {
        const reminder = canonicalToReminder(task);
        let appleId = task.apple_id;

        if (appleId) {
          // 已有 Apple 提醒 → 判断是否需要更新
          const existing = await this.reminders.getReminder(appleId);
          if (existing) {
            // 提醒存在，检查是否有变更
            if (!hasReminderChanged(task, existing)) {
              continue; // 无变更，跳过
            }

            if (task.is_completed) {
              await this.reminders.completeReminder(appleId);
            } else {
              await this.reminders.updateReminder(appleId, reminder);
            }
          } else {
            // 提醒已被删除 → 重新创建
            appleId = await this.reminders.createReminder(reminder);
            this.store.update(task.id, { apple_id: appleId });
          }
        } else {
          // 新任务 → 创建 Apple 提醒（先防重复检查）
          // 检查是否已有同名 + 同 conexId 的提醒（防止历史残留导致重复创建）
          try {
            const existingList = await this.reminders.listReminders();
            const dup = existingList.find((r) => {
              if (r.title !== task.name) return false;
              const conexId = extractConexIdFromNotes(r.notes);
              return conexId === task.anytype_id;
            });
            if (dup && dup.id) {
              appleId = dup.id;
              this.store.update(task.id, { apple_id: appleId });
            } else {
              appleId = await this.reminders.createReminder(reminder);
              this.store.update(task.id, { apple_id: appleId });
            }
          } catch {
            appleId = await this.reminders.createReminder(reminder);
            this.store.update(task.id, { apple_id: appleId });
          }
        }

        // 同步周期规则（AppleScript 无法直接设置，通过 EventKit 二进制）
        if (appleId && task.recurrence) {
          await this.reminders.setRecurrence(appleId, task.recurrence);
        } else if (appleId && !task.recurrence) {
          await this.reminders.removeRecurrence(appleId);
        }

        this.store.update(task.id, { last_synced: new Date().toISOString() });
        count += 1;
      } catch (e: any) {
        console.error(`  ⚠️ push Apple 失败 [${task.name}]: ${e.message}`);
      }
    }
    return count;
  }
}
