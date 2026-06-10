// Apple Reminders 测试脚本 — 验证 AppleScript 连通性
// 使用: bun run scripts/apple-test.ts

import { $ } from "bun";

async function runAppleScript(code: string): Promise<string> {
  const result = await $`osascript -e ${code}`.text();
  return result.trim();
}

async function main() {
  console.log("🍎 Apple Reminders 连通性验证");
  console.log("──────────────────────────────────────");

  // 1. 验证 Reminders 应用存在且可访问
  console.log("\n1️⃣  验证 Reminders 应用");
  try {
    const version = await runAppleScript(`
      tell application "System Events"
        set appName to name of first application process whose name is "Reminders"
        return appName
      end tell
    `);
    console.log(`   ✅ Reminders 正在运行: ${version}`);
  } catch {
    console.log(`   ⚠️  Reminders 未运行，尝试启动...`);
    try {
      await runAppleScript(`tell application "Reminders" to activate`);
      console.log(`   ✅ Reminders 已启动`);
    } catch (e) {
      console.log(`   ❌ 无法启动 Reminders: ${e}`);
      process.exit(1);
    }
  }

  // 2. 检查 CONEX 列表是否存在，不存在则创建
  console.log("\n2️⃣  检查 CONEX-Anytype 列表");
  let listExists = false;
  try {
    const lists = await runAppleScript(`
      tell application "Reminders"
        set listNames to name of every list
        set AppleScript's text item delimiters to ", "
        return listNames as string
      end tell
    `);
    console.log(`   现有列表: ${lists}`);

    if (lists.includes("CONEX-Anytype")) {
      listExists = true;
      console.log(`   ✅ CONEX-Anytype 列表已存在`);
    }
  } catch (e) {
    console.log(`   ⚠️  查询列表失败: ${e}`);
  }

  if (!listExists) {
    try {
      await runAppleScript(`
        tell application "Reminders"
          make new list with properties {name:"CONEX-Anytype"}
        end tell
      `);
      console.log(`   ✅ 已创建 CONEX-Anytype 列表`);
    } catch (e) {
      console.log(`   ⚠️  创建列表失败: ${e}`);
    }
  }

  // 3. 创建测试提醒
  console.log("\n3️⃣  创建测试提醒");
  const testTitle = `CONEX 测试任务 ${new Date().toISOString().slice(0, 16)}`;
  try {
    await runAppleScript(`
      tell application "Reminders"
        set newReminder to make new reminder
        set name of newReminder to "${testTitle}"
        set body of newReminder to "由 CONEX 同步测试创建"
        set due date of newReminder to (current date) + (24 * 60 * 60)
        set priority of newReminder to 1
      end tell
    `);
    console.log(`   ✅ 已创建: ${testTitle}`);
  } catch (e) {
    console.log(`   ❌ 创建失败: ${e}`);
  }

  // 4. 读取提醒列表
  console.log("\n4️⃣  读取今日提醒");
  try {
    const reminders = await runAppleScript(`
      tell application "Reminders"
        set resultList to {}
        set today to current date
        set time of today to 0
        
        repeat with r in reminders whose completed is false
          set end of resultList to name of r
        end repeat
        
        set AppleScript's text item delimiters to linefeed
        return resultList as string
      end tell
    `);
    const lines = reminders.split("\n").filter((l) => l.length > 0);
    console.log(`   待办项: ${lines.length}`);
    for (const line of lines.slice(0, 10)) {
      console.log(`   📋 ${line}`);
    }
  } catch (e) {
    console.log(`   ⚠️  读取失败: ${e}`);
  }

  // 5. 清理测试提醒
  console.log("\n5️⃣  清理测试提醒");
  try {
    await runAppleScript(`
      tell application "Reminders"
        set testReminders to reminders whose name starts with "CONEX 测试任务"
        repeat with r in testReminders
          delete r
        end repeat
      end tell
    `);
    console.log(`   ✅ 测试提醒已清理`);
  } catch (e) {
    console.log(`   ⚠️  清理失败: ${e}`);
  }

  console.log("\n──────────────────────────────────────");
  console.log("✅ Apple Reminders 验证完成");
}

main().catch(console.error);