// CLI history 命令 — 查看同步历史

import { Command } from "commander";
import { getSyncStore } from "../store/db.js";

export const historyCommand = new Command("history")
    .description("查看同步历史和冲突日志")
    .option("-n, --limit <数>", "显示条数", "20")
    .action((options) => {
        const store = getSyncStore();
        const limit = parseInt(options.limit, 10) || 20;

        const conflicts = store.listConflicts(limit);

        console.log("📋 CONEX 同步历史\n");

        if (conflicts.length === 0) {
            console.log("  暂无冲突记录。");
        } else {
            console.log(`  冲突日志 (最近 ${conflicts.length} 条):`);
            console.log("");
            for (const c of conflicts) {
                const status = c.resolution
                    ? `✅ 已解决 (${c.resolution})`
                    : "⏳ 未解决";
                console.log(`  [${c.created_at}] ${c.conflict_type}`);
                console.log(`    Anytype: ${c.anytype_id}`);
                console.log(`    Apple:   ${c.apple_id}`);
                console.log(`    状态:    ${status}`);
                console.log("");
            }
        }

        // 同步状态
        const state = store.getSyncState();
        console.log("  同步统计:");
        console.log(`    上次轮询:  ${state.last_poll_at}`);
        console.log(`    累计同步:  ${state.total_synced} 个对象`);
        if (state.last_error) {
            console.log(`    上次错误:  ${state.last_error}`);
        }

        process.exit(0);
    });