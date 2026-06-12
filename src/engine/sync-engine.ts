// CONEX 同步引擎 — 核心同步循环调度器
//
// 工作流:
//   1. 轮询 Anytype API 获取变更的任务对象
//   2. 通过 Mapper 转换为 ReminderData
//   3. 检查 id_map 决定是创建还是更新 Apple 提醒
//   4. 冲突检测 + 日志
//   5. 更新 sync_state

import type { AnytypeObject } from "../adapters/anytype/types.js";
import { AnytypeAdapter, getAnytypeAdapter } from "../adapters/anytype/adapter.js";
import { AppleRemindersAdapter } from "../adapters/apple/reminders.js";
import type { ReminderData } from "../adapters/apple/types.js";
import type { SyncResult, AdapterStatus } from "../adapters/types.js";
import { TaskToReminderMapper, computeContentHash } from "../mappers/task-to-reminder.js";
import { getSyncStore, type SyncStore } from "../store/db.js";

export interface SyncConfig {
    /** Anytype 空间 ID (留空则同步所有空间的任务) */
    spaceId?: string;
    /** 每次查询的最大对象数 */
    batchSize: number;
    /** 是否在同步前先创建 CONEX 列表 */
    ensureRemindersList: boolean;
}

const DEFAULT_CONFIG: SyncConfig = {
    batchSize: 50,
    ensureRemindersList: true,
};

export class SyncEngine {
    private anytype: AnytypeAdapter;
    private reminders: AppleRemindersAdapter;
    private store: SyncStore;
    private mapper: TaskToReminderMapper;
    private config: SyncConfig;

    private constructor(config?: Partial<SyncConfig>) {
        this.anytype = null as any; // 将在 init 中初始化
        this.reminders = new AppleRemindersAdapter();
        this.store = getSyncStore();
        this.mapper = new TaskToReminderMapper("forward");
        this.config = { ...DEFAULT_CONFIG, ...config };
    }

    /** 异步工厂方法 */
    static async create(config?: Partial<SyncConfig>): Promise<SyncEngine> {
        const engine = new SyncEngine(config);
        engine.anytype = await AnytypeAdapter.create();
        return engine;
    }

    /** 检查各适配器可用性 */
    async checkStatus(): Promise<{
        anytype: AdapterStatus;
        reminders: { available: boolean; error?: string };
        store: { path: string; objects: number; lastSync?: string };
    }> {
        const anytypeStatus = await this.anytype.healthCheck();
        const remindersStatus = await this.reminders.ensureReady();
        const state = this.store.getSyncState();

        return {
            anytype: anytypeStatus,
            reminders: {
                available: remindersStatus.available,
                error: remindersStatus.error,
            },
            store: {
                path: "~/.conex/sync.db",
                objects: this.store.countIdMaps(),
                lastSync: state.last_poll_at || undefined,
            },
        };
    }

    /** 执行一次完整的同步 */
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
            // 1. 确保 Reminders 列表就绪
            if (this.config.ensureRemindersList) {
                const ready = await this.reminders.ensureReady();
                if (!ready.available) {
                    throw new Error(`Reminders 不可用: ${ready.error}`);
                }
            }

            // 2. 获取上次同步时间
            const state = this.store.getSyncState();
            const lastSync = state.last_poll_at;

            // 3. 从 Anytype 查询变更的任务
            const tasks = await this.anytype.queryObjects({
                type: "Task",
                spaceId: this.config.spaceId,
                lastSync: lastSync || undefined,
                limit: this.config.batchSize,
                sort: "lastModifiedDesc",
            });

            // 4. 逐个处理
            for (const task of tasks) {
                try {
                    const action = await this.syncOneTask(task);
                    if (action === "created") result.created += 1;
                    else if (action === "updated") result.updated += 1;
                    else result.skipped += 1;
                } catch (e) {
                    const msg = e instanceof Error ? e.message : String(e);
                    result.errors.push(`[${task.id}] ${task.name}: ${msg}`);
                }
            }

            // 5. 更新同步状态
            this.store.updateSyncState({
                last_poll_at: new Date().toISOString(),
                total_synced: state.total_synced + tasks.length,
                last_error: result.errors.length > 0
                    ? result.errors.join("; ").slice(0, 500)
                    : null,
            });

            result.success = true;
            result.duration = Date.now() - start;
        } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            result.errors.push(msg);
            result.duration = Date.now() - start;

            this.store.updateSyncState({
                last_error: msg.slice(0, 500),
            });
        }

        return result;
    }

    /** 同步单个任务，返回操作类型 */
    private async syncOneTask(task: AnytypeObject): Promise<"created" | "updated" | "skipped"> {
        // 1. 查找 ID 映射
        const idMap = this.store.getIdMap(task.id);

        // 2. 计算当前内容的哈希
        const currentHash = computeContentHash(task);

        if (!idMap) {
            // ── 新对象: 创建 Apple Reminder ──
            const reminderData = this.mapper.toTarget(task);
            const appleId = await this.reminders.createReminder(reminderData);

            this.store.upsertIdMap({
                anytype_id: task.id,
                apple_id: appleId,
                apple_type: "reminder",
                direction: "forward",
                last_hash: currentHash,
            });
            return "created";
        } else {
            // ── 已有映射: 检查是否需要更新 ──

            // 哈希相同 → 内容无变化，跳过
            if (idMap.last_hash === currentHash) {
                return "skipped";
            }

            const reminderData = this.mapper.toTarget(task);

            // 3. 冲突检测
            const existingReminder = await this.reminders.getReminder(idMap.apple_id);

            if (existingReminder) {
                const conflicts = this.mapper.detectConflict(task, existingReminder);

                if (conflicts.length > 0) {
                    // 内容有差异: 记录冲突，使用 Anytype 版本
                    this.store.logConflict({
                        anytype_id: task.id,
                        apple_id: idMap.apple_id,
                        conflict_type: "content_mismatch",
                    });
                }

                // 任何情况下都推送 Anytype 版本到 Apple
                await this.reminders.updateReminder(idMap.apple_id, reminderData);
                this.store.upsertIdMap({
                    ...idMap,
                    last_hash: currentHash,
                });
                return "updated";
            } else {
                // Apple 侧提醒已被删除 → 重建
                const newAppleId = await this.reminders.createReminder(reminderData);
                this.store.upsertIdMap({
                    ...idMap,
                    apple_id: newAppleId,
                    last_hash: currentHash,
                });
                this.store.logConflict({
                    anytype_id: task.id,
                    apple_id: idMap.apple_id,
                    conflict_type: "deleted_on_one_side",
                });
                return "created";
            }
        }
    }
}