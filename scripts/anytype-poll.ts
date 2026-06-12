// Anytype REST API 连通性测试脚本 (v4: 使用实际 API 端点 + 适配器)
// 使用方法: ANYTYPE_API_KEY=*** bun run scripts/anytype-poll.ts

import { loadCredentials } from "../src/adapters/anytype/auth.js";

async function main() {
  const creds = await loadCredentials();
  console.log("🔌 连接 Anytype API:", creds.apiBaseUrl);
  console.log("──────────────────────────────────────");

  if (!creds.apiKey) {
    console.error("❌ 需要配置 API Key");
    process.exit(1);
  }

  const headers = {
    Authorization: `Bearer ${creds.apiKey}`,
    "Anytype-Version": creds.apiVersion,
    "Content-Type": "application/json",
  };

  // 1. 列出空间
  console.log("\n1️⃣  列出空间");
  const spacesRes = await fetch(`${creds.apiBaseUrl}/v1/spaces`, { headers });
  const spacesData = await spacesRes.json() as any;
  const spaces = spacesData.data || [];
  console.log(`   空间数: ${spaces.length}`);
  for (const s of spaces) {
    console.log(`   📁 ${s.name} (${s.id.slice(0, 20)}...)`);
  }

  if (spaces.length === 0) {
    console.log("   没有空间");
    process.exit(0);
  }

  // 2. 选择 labry 空间查询 Task
  const labry = spaces.find((s: any) => s.name === "labry") || spaces[0];
  const spaceId = labry.id;

  console.log(`\n2️⃣  查询 Task 对象 (空间: ${labry.name})`);
  const searchRes = await fetch(`${creds.apiBaseUrl}/v1/search`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      types: ["task"],
      limit: 20,
      sort: { direction: "desc", property_key: "last_modified_date" },
    }),
  });
  const searchData = await searchRes.json() as any;
  const tasks = searchData.data || [];
  console.log(`   任务数: ${tasks.length} (展示前 20 个)`);

  for (const task of tasks.slice(0, 20)) {
    const props = (task.properties || []) as any[];
    const done = props.find((p: any) => p.key === "done")?.checkbox || false;
    const deadline = props.find((p: any) => p.key === "due_date")?.date || "";
    const status = props.find((p: any) => p.key === "status")?.select?.name || "";
    const icon = status === "DONE" || done ? "✅" : "⬜";
    console.log(`   ${icon} ${task.name || "(未命名)"}`);
    if (deadline) console.log(`       截止: ${deadline}`);
    if (status) console.log(`       状态: ${status}`);
  }

  // 3. 类型列表
  console.log(`\n3️⃣  类型列表 (空间: ${labry.name})`);
  const typesRes = await fetch(`${creds.apiBaseUrl}/v1/spaces/${spaceId}/types?limit=50`, { headers });
  const typesData = await typesRes.json() as any;
  const types = typesData.data || [];
  console.log(`   类型数: ${types.length}`);
  for (const t of types) {
    console.log(`   📄 ${t.name} (key: ${t.key})`);
  }

  console.log("\n──────────────────────────────────────");
  console.log("✅ 验证完成 — Anytype API 可用");
}

main().catch(console.error);
