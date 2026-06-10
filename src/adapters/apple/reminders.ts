// Apple Reminders 适配器 — 通过 AppleScript 操作 Reminders
// 参见 ARCHITECTURE.md §5.2: Phase 1 用 AppleScript, Phase 3 迁移到 Swift/EventKit

import { $ } from "bun";
import type { ReminderData, ReminderList, AppleRemindersStatus } from "./types.js";

const CONEX_LIST = "CONEX-Anytype";

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
    // AppleScript date: "Thursday, June 11, 2026 at 10:00:00 AM"
    const days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const months = ["January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December"];
    const day = days[d.getDay()];
    const month = months[d.getMonth()];
    const date = d.getDate();
    const year = d.getFullYear();
    const hours = d.getHours();
    const minutes = d.getMinutes();
    const seconds = d.getSeconds();
    const ampm = hours >= 12 ? "PM" : "AM";
    const h12 = hours % 12 || 12;

    return `date "${day}, ${month} ${date}, ${year} at ${h12}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")} ${ampm}"`;
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
            const result = await $`osascript -e ${script}`;
            return result.text().trim();
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
        const sinceStr = dateToAppleScript(since);
        try {
            const raw = await this.runScript(`
                tell application "Reminders"
                    set output to {}
                    set sc to reminders in list "${esc(this.listName)}" whose completed is true and completion date > ${sinceStr}
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
        const parts: string[] = [];
        parts.push(`set name of newReminder to "${esc(data.title)}"`);

        if (data.notes) {
            parts.push(`set body of newReminder to "${esc(data.notes)}"`);
        }

        if (data.dueDate) {
            parts.push(`set due date of newReminder to ${dateToAppleScript(data.dueDate)}`);
        }

        if (data.priority !== undefined && data.priority >= 0) {
            parts.push(`set priority of newReminder to ${data.priority}`);
        }

        const conexTag = data.conexId ? ` [conex:${esc(data.conexId)}]` : "";

        const script = `
            tell application "Reminders"
                set newReminder to make new reminder at list "${esc(this.listName)}"
                ${parts.join("\n                ")}
                set body of newReminder to (body of newReminder) & "${conexTag}"
                return id of newReminder
            end tell
        `;

        const id = await this.runScript(script);

        if (!id) {
            throw new AppleRemindersAdapterError("创建提醒后未能获取 ID");
        }

        return id;
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
        if (data.dueDate !== undefined) {
            parts.push(`set due date of targetReminder to ${dateToAppleScript(data.dueDate)}`);
        }
        if (data.priority !== undefined) {
            parts.push(`set priority of targetReminder to ${data.priority}`);
        }

        if (parts.length === 0) return;

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
                        set output to (name of r) & "|SEP|" & (body of r) & "|SEP|" & (completed of r as string)
                        return output
                    on error
                        return "NOT_FOUND"
                    end try
                end tell
            `);

            if (raw === "NOT_FOUND") return null;

            const [title, notes, completedStr] = raw.split("|SEP|", 3);
            return {
                title,
                id,
                notes: notes || undefined,
                isCompleted: completedStr === "true",
            };
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