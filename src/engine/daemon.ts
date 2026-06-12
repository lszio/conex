// CONEX 双向同步守护进程
// 高效轮询 Anytype 和 Apple Reminders，双向同步变更
//
// 运行模式:
//   1. 从 Anytype 拉取变更 → 推送到 Apple Reminders
//   2. 从 Apple Reminders 拉取变更 → 推送到 Anytype

import type { AnytypeObject } from "../adapters/anytype/types.js";
import { isCompletedByStatus } from "../adapters/anytype/types.js";
import { AnytypeAdapter, getAnytypeAdapter } from "../adapters/anytype/adapter.js";
import { AppleRemindersAdapter } from "../adapters/apple/reminders.js";
import type { ReminderData } from "../adapters/apple/types.js";
import { TaskToReminderMapper, computeContentHash, extractConexIdFromNotes } from "../mappers/task-to-reminder.js";
import { getSyncStore, type SyncStore } from "../store/db.js";

export interface DaemonConfig {
  spaceId: string;
  pollIntervalMs: number;
  /** Anytype 变更检测窗口（秒）— 只拉取这个时间内的变更 */
  changeWindowSec: number;
  /** Apple 侧变更检测窗口 */
  appleChangeWindowSec: number;
}

const DEFAULT_CONFIG: DaemonConfig = {
  spaceId: "",
  pollIntervalMs: 30_000,      // 30 秒
  changeWindowSec: 120,         // 往前看 2 分钟
  appleChangeWindowSec: 300,    // Apple 侧往前看 5 分钟
};

export class BidirectionalDaemon {
  private anytype: AnytypeAdapter;
  private reminders: AppleRemindersAdapter;
  private store: SyncStore;
  private mapper: TaskToReminderMapper;
  private config: DaemonConfig;
  private running = false;
  private _intervalId: ReturnType<typeof setInterval> | null = null;
  private _checkCount = 0;

  constructor(config: Partial<DaemonConfig> = {}) {
    this.anytype = getAnytypeAdapter();
    this.reminders = new AppleRemindersAdapter();
    this.store = getSyncStore();
    this.mapper = new TaskToReminderMapper("bidirectional");
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /** 启动守护进程 */
  start(): void {
    if (this.running) return;
    this.running = true;
    this._checkCount = 0;
    console.log(`🔁 CONEX 守护进程启动 (每 ${this.config.pollIntervalMs / 1000}s 轮询)`);

    // 立即执行一次
    this.tick();

    // 定时轮询
    this._intervalId = setInterval(() => this.tick(), this.config.pollIntervalMs);
  }

  /** 停止守护进程 */
  stop(): void {
    this.running = false;
    if (this._intervalId) {
      clearInterval(this._intervalId);
      this._intervalId = null;
    }
    console.log("⏹ CONEX 守护进程停止");
  }

  /** 判断是否在运行 */
  get isRunning(): boolean {
    return this.running;
  }

  /** 单次同步 tick */
  async tick(): Promise<{
    anytypeToApple: { created: number; updated: number; skipped: number };
    appleToAnytype: { updated: number };
  }> {
    const result = {
      anytypeToApple: { created: 0, updated: 0, skipped: 0 },
      appleToAnytype: { updated: 0 },
    };

    try {
      // ── 方向 1: Anytype → Apple Reminders ──
      const anytypeChanges = await this.pollAnytypeChanges();
      for (const task of anytypeChanges) {
        try {
          const action = await this.syncAnytypeToApple(task);
          if (action === "created") result.anytypeToApple.created += 1;
          else if (action === "updated") result.anytypeToApple.updated += 1;
          else result.anytypeToApple.skipped += 1;
        } catch (e: any) {
          console.error(`  ⚠️ Anytype→Apple 失败 [${task.name}]: ${e.message}`);
        }
      }

      // ── 方向 2: Apple Reminders → Anytype ──
      // 检测 Apple 侧近期完成的提醒，写回 Anytype
      const appleChanges = await this.pollAppleChanges();
      for (const reminder of appleChanges) {
        try {
          await this.syncAppleToAnytype(reminder);
          result.appleToAnytype.updated += 1;
        } catch (e: any) {
          console.error(`  ⚠️ Apple→Anytype 失败 [${reminder.title}]: ${e.message}`);
        }
      }

      this._checkCount += 1;

      // 只在有变更时输出日志
      const total = result.anytypeToApple.created + result.anytypeToApple.updated
        + result.appleToAnytype.updated;
      if (total > 0) {
        console.log(`  📊 Tick #${this._checkCount}: ${total} 变更同步`);
      }
    } catch (e: any) {
      console.error(`  ❌ Tick 失败: ${e.message}`);
    }

    return result;
  }

  // ──────────────────────────────────────────
  // Anytype 变更检测
  // ──────────────────────────────────────────

  /** 轮询 Anytype 获取最近变更的任务 */
  private async pollAnytypeChanges(): Promise<AnytypeObject[]> {
    const since = new Date(Date.now() - this.config.changeWindowSec * 1000).toISOString();
    const tasks = await this.anytype.queryObjects({
      type: "Task",
      spaceId: this.config.spaceId,
      limit: 100,
      sort: "lastModifiedDesc",
    });

    // 只返回在时间窗口内的变更
    return tasks.filter((t) => {
      const prop = t.properties?.find((p) => p.key === "last_modified_date");
      const modified = prop?.date;
      return modified && modified >= since;
    });
  }

  /** 同步 Anytype Task → Apple Reminder */
  private async syncAnytypeToApple(task: AnytypeObject): Promise<"created" | "updated" | "skipped"> {
    const idMap = this.store.getIdMap(task.id);
    const currentHash = computeContentHash(task);

    if (!idMap) {
      // 新任务 → 创建提醒
      const reminderData = this.mapper.toTarget(task);
      const appleId = await this.reminders.createReminder(reminderData);
      this.store.upsertIdMap({
        anytype_id: task.id,
        apple_id: appleId,
        apple_type: "reminder",
        direction: "bidirectional",
        last_hash: currentHash,
      });
      return "created";
    }

    // 已有映射 → 检查哈希
    if (idMap.last_hash === currentHash) {
      return "skipped";
    }

    // 内容变更 → 更新提醒
    const reminderData = this.mapper.toTarget(task);
    await this.reminders.updateReminder(idMap.apple_id, reminderData);
    this.store.upsertIdMap({
      ...idMap,
      last_hash: currentHash,
    });
    return "updated";
  }

  // ──────────────────────────────────────────
  // Apple Reminders 变更检测
  // ──────────────────────────────────────────

  /** 轮询 Apple Reminders 获取近期完成的提醒 */
  private async pollAppleChanges(): Promise<ReminderData[]> {
    const since = new Date(Date.now() - this.config.appleChangeWindowSec * 1000);
    const completed = await this.reminders.getRecentCompletions(since);
    return completed;
  }

  /** 同步 Apple Reminder 完成状态 → Anytype */
  private async syncAppleToAnytype(reminder: ReminderData): Promise<void> {
    const conexId = reminder.conexId || extractConexIdFromNotes(reminder.notes);
    if (!conexId) {
      // 没有 conexId → 不是 CONEX 管理的提醒，忽略
      return;
    }

    const idMap = this.store.getIdMap(conexId);
    if (!idMap || !this.config.spaceId) return;

    // 获取 Anytype 当前状态
    try {
      const task = await this.anytype.getObject(this.config.spaceId, conexId);
      if (!task) return;

      // 如果 Anytype 已经标记为完成，跳过
      if (isCompletedByStatus(task)) return;

      // Apple 侧完成了 → 在 Anytype 也标记为 DONE
      await this.anytype.setTaskDone(this.config.spaceId, conexId);
      console.log(`  ✅ Apple→Anytype: "${reminder.title}" 标记为完成`);

      // 更新本地哈希
      const updatedHash = computeContentHash(task);
      this.store.upsertIdMap({
        ...idMap,
        last_hash: updatedHash,
      });

      // 记录日志
      this.store.logConflict({
        anytype_id: conexId,
        apple_id: idMap.apple_id,
        conflict_type: "apple_completed",
      });
    } catch (e: any) {
      // 对象可能已被删除
      console.error(`  ⚠️ 无法更新 Anytype 对象 ${conexId}: ${e.message}`);
    }
  }
}
