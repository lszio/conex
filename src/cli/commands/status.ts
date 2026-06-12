// CLI status 命令 — 查看各适配器状态和同步概览

import { Command } from "commander";
import { SyncEngine } from "../../engine/sync-engine.js";
import { getSyncStore } from "../../store/db.js";

export const statusCommand = new Command("status")
    .description("查看 CONEX 各适配器状态和同步概览")
    .action(async () => {
        const engine = new SyncEngine();
        const store = getSyncStore();

        console.log("🔍 CONEX 状态\n");

        // 适配器状态
        const status = await engine.checkStatus();

        console.log("📡 Anytype API:");
        console.log(`   连接:   ${status.anytype.connected ? "✅ 已连接" : "❌ 未连接"}`);
        if (status.anytype.error) {
            console.log(`   错误:   ${status.anytype.error}`);
        }

        console.log("\n🍎 Apple Reminders:");
        console.log(`   可用:   ${status.reminders.available ? "✅ 可用" : "❌ 不可用"}`);
        if (status.reminders.error) {
            console.log(`   错误:   ${status.reminders.error}`);
        }

        console.log("\n💾 本地存储:");
        console.log(`   位置:   ${status.store.path}`);
        console.log(`   映射数: ${status.store.objects} 个对象`);
        console.log(`   上次同步: ${status.store.lastSync || "从未"}`);

        // 同步状态
        const state = store.getSyncState();
        console.log(`   累计同步: ${state.total_synced} 个对象`);
        if (state.last_error) {
            console.log(`   上次错误: ${state.last_error}`);
        }

        // 冲突
        const conflicts = store.listConflicts(5);
        if (conflicts.length > 0) {
            console.log(`\n⚠️  未解决的冲突: ${conflicts.length}`);
            for (const c of conflicts.slice(0, 3)) {
                if (!c.resolution) {
                    console.log(`   • ${c.anytype_id} ↔ ${c.apple_id} (${c.conflict_type})`);
                }
            }
        } else {
            console.log("\n⚠️  冲突日志: 无");
        }

        process.exit(0);
    });