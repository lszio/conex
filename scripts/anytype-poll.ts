// Anytype API 查询脚本 — 验证连通性 (v4: 使用实际 API 端点)
// 使用: ANYTYPE_API_KEY=<key> bun run scripts/anytype-poll.ts
import { loadCredentials } from "../src/adapters/anytype/auth.js";

const creds = loadCredentials();
const API_BASE = creds.apiBaseUrl;
const headers = {
  Authorization: `Bearer ${creds.apiKey}`,
  "Anytype-Version": creds.apiVersion,
  "Content-Type": "application/json",
};

async function main() {
  console.log(`🔌 连接 Anytype API: ${API_BASE}`);
  console.log("──────────────────────────────────────");

  // 1. 列出空间
  console.log("\\n1️⃣  列出空间");
  try {
    const res = await fetch(`${API_BASE}/v1/spaces`, { headers });
    const data = await res.json() as any;
    const spaces = data.data || [];
    console.log(`   空间数: ${spaces.length}`);
    for (const space of spaces) {
      console.log(`   📁 ${space.name} (${space.id.slice(0, 20)}...)`);
    }

    // 2. 选择一个空间查询 Task
    const labry = spaces.find((s: any) => s.name === "labry");
    const targetSpace = labry || spaces[0];
    const spaceId = targetSpace.id;

    console.log(`\\n2️⃣  查询 Task 对象 (空间: ${targetSpace.name})`);
    const searchRes = await fetch(`${API_BASE}/v1/search`, {
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
      const priority = props.find((p: any) => p.key === "priority")?.select?.name || "";
      const status = props.find((p: any) => p.key === "status")?.select?.name || "";
      const icon = done ? "✅" : "⬜";
      console.log(`   ${icon} ${task.name || "(未命名)"}`);
      if (deadline) console.log(`       截止: ${deadline}`);
      if (status) console.log(`       状态: ${status}`);
      if (priority) console.log(`       优先级: ${priority}`);
    }

    // 3. 类型列表
    console.log(`\\n3️⃣  类型列表 (空间: ${targetSpace.name})`);
    const typesRes = await fetch(`${API_BASE}/v1/spaces/${spaceId}/types?limit=50`, { headers });
    const typesData = await typesRes.json() as any;
    const types = typesData.data || [];
    console.log(`   类型数: ${types.length}`);
    for (const t of types) {
      console.log(`   📄 ${t.name} (key: ${t.key})`);
    }

  } catch (e) {
    console.log(`   ❌ 连接失败: ${e}`);
    console.log("   确保 Anytype 桌面端正在运行 (端口 31009)");
    process.exit(1);
  }

  console.log("\\n──────────────────────────────────────");
  console.log("✅ 验证完成 — Anytype API 可用");
}

main().catch(console.error);
