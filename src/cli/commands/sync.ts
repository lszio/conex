// CLI sync 命令 — 执行双向同步

import { Command } from "commander";
import { SyncEngine } from "../../engine/sync-engine.js";
import { BidirectionalDaemon } from "../../engine/daemon.js";

export const syncCommand = new Command("sync")
    .description("执行一次双向同步 (Anytype ↔ Apple Reminders)")
    .option("--space <id>", "限定同步到指定 Anytype 空间")
    .option("--batch-size <size>", "每批查询的对象数", "50")
    .option("--no-bidi", "仅单向同步 (Anytype → Apple)")
    .action(async (options) => {
        const spaceId = options.space || "";

        // ── 方向 1: Anytype → Apple ──
        const engine = new SyncEngine({
            spaceId,
            batchSize: parseInt(options.batchSize, 10) || 50,
        });

        console.log("🔄 CONEX 双向同步开始...\n");

        const result = await engine.sync();

        console.log("方向 1: Anytype → Apple");
        console.log(`  📝 新增:       ${result.created}`);
        console.log(`  🔄 更新:       ${result.updated}`);
        console.log(`  ⏭️  跳过:       ${result.skipped}`);
        console.log(`  ⚠️  冲突:       ${result.conflicts}`);

        if (result.errors.length > 0) {
            console.log(`  ❌ 错误 (${result.errors.length}):`);
            for (const err of result.errors.slice(0, 5)) {
                console.log(`    • ${err}`);
            }
        }

        // ── 方向 2: Apple → Anytype (除非 --no-bidi) ──
        let appleToAnytypeCount = 0;
        if (options.bidi !== false) {
            console.log("\n方向 2: Apple → Anytype");
            try {
                const daemon = new BidirectionalDaemon({
                    spaceId,
                    changeWindowSec: 300,   // 往前看 5 分钟
                    appleChangeWindowSec: 300,
                });

                const bidiResult = await daemon.tick();
                appleToAnytypeCount = bidiResult.appleToAnytype.updated;
                if (appleToAnytypeCount > 0) {
                    console.log(`  ✅ 写回 Anytype 完成: ${appleToAnytypeCount}`);
                } else {
                    console.log(`  ⏭️  无变更`);
                }
            } catch (e: any) {
                console.log(`  ⚠️  Apple→Anytype 失败: ${e.message}`);
            }
        }

        const totalChanges = result.created + result.updated + appleToAnytypeCount;
        const success = result.success;

        console.log(`\n📊 汇总: ${totalChanges} 变更 (${result.duration}ms)`);
        console.log(`✅ 状态: ${success ? "成功" : "部分失败"}`);

        process.exit(success ? 0 : 1);
    });
