// CONEX CLI 主入口
// 使用: bun run src/cli/index.ts sync|status|config|history

import { Command } from "commander";
import { syncCommand } from "./commands/sync.js";
import { statusCommand } from "./commands/status.js";
import { configCommand } from "./commands/config.js";
import { historyCommand } from "./commands/history.js";
import { daemonCommand } from "./commands/daemon.js";
import { readFileSync, existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

// 尝试读取 package.json 获取版本号
let version = "0.1.0";
const pkgPath = join(__dirname, "..", "..", "package.json");
if (existsSync(pkgPath)) {
    try {
        const pkg = JSON.parse(readFileSync(pkgPath, "utf-8"));
        version = pkg.version || version;
    } catch { /* ignore */ }
}

const program = new Command();

program
    .name("conex")
    .description("CONEX — Cross-Platform Nexus: Anytype ↔ Apple 同步桥接")
    .version(version);

program.addCommand(syncCommand);
program.addCommand(statusCommand);
program.addCommand(configCommand);
program.addCommand(historyCommand);
program.addCommand(daemonCommand);

// 默认显示帮助
if (process.argv.length <= 2) {
    program.outputHelp();
} else {
    program.parse();
}