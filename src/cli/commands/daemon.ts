// CLI daemon 命令 — 启动/停止守护进程
import { Command } from "commander";
import { BidirectionalDaemon } from "../../engine/daemon.js";

export const daemonCommand = new Command("daemon")
  .description("启动/管理双向同步守护进程")
  .addCommand(
    new Command("start")
      .description("启动守护进程（前台运行，Ctrl+C 停止）")
      .option("--space <id>", "Anytype 空间 ID")
      .option("--interval <ms>", "轮询间隔（毫秒）", "30000")
      .action(async (options) => {
        const spaceId = options.space || "";
        if (!spaceId) {
          console.error("❌ 需要 --space <id> 参数");
          process.exit(1);
        }

        const daemon = new BidirectionalDaemon({
          spaceId,
          pollIntervalMs: parseInt(options.interval, 10) || 30000,
        });

        console.log(`🔄 CONEX 双向同步守护进程`);
        console.log(`   空间: ${spaceId.slice(0, 24)}...`);
        console.log(`   轮询间隔: ${options.interval}ms`);
        console.log(`   按 Ctrl+C 停止\n`);

        daemon.start();

        // 保持进程运行
        process.on("SIGINT", () => {
          console.log("\n正在停止...");
          daemon.stop();
          process.exit(0);
        });

        process.on("SIGTERM", () => {
          daemon.stop();
          process.exit(0);
        });

        // 让进程保持活动
        await new Promise(() => {});
      }),
  );
