// CLI sync 命令 — 执行同步

import { Command } from "commander";
import { SyncEngine } from "../engine/sync-engine.js";

export const syncCommand = new Command("sync")
    .description("执行一次 Anytype → Apple 同步")
    .option("--space <id>", "限定同步到指定 Anytype 空间")
    .option("--batch-size <size>", "每批查询的对象数", "50")
    .action(async (options) => {
        const engine = new SyncEngine({
            spaceId: options.space,
            batchSize: parseInt(options.batchSize, 10) || 50,
        });

        console.log("🔄 CONEX 同步开始...\n");

        const result = await engine.sync();

        console.log("同步结果:");
        console.log(`  ✅ 状态:       ${result.success ? "成功" : "失败"}`);
        console.log(`  📝 新增:       ${result.created}`);
        console.log(`  🔄 更新:       ${result.updated}`);
        console.log(`  ⏭️  跳过:       ${result.skipped}`);
        console.log(`  ⚠️  冲突:       ${result.conflicts}`);
        console.log(`  ⏱  耗时:       ${result.duration}ms`);

        if (result.errors.length > 0) {
            console.log(`\n❌ 错误 (${result.errors.length}):`);
            for (const err of result.errors.slice(0, 10)) {
                console.log(`  • ${err}`);
            }
            if (result.errors.length > 10) {
                console.log(`  ...以及 ${result.errors.length - 10} 个更多错误`);
            }
        }

        process.exit(result.success ? 0 : 1);
    });