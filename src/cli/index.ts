// CONEX CLI 入口 — 所有子命令内联以兼容 bun build 相对路径解析
import { Command } from "commander";
import { readFileSync, existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";
import { SyncEngine } from "../engine/sync-engine.ts";
import { getSyncStore } from "../store/db.ts";
import { hasApiKey, saveCredentials } from "../adapters/anytype/auth.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

let version = "0.1.0";
const pkgPath = join(__dirname, "..", "..", "package.json");
if (existsSync(pkgPath)) {
  try {
    const pkg = JSON.parse(readFileSync(pkgPath, "utf-8"));
    version = pkg.version || version;
  } catch {
    /* ignore */
  }
}

const program = new Command();

program.name("conex").description("CONEX — Cross-Platform Nexus: Anytype ↔ Apple 同步桥接").version(version);

// --- sync ---
program
  .command("sync")
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

// --- status ---
program
  .command("status")
  .description("查看 CONEX 各适配器状态和同步概览")
  .action(async () => {
    const engine = new SyncEngine();
    const store = getSyncStore();

    console.log("🔍 CONEX 状态\n");

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

    const state = store.getSyncState();
    console.log(`   累计同步: ${state.total_synced} 个对象`);
    if (state.last_error) {
      console.log(`   上次错误: ${state.last_error}`);
    }

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

// --- config ---
const configCmd = program.command("config").description("查看和修改 CONEX 配置");

configCmd
  .command("show")
  .description("显示当前配置")
  .action(() => {
    const store = getSyncStore();
    const config = store.listConfig();

    console.log("🔧 CONEX 配置\n");

    const keys: Record<string, string> = {
      "anytype.api_key_set": hasApiKey() ? "✅ 已配置" : "❌ 未配置",
    };

    for (const row of config) {
      keys[row.key] = row.value;
    }

    const maxLen = Math.max(...Object.keys(keys).map((k) => k.length));
    for (const [key, value] of Object.entries(keys)) {
      console.log(`  ${key.padEnd(maxLen)}  = ${value}`);
    }

    process.exit(0);
  });

configCmd
  .command("set")
  .description("设置配置值")
  .argument("<key>", "配置键")
  .argument("<value>", "配置值")
  .action((key: string, value: string) => {
    if (key === "anytype.api_key") {
      saveCredentials({ apiKey: value });
      console.log("✅ Anytype API Key 已保存到 ~/.conex/credentials.json");
      process.exit(0);
    }

    const store = getSyncStore();
    store.setConfig(key, value);
    console.log(`✅ ${key} = ${value}`);
    process.exit(0);
  });

configCmd
  .command("delete")
  .description("删除配置项")
  .argument("<key>", "配置键")
  .action((key: string) => {
    const store = getSyncStore();
    store.deleteConfig(key);
    console.log(`✅ 已删除: ${key}`);
    process.exit(0);
  });

// --- history ---
program
  .command("history")
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
        const status = c.resolution ? `✅ 已解决 (${c.resolution})` : "⏳ 未解决";
        console.log(`  [${c.created_at}] ${c.conflict_type}`);
        console.log(`    Anytype: ${c.anytype_id}`);
        console.log(`    Apple:   ${c.apple_id}`);
        console.log(`    状态:    ${status}`);
        console.log("");
      }
    }

    const state = store.getSyncState();
    console.log("  同步统计:");
    console.log(`    上次轮询:  ${state.last_poll_at}`);
    console.log(`    累计同步:  ${state.total_synced} 个对象`);
    if (state.last_error) {
      console.log(`    上次错误:  ${state.last_error}`);
    }

    process.exit(0);
  });

// --- bootstrap ---
if (process.argv.length <= 2) {
  program.outputHelp();
} else {
  program.parse();
}