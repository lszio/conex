// CONEX CLI 入口 — 所有子命令内联以兼容 bun build 相对路径解析
import { Command } from "commander";
import { readFileSync, existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";
import { SyncEngine } from "../engine/sync-engine.ts";
import { BidirectionalDaemon } from "../engine/daemon.ts";
import { getCanonicalStore } from "../store/canonical-store.ts";
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

// --- sync (bidirectional) ---
program
  .command("sync")
  .description("执行一次双向同步 (Anytype ↔ Apple Reminders)")
  .option("--space <id>", "限定同步到指定 Anytype 空间")
  .option("--batch-size <size>", "每批查询的对象数", "50")
  .option("--no-bidi", "仅单向同步 (Anytype → Apple)")
  .action(async (options) => {
    const spaceId = options.space || "";
    const engine = await SyncEngine.create({
      spaceId,
      batchSize: parseInt(options.batchSize, 10) || 50,
    });

    console.log("🔄 CONEX 双向同步开始...\n");

    // 方向1: Anytype → Apple
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

    // 方向2: Apple → Anytype (除非 --no-bidi)
    let appleToAnytypeCount = 0;
    if (options.bidi !== false) {
      console.log("\n方向 2: Apple → Anytype");
      try {
        const daemon = new BidirectionalDaemon({
          spaceId,
          changeWindowSec: 300,
          appleChangeWindowSec: 300,
        });
        const bidiResult = await daemon.tick();
        appleToAnytypeCount = bidiResult.appleToCanonical.updated + bidiResult.pushToAnytype + bidiResult.pushToApple;
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

// --- status ---
program
  .command("status")
  .description("查看 CONEX 各适配器状态和同步概览")
  .action(async () => {
    const engine = await SyncEngine.create();
    const store = getCanonicalStore();

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

    console.log("\n💾 Canonical Store:");
    const all = store.list({ limit: 1000 });
    const completed = all.filter((t) => t.is_completed);
    const pending = all.filter((t) => !t.is_completed);
    const synced = all.filter((t) => t.last_synced);
    const pendingSync = all.filter((t) => !t.last_synced && t.anytype_id);
    console.log(`   总任务:  ${all.length}`);
    console.log(`   已完成:  ${completed.length}`);
    console.log(`   待办:    ${pending.length}`);
    console.log(`   已同步:  ${synced.length}`);
    console.log(`   待推送:  ${pendingSync.length}`);

    // 按来源统计
    const bySource: Record<string, number> = {};
    for (const t of all) {
      bySource[t.source] = (bySource[t.source] || 0) + 1;
    }
    for (const [src, count] of Object.entries(bySource)) {
      console.log(`   来源[${src}]: ${count}`);
    }

    // 优先级分布
    const priorityLabels = ["Q4(无)", "Q2(计划)", "Q3(委托)", "Q1(执行)"];
    const byPriority: Record<number, number> = {};
    for (const t of all) {
      byPriority[t.priority] = (byPriority[t.priority] || 0) + 1;
    }
    for (let p = 0; p <= 3; p++) {
      if (byPriority[p]) {
        console.log(`   优先级[${priorityLabels[p]}]: ${byPriority[p]}`);
      }
    }

    process.exit(0);
  });

// --- config ---
const configCmd = program.command("config").description("查看和修改 CONEX 配置");

configCmd
  .command("show")
  .description("显示当前配置")
  .action(async () => {
    const store = getSyncStore();
    const config = store.listConfig();

    console.log("🔧 CONEX 配置\n");

    const keySet = await hasApiKey();
    const keys: Record<string, string> = {
      "anytype.api_key_set": keySet ? "✅ 已配置" : "❌ 未配置",
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
  .action(async (key: string, value: string) => {
    if (key === "anytype.api_key") {
      await saveCredentials({ apiKey: value });
      console.log("✅ Anytype API Key 已保存到 ~/.conex/config.yaml");
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
