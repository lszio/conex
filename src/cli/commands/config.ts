// CLI config 命令 — 查看和修改 CONEX 配置

import { Command } from "commander";
import { getSyncStore } from "../store/db.js";
import { hasApiKey, saveCredentials } from "../adapters/anytype/auth.js";

export const configCommand = new Command("config")
    .description("查看和修改 CONEX 配置")
    .addCommand(
        new Command("show")
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
            }),
    )
    .addCommand(
        new Command("set")
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
            }),
    )
    .addCommand(
        new Command("delete")
            .description("删除配置项")
            .argument("<key>", "配置键")
            .action((key: string) => {
                const store = getSyncStore();
                store.deleteConfig(key);
                console.log(`✅ 已删除: ${key}`);
                process.exit(0);
            }),
    );