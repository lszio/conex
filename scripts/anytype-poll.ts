// Anytype API 查询脚本 — 验证连通性
// 使用: ANYTYPE_API_KEY=<key> bun run scripts/anytype-poll.ts
// 必须先创建 API Key: 打开 Anytype → 设置 → API Keys → Create new

const API_BASE = process.env.ANYTYPE_API_BASE_URL || "http://127.0.0.1:31009";
const API_KEY = process.env.ANYTYPE_API_KEY;

if (!API_KEY) {
  console.error("❌ 需要 ANYTYPE_API_KEY 环境变量");
  console.error("   创建 API Key: Anytype → 设置 → API Keys → Create new");
  console.error("   使用: ANYTYPE_API_KEY=<key> bun run scripts/anytype-poll.ts");
  process.exit(1);
}

const headers = {
  Authorization: `Bearer ${API_KEY}`,
  "Anytype-Version": "2025-11-08",
  "Content-Type": "application/json",
};

async function main() {
  console.log(`🔌 连接 Anytype API: ${API_BASE}`);
  console.log("──────────────────────────────────────");

  // 1. 健康检查
  console.log("\n1️⃣  健康检查");
  try {
    const res = await fetch(`${API_BASE}/api/v1/health`, { headers });
    const text = await res.text();
    console.log(`   状态: ${res.status} — ${text.slice(0, 200)}`);
  } catch (e) {
    console.log(`   ❌ 连接失败: ${e}`);
    console.log("   确保 Anytype 桌面端正在运行");
    process.exit(1);
  }

  // 2. 列出空间
  console.log("\n2️⃣  列出空间");
  try {
    const res = await fetch(`${API_BASE}/api/v1/spaces`, { headers });
    const data = await res.json() as any;
    console.log(`   空间数: ${data.spaces?.length || 0}`);
    for (const space of data.spaces || []) {
      console.log(`   - ${space.name} (${space.id})`);
    }
  } catch (e) {
    console.log(`   ⚠️  查询空间失败: ${e}`);
  }

  // 3. 查询对象 (限制 5 条)
  console.log("\n3️⃣  查询最近对象 (limit: 5)");
  try {
    const res = await fetch(`${API_BASE}/api/v1/objects?limit=5&sort=lastModifiedDesc`, { headers });
    const data = await res.json() as any;
    const objects = data.objects || data.data || [];
    console.log(`   对象数: ${objects.length}`);
    for (const obj of objects.slice(0, 5)) {
      console.log(`   📄 ${obj.name || "(未命名)"} (type: ${obj.type || obj.layout || "?"})`);
    }
  } catch (e) {
    console.log(`   ⚠️  查询对象失败: ${e}`);
  }

  // 4. 查询 Task 类型对象
  console.log("\n4️⃣  查询 Task 类型对象 (limit: 5)");
  try {
    const res = await fetch(`${API_BASE}/api/v1/objects?type=Task&limit=5`, { headers });
    const data = await res.json() as any;
    const tasks = data.objects || data.data || [];
    console.log(`   任务数: ${tasks.length}`);
    for (const task of tasks.slice(0, 5)) {
      console.log(`   ✅ ${task.name} (状态: ${task.done ? "完成" : "待办"})`);
      if (task.deadline) console.log(`      截止: ${task.deadline}`);
    }
  } catch (e) {
    console.log(`   ⚠️  查询 Task 失败: ${e}`);
  }

  console.log("\n──────────────────────────────────────");
  console.log("✅ 验证完成");
}

main().catch(console.error);