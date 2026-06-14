// Apple Reminders 适配器 — 通过 AppleScript 操作 Reminders
// 参见 ARCHITECTURE.md §5.2: Phase 1 用 AppleScript, Phase 3 迁移到 Swift/EventKit

import { execFile } from "child_process";
import { promisify } from "util";

const execFileAsync = promisify(execFile);
import type { ReminderData, ReminderList, AppleRemindersStatus } from "./types.js";

const CONEX_LIST = "CONEX-Anytype";

/** Swift 辅助二进制路径（EventKit 读写周期规则） */
const EK_RECURRENCE_BIN = new URL("ek_recurrence", import.meta.url).pathname;

/**
 * 安全转义字符串用于嵌入 AppleScript 字面量。
 * AppleScript 字符串使用双引号，需转义内部的 " 和 \。
 */
function esc(str: string): string {
    return str
        .replace(/\\/g, "\\\\")
        .replace(/"/g, '\\"')
        .replace(/\n/g, "\\n");
}

/** 将 Date 转为 AppleScript 的 date 字面量字符串 */
function dateToAppleScript(d: Date): string {
    // 不依赖 locale 的日期字符串，使用组件逐个设置
    const year = d.getFullYear();
    const month = d.getMonth() + 1;  // 1-based
    const day = d.getDate();
    const hours = d.getHours();
    const minutes = d.getMinutes();
    const seconds = d.getSeconds();
    return `(set _d to current date ¬
        set year of _d to ${year} ¬
        set month of _d to ${month} ¬
        set day of _d to ${day} ¬
        set hours of _d to ${hours} ¬
        set minutes of _d to ${minutes} ¬
        set seconds of _d to ${seconds} ¬
        _d)`;
}

export class AppleRemindersAdapterError extends Error {
    constructor(message: string, public appleScriptError?: string) {
        super(message);
        this.name = "AppleRemindersAdapterError";
    }
}

export class AppleRemindersAdapter {
    private listName: string;

    constructor(listName: string = CONEX_LIST) {
        this.listName = listName;
    }

    /** 执行 AppleScript 并返回 stdout */
    private async runScript(script: string): Promise<string> {
        try {
            const { stdout } = await execFileAsync("osascript", ["-e", script], {
                maxBuffer: 1024 * 1024, // 1MB
                timeout: 30_000,        // 30s
            });
            return stdout.trim();
        } catch (e: unknown) {
            const msg = e instanceof Error ? e.message : String(e);
            // AppleScript 错误通常是 stderr 中的内容
            throw new AppleRemindersAdapterError(
                `AppleScript 执行失败: ${msg}`,
                msg,
            );
        }
    }

    /** 检查 Reminders 可用性并创建 CONEX 列表 */
    async ensureReady(): Promise<AppleRemindersStatus> {
        try {
            // 检查 Reminders 是否在运行
            const running = await this.runScript(`
                tell application "System Events"
                    set isRunning to exists (processes where name is "Reminders")
                end tell
                if isRunning then return "running"
                return "not_running"
            `);

            if (running === "not_running") {
                await this.runScript(`tell application "Reminders" to activate`);
            }

            // 检查 CONEX 列表是否存在
            const lists = await this.runScript(`
                tell application "Reminders"
                    set listNames to name of every list
                    set AppleScript's text item delimiters to ", "
                    return listNames as string
                end tell
            `);

            if (!lists.includes(this.listName)) {
                await this.runScript(`
                    tell application "Reminders"
                        make new list with properties {name:"${esc(this.listName)}"}
                    end tell
                `);
            }

            return { available: true, remindersRunning: running === "running" };
        } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            return { available: false, remindersRunning: false, error: msg };
        }
    }

    /** 获取近期完成的提醒（用于增量判断） */
    async getRecentCompletions(since: Date): Promise<ReminderData[]> {
      const y = since.getFullYear();
      const m = since.getMonth() + 1;
      const d = since.getDate();
      const h = since.getHours();
      const min = since.getMinutes();
      const s = since.getSeconds();

      try {
        const raw = await this.runScript(`
              tell application "Reminders"
                  set _since to current date
                  set year of _since to ${y}
                  set month of _since to ${m}
                  set day of _since to ${d}
                  set hours of _since to ${h}
                  set minutes of _since to ${min}
                  set seconds of _since to ${s}
                  set output to {}
                  set sc to reminders in list "${esc(this.listName)}" whose completed is true and completion date > _since
                  repeat with r in sc
                      set end of output to name of r & "|SEP|" & id of r
                  end repeat
                  set AppleScript's text item delimiters to linefeed
                  return output as string
              end tell
          `);

            return raw
                .split("\n")
                .filter((l) => l.includes("|SEP|"))
                .map((l) => {
                    const [title, id] = l.split("|SEP|", 2);
                    return { title, id, isCompleted: true };
                });
        } catch {
            return [];
        }
    }

    /** 创建提醒 */
    async createReminder(data: ReminderData): Promise<string> {
        // 先创建提醒（不含截止日期，避免 locale 问题）
        const createParts: string[] = [];
        createParts.push(`set name of newReminder to "${esc(data.title)}"`);

        if (data.notes) {
            createParts.push(`set body of newReminder to "${esc(data.notes)}"`);
        }

        if (data.priority !== undefined && data.priority >= 0) {
            createParts.push(`set priority of newReminder to ${data.priority}`);
        }

        const createScript = `
            tell application "Reminders"
                set newReminder to make new reminder at list "${esc(this.listName)}"
                ${createParts.join("\n                ")}
                return id of newReminder
            end tell
        `;

        const id = await this.runScript(createScript);

        if (!id) {
            throw new AppleRemindersAdapterError("创建提醒后未能获取 ID");
        }

        // 单独设置截止日期（如果有）
        if (data.dueDate) {
            await this.setDueDate(id, data.dueDate);
        }

        // 单独设置闹钟（如果有）
        if (data.alarmDate) {
            await this.setAlarmDate(id, data.alarmDate);
        }

        return id;
    }

    /** 单独设置提醒的截止日期（避免 locale 问题） */
    private async setDueDate(id: string, dueDate: Date): Promise<void> {
        const y = dueDate.getFullYear();
        const m = dueDate.getMonth() + 1;
        const d = dueDate.getDate();
        const h = dueDate.getHours();
        const min = dueDate.getMinutes();
        const s = dueDate.getSeconds();

        await this.runScript(`
            tell application "Reminders"
                set targetReminder to reminder id "${esc(id)}"
                set dueDate to current date
                set year of dueDate to ${y}
                set month of dueDate to ${m}
                set day of dueDate to ${d}
                set hours of dueDate to ${h}
                set minutes of dueDate to ${min}
                set seconds of dueDate to ${s}
                set due date of targetReminder to dueDate
            end tell
        `);
    }

    /** 单独设置提醒的闹钟时间（remind me date） */
    private async setAlarmDate(id: string, alarmDate: Date): Promise<void> {
        const y = alarmDate.getFullYear();
        const m = alarmDate.getMonth() + 1;
        const d = alarmDate.getDate();
        const h = alarmDate.getHours();
        const min = alarmDate.getMinutes();

        await this.runScript(`
            tell application "Reminders"
                set targetReminder to reminder id "${esc(id)}"
                set _alarm to current date
                set year of _alarm to ${y}
                set month of _alarm to ${m}
                set day of _alarm to ${d}
                set hours of _alarm to ${h}
                set minutes of _alarm to ${min}
                set seconds of _alarm to 0
                set remind me date of targetReminder to _alarm
            end tell
        `);
    }

    /** 更新提醒 */
    async updateReminder(id: string, data: Partial<ReminderData>): Promise<void> {
        const parts: string[] = [];

        if (data.title !== undefined) {
            parts.push(`set name of targetReminder to "${esc(data.title)}"`);
        }
        if (data.notes !== undefined) {
            parts.push(`set body of targetReminder to "${esc(data.notes)}"`);
        }
        if (data.priority !== undefined) {
            parts.push(`set priority of targetReminder to ${data.priority}`);
        }

        if (parts.length > 0) {
            await this.runScript(`
                tell application "Reminders"
                    try
                        set targetReminder to reminder id "${esc(id)}"
                        ${parts.join("\n                    ")}
                    on error errMsg
                        return "ERROR: " & errMsg
                    end try
                end tell
            `);
        }

        // 单独设置截止日期（如果有）
        if (data.dueDate !== undefined) {
            await this.setDueDate(id, data.dueDate);
        }

        // 单独设置闹钟（如果有）
        if (data.alarmDate !== undefined) {
            await this.setAlarmDate(id, data.alarmDate);
        }
    }

    /** 完成提醒 */
    async completeReminder(id: string): Promise<void> {
        await this.runScript(`
            tell application "Reminders"
                try
                    set targetReminder to reminder id "${esc(id)}"
                    set completed of targetReminder to true
                end try
            end tell
        `);
    }

    /** 取消完成状态 */
    async uncompleteReminder(id: string): Promise<void> {
        await this.runScript(`
            tell application "Reminders"
                try
                    set targetReminder to reminder id "${esc(id)}"
                    set completed of targetReminder to false
                end try
            end tell
        `);
    }

    /** 列出 CONEX 列表中的所有未完成提醒 */
    async listReminders(list?: string): Promise<ReminderData[]> {
        const targetList = list || this.listName;
        const raw = await this.runScript(`
            tell application "Reminders"
                set output to {}
                set sc to reminders in list "${esc(targetList)}" whose completed is false
                repeat with r in sc
                    set end of output to (name of r) & "|SEP|" & (id of r) & "|SEP|" & (body of r) & "|SEP|" & (priority of r as string)
                end repeat
                set AppleScript's text item delimiters to linefeed
                return output as string
            end tell
        `);

        return raw
            .split("\n")
            .filter((l) => l.includes("|SEP|"))
            .map((l) => {
                const [title, id, notes, priorityStr] = l.split("|SEP|", 4);
                const data: ReminderData = { title, id, notes: notes || undefined };
                const p = parseInt(priorityStr, 10);
                if (!isNaN(p)) data.priority = p;
                return data;
            });
    }

    /** 获取单个提醒的详情 */
    async getReminder(id: string): Promise<ReminderData | null> {
        try {
            const raw = await this.runScript(`
                tell application "Reminders"
                    try
                        set r to reminder id "${esc(id)}"
                        set _dueY to ""
                        set _dueM to ""
                        set _dueD to ""
                        try
                            set _due to due date of r
                            set _dueY to year of _due as string
                            set _dueM to month of _due as integer
                            set _dueD to day of _due as string
                        end try
                        set _alarmY to ""
                        set _alarmM to ""
                        set _alarmD to ""
                        set _alarmH to ""
                        set _alarmMin to ""
                        try
                            set _alarm to remind me date of r
                            set _alarmY to year of _alarm as string
                            set _alarmM to month of _alarm as integer
                            set _alarmD to day of _alarm as string
                            set _alarmH to hours of _alarm as string
                            set _alarmMin to minutes of _alarm as string
                        end try
                        set output to ¬
                            (name of r) & "|SEP|" & ¬
                            (body of r) & "|SEP|" & ¬
                            (completed of r as string) & "|SEP|" & ¬
                            _dueY & "|SEP|" & _dueM & "|SEP|" & _dueD & "|SEP|" & ¬
                            (priority of r as string) & "|SEP|" & ¬
                            _alarmY & "|SEP|" & _alarmM & "|SEP|" & _alarmD & "|SEP|" & _alarmH & "|SEP|" & _alarmMin
                        return output
                    on error
                        return "NOT_FOUND"
                    end try
                end tell
            `);

            if (raw === "NOT_FOUND") return null;

            const parts = raw.split("|SEP|", 12);
            const [title, notes, completedStr, dueY, dueM, dueD, priorityStr, alarmY, alarmM, alarmD, alarmH, alarmMin] = parts;
            const result: ReminderData = {
                title,
                id,
                notes: notes || undefined,
                isCompleted: completedStr === "true",
            };

            if (dueY && dueM && dueD) {
                const y = parseInt(dueY, 10);
                const m = parseInt(dueM, 10) - 1;
                const d = parseInt(dueD, 10);
                if (!isNaN(y) && !isNaN(m) && !isNaN(d)) {
                    result.dueDate = new Date(y, m, d);
                }
            }

            const priorityVal = parseInt(priorityStr, 10);
            if (!isNaN(priorityVal)) {
                result.priority = priorityVal;
            }

            if (alarmY && alarmM && alarmD) {
                const y = parseInt(alarmY, 10);
                const m = parseInt(alarmM, 10) - 1;
                const d = parseInt(alarmD, 10);
                const h = alarmH ? parseInt(alarmH, 10) : 0;
                const min = alarmMin ? parseInt(alarmMin, 10) : 0;
                if (!isNaN(y) && !isNaN(m) && !isNaN(d)) {
                    result.alarmDate = new Date(y, m, d, h, min);
                }
            }

            return result;
        } catch {
            return null;
        }
    }

    /** 删除提醒 */
    async deleteReminder(id: string): Promise<void> {
        await this.runScript(`
            tell application "Reminders"
                try
                    delete reminder id "${esc(id)}"
                end try
            end tell
        `);
    }

    // ──────────────────────────────────────
    // 周期规则（通过 Swift/EventKit 二进制）
    // ──────────────────────────────────────

    /** 读取提醒的周期规则，返回 JSON 字符串或 "{}" */
    async getRecurrence(id: string): Promise<string> {
        try {
            const json = await execFileAsync(EK_RECURRENCE_BIN, ["read", id], {
                timeout: 10_000,
                maxBuffer: 16 * 1024,
            });
            const out = json.stdout.trim();
            return out === "{}" ? "" : out;
        } catch (e: unknown) {
            const msg = e instanceof Error ? e.message : String(e);
            console.error(`  ⚠️ getRecurrence 失败: ${msg}`);
            return "";
        }
    }

    /** 设置提醒的周期规则（JSON 字符串） */
    async setRecurrence(id: string, recurrenceJson: string): Promise<boolean> {
        try {
            const result = await execFileAsync(EK_RECURRENCE_BIN, ["write", id, recurrenceJson], {
                timeout: 10_000,
                maxBuffer: 16 * 1024,
            });
            return result.stdout.trim() === "OK";
        } catch (e: unknown) {
            const msg = e instanceof Error ? e.message : String(e);
            console.error(`  ⚠️ setRecurrence 失败: ${msg}`);
            return false;
        }
    }

    /** 移除提醒的周期规则 */
    async removeRecurrence(id: string): Promise<boolean> {
        try {
            const result = await execFileAsync(EK_RECURRENCE_BIN, ["remove", id], {
                timeout: 10_000,
                maxBuffer: 16 * 1024,
            });
            return result.stdout.trim() === "OK";
        } catch (e: unknown) {
            const msg = e instanceof Error ? e.message : String(e);
            console.error(`  ⚠️ removeRecurrence 失败: ${msg}`);
            return false;
        }
    }

    /** 获取所有列表及其提醒数 */
    async listAllLists(): Promise<ReminderList[]> {
        const raw = await this.runScript(`
            tell application "Reminders"
                set output to {}
                repeat with lst in every list
                    set end of output to (name of lst) & "|SEP|" & (count of reminders in lst)
                end repeat
                set AppleScript's text item delimiters to linefeed
                return output as string
            end tell
        `);

        return raw
            .split("\n")
            .filter((l) => l.includes("|SEP|"))
            .map((l) => {
                const [name, countStr] = l.split("|SEP|", 2);
                return { name, count: parseInt(countStr, 10) || 0 };
            });
    }
}