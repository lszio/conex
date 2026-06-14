// CONEX 双向同步守护进程 v2
// 基于 Canonical Store 居中架构

import { AnytypeAdapter } from "../adapters/anytype/adapter.js";
import { AppleRemindersAdapter } from "../adapters/apple/reminders.js";
import type { ReminderData } from "../adapters/apple/types.js";
import {
  anytypeToCanonical,
  computeAnytypeContentHash,
  canonicalToAnytypePatch,
} from "../mappers/anytype-mapper.js";
import {
  reminderToCanonical,
  canonicalToReminder,
  extractConexIdFromNotes,
} from "../mappers/reminder-mapper.js";
import { getCanonicalStore, type CanonicalStore } from "../store/canonical-store.js";
import { getSyncStore } from "../store/db.js";

export interface DaemonConfig {
  spaceId: string;
  pollIntervalMs: number;
  changeWindowSec: number;
  appleChangeWindowSec: number;
}

const DEFAULT_CONFIG: DaemonConfig = {
  spaceId: "",
  pollIntervalMs: 30_000,
  changeWindowSec: 120,
  appleChangeWindowSec: 300,
};

export class BidirectionalDaemon {
  private reminders!: AppleRemindersAdapter;
  private store!: CanonicalStore;
  private legacyStore!: ReturnType<typeof getSyncStore>;
  private config: DaemonConfig;
  private running = false;
  private _intervalId: ReturnType<typeof setInterval> | null = null;
  private _checkCount = 0;

  constructor(config: Partial<DaemonConfig> = {}) {
    this.reminders = new AppleRemindersAdapter();
    this.store = getCanonicalStore();
    this.legacyStore = getSyncStore();
    this.config = { ...DEFAULT_CONFIG, ...config };
    // anytype 由工厂方法初始化
    this._anytypePromise = null;
  }

  private _anytypePromise: Promise<AnytypeAdapter> | null = null;

  private async getAnytype(): Promise<AnytypeAdapter> {
    if (!this._anytypePromise) {
      this._anytypePromise = AnytypeAdapter.create();
    }
    return this._anytypePromise;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this._checkCount = 0;
    console.log(`🔁 CONEX 守护进程启动 (每 ${this.config.pollIntervalMs / 1000}s 轮询)`);
    this.tick();
    this._intervalId = setInterval(() => this.tick(), this.config.pollIntervalMs);
  }

  stop(): void {
    this.running = false;
    if (this._intervalId) {
      clearInterval(this._intervalId);
      this._intervalId = null;
    }
    console.log("⏹ CONEX 守护进程停止");
  }

  get isRunning(): boolean {
    return this.running;
  }

  async tick(): Promise<{
    anytypeToCanonical: { created: number; updated: number; skipped: number };
    appleToCanonical: { updated: number };
    pushToAnytype: number;
    pushToApple: number;
  }> {
    const result = {
      anytypeToCanonical: { created: 0, updated: 0, skipped: 0 },
      appleToCanonical: { updated: 0 },
      pushToAnytype: 0,
      pushToApple: 0,
    };

    try {
      // 方向 1: Anytype → Canonical
      const tasks = await this.pollAnytype();
      for (const obj of tasks) {
        try {
          // 检查是否已有 canonical 记录
          const existing = this.store.getByAnytypeId(obj.id);
          const hash = computeAnytypeContentHash(obj);

          if (!existing) {
            this.store.insert(anytypeToCanonical(obj));
            result.anytypeToCanonical.created += 1;
          } else {
            try {
              const prev = existing.raw_json ? JSON.parse(existing.raw_json) : null;
              if (prev && computeAnytypeContentHash(prev) === hash) {
                result.anytypeToCanonical.skipped += 1;
                continue;
              }
            } catch {}
            const task = anytypeToCanonical(obj);
            task.id = existing.id;
            task.created_at = existing.created_at;
            this.store.upsert(task);
            result.anytypeToCanonical.updated += 1;
          }
        } catch (e: any) {
          console.error(`  ⚠️ Anytype→Canonical 失败 [${obj.name}]: ${e.message}`);
        }
      }

      // 方向 2: Apple → Canonical (完成状态同步)
      const appleChanges = await this.pollApple();
      for (const reminder of appleChanges) {
        try {
          const conexId = reminder.conexId || extractConexIdFromNotes(reminder.notes);
          if (!conexId) continue;
          const task = this.store.getByAnytypeId(conexId);
          if (!task || task.is_completed) continue;

          this.store.update(task.id, {
            is_completed: 1,
            completion_date: new Date().toISOString(),
            last_synced: null, // 标记为待推送
          });
          result.appleToCanonical.updated += 1;
        } catch (e: any) {
          console.error(`  ⚠️ Apple→Canonical 失败 [${reminder.title}]: ${e.message}`);
        }
      }

      // 方向 3: Canonical → Anytype
      result.pushToAnytype = await this.pushToAnytype();

      // 方向 4: Canonical → Apple
      result.pushToApple = await this.pushToApple();

      this._checkCount += 1;

      const total = result.anytypeToCanonical.created + result.anytypeToCanonical.updated
        + result.appleToCanonical.updated + result.pushToAnytype + result.pushToApple;
      if (total > 0) {
        console.log(`  📊 Tick #${this._checkCount}: ${total} 变更同步`);
      }
    } catch (e: any) {
      console.error(`  ❌ Tick 失败: ${e.message}`);
    }

    return result;
  }

  private async pollAnytype(): Promise<any[]> {
    const since = new Date(Date.now() - this.config.changeWindowSec * 1000).toISOString();
    const at = await this.getAnytype();
    const tasks = await at.queryObjects({
      type: "Task",
      spaceId: this.config.spaceId,
      limit: 100,
      sort: "lastModifiedDesc",
    });
    return tasks.filter((t: any) => {
      const modified = t.properties?.find((p: any) => p.key === "last_modified_date")?.date;
      return modified && modified >= since;
    });
  }

  private async pollApple(): Promise<ReminderData[]> {
    const since = new Date(Date.now() - this.config.appleChangeWindowSec * 1000);
    return this.reminders.getRecentCompletions(since);
  }

  private async pushToAnytype(): Promise<number> {
    if (!this.config.spaceId) return 0;
    const pending = this.store.list({ limit: 50 })
      .filter((t) => t.anytype_id && t.space_id && (t.is_completed || t.last_synced === null));

    let count = 0;
    for (const task of pending) {
      if (!task.anytype_id || !task.space_id) continue;
      try {
        if (task.is_completed) {
          const at = await this.getAnytype();
          await at.setTaskDone(task.space_id, task.anytype_id);
        } else if (task.status_select && task.status_tag_key) {
          const at = await this.getAnytype();
          await at.setProperties(task.space_id, task.anytype_id, [
            { key: "status", select: task.status_tag_key },
          ]);
        }
        this.store.update(task.id, { last_synced: new Date().toISOString() });
        count += 1;
      } catch (e: any) {
        console.error(`  ⚠️ push Anytype 失败 [${task.name}]: ${e.message}`);
      }
    }
    return count;
  }

  private async pushToApple(): Promise<number> {
    const pending = this.store.list({ limit: 50 })
      .filter((t) => t.anytype_id && t.last_synced === null);

    let count = 0;
    for (const task of pending) {
      if (!task.anytype_id) continue;
      try {
        const reminder = canonicalToReminder(task);

        if (task.apple_id) {
          if (task.is_completed) {
            await this.reminders.completeReminder(task.apple_id);
          } else {
            await this.reminders.updateReminder(task.apple_id, reminder);
          }
        } else {
          const appleId = await this.reminders.createReminder(reminder);
          this.store.update(task.id, { apple_id: appleId });
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
