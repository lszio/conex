// Anytype REST API 连通性测试脚本
// 使用方法: ANYTYPE_API_KEY=*** bun run scripts/anytype-poll.ts

import { AnytypeAdapter } from "../src/adapters/anytype/adapter";
import { loadCredentials } from "../src/adapters/anytype/auth";

async function main() {
  const creds = await loadCredentials();
  console.log("CONEX - Anytype API 连通性测试");
  console.log("========================================");
  console.log("  API:", creds.apiBaseUrl);
  console.log("  Key:", creds.apiKey ? creds.apiKey.slice(0, 8) + "..." + creds.apiKey.slice(-4) : "NOT SET");
  console.log("");

  if (!creds.apiKey) {
    console.error("需要配置 API Key");
    console.error("  bun run src/cli/index.ts config set anytype.api_key <你的key>");
    console.error("  或设置环境变量: ANYTYPE_API_KEY=***");
    process.exit(1);
  }

  const adapter = await AnytypeAdapter.create();

  // 1. 列出空间
  console.log("1. 列出空间");
  const spaces = await adapter.listSpaces();
  console.log("  空间数:", spaces.length);
  for (const s of spaces) {
    console.log("  -", s.name, "(", s.id, ")");
  }
  console.log("");

  if (spaces.length === 0) {
    console.log("没有空间");
    process.exit(0);
  }

  const spaceId = spaces[0].id;

  // 2. 查询对象
  console.log("2. 查询对象 (无过滤, limit: 3)");
  const objs = await adapter.queryObjects({ spaceId, limit: 3 });
  console.log("  对象数:", objs.length);
  for (const o of objs) {
    const typeKey = o.type ? (o.type.key || o.type.name || "?") : "?";
    console.log("  -", o.name || "(无标题)", "(type:", typeKey, ")");
  }
  console.log("");

  // 3. 按 type 过滤
  console.log("3. 查询 type=note (limit: 3)");
  const notes = await adapter.queryObjects({ spaceId, limit: 3, type: "note" });
  console.log("  对象数:", notes.length);
  for (const o of notes) {
    console.log("  -", o.name || "(无标题)");
  }
  console.log("");

  // 4. 列出类型
  console.log("4. 列出类型 (limit: 20)");
  const types = await adapter.listTypes(spaceId);
  console.log("  类型数:", types.length);
  for (const t of types.slice(0, 30)) {
    console.log("  - key:", t.key || "?", "name:", t.name || "?");
  }
  console.log("");

  // 5. 空间内搜索
  console.log("5. 空间内搜索 (query=test, limit: 3)");
  const results = await adapter.search("test", 3);
  console.log("  结果数:", results.length);
  for (const r of results) {
    console.log("  -", r.name || "(无标题)");
  }

  console.log("");
  console.log("========================================");
  console.log("验证完成");
}

main().catch(console.error);