// 检查 Anytype Task 类型的所有实际字段/属性
import { loadCredentials } from "../src/adapters/anytype/auth.js";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnytypeProp = Record<string, any>;

async function main() {
  const creds = await loadCredentials();
  console.log("=== 1. 查询 Task 类型定义 ===");
  
  // 先获取类型列表，找到 Task 类型的 ID
  const spacesRes = await fetch(new URL("/v1/spaces", creds.apiBaseUrl).toString(), {
    headers: {
      Authorization: `Bearer ${creds.apiKey}`,
      "Anytype-Version": creds.apiVersion,
    },
  });
  const spacesData: any = await spacesRes.json();
  const spaces = spacesData.data || [];
  console.log(`  找到 ${spaces.length} 个空间`);
  
  for (const space of spaces) {
    console.log(`\n  空间: ${space.name} (${space.id.slice(0, 24)}...)`);
    
    // 获取该空间的类型列表
    const typesRes = await fetch(
      new URL(`/v1/spaces/${space.id}/types`, creds.apiBaseUrl).toString(),
      {
        headers: {
          Authorization: `Bearer ${creds.apiKey}`,
          "Anytype-Version": creds.apiVersion,
        },
      }
    );
    const typesData: any = await typesRes.json();
    const types = typesData.data || [];
    
    const taskType = types.find((t: any) => t.key === "task" || t.name?.toLowerCase() === "task");
    if (taskType) {
      console.log(`  📋 Task 类型: ${taskType.name} (key=${taskType.key}, id=${taskType.id.slice(0, 24)}...)`);
      
      // 如果类型对象本身带 properties，直接看
      if (taskType.properties && taskType.properties.length > 0) {
        console.log(`\n  === Task 属性列表 (来自类型定义) ===`);
        for (const prop of taskType.properties) {
          if (prop.object === "property") {
            console.log(`  [${prop.key}] ${prop.name}`);
            console.log(`    format: ${prop.format}`);
            console.log(`    id: ${prop.id.slice(0, 24)}...`);
            if (prop.format === "select" && prop.select) {
              console.log(`    select: ${JSON.stringify(prop.select)}`);
            }
            if (prop.format === "multi_select" && prop.multi_select) {
              console.log(`    multi_select: ${JSON.stringify(prop.multi_select).slice(0, 200)}`);
            }
            if (prop.format === "objects" && prop.objects) {
              console.log(`    objects: ${JSON.stringify(prop.objects)}`);
            }
          }
        }
      }
    }
    
    // 从 Property 类型中搜索所有属性定义
    const propSearchRes = await fetch(new URL("/v1/search", creds.apiBaseUrl).toString(), {
      method: "POST",
      headers: {
        Authorization: `Bearer ${creds.apiKey}`,
        "Anytype-Version": creds.apiVersion,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ types: ["property"], limit: 100 }),
    });
    const propData: any = await propSearchRes.json();
    const allProps = propData.data || [];
    
    // 找出 key 和 format
    const propDefs: Array<{ key: string; name: string; format: string; options?: unknown }> = [];
    for (const propObj of allProps) {
      const props = propObj.properties || [];
      const keyP = props.find((p: any) => p.key === "key");
      const nameP = props.find((p: any) => p.key === "name");
      const formatP = props.find((p: any) => p.key === "format");
      const optsP = props.find((p: any) => p.key === "options");
      
      propDefs.push({
        key: keyP?.text || "?",
        name: nameP?.text || "?",
        format: formatP?.select?.name || "?",
        options: optsP?.multi_select || optsP?.select || undefined,
      });
    }
    
    // 只显示关键字段（去重）
    const seen = new Set<string>();
    console.log(`\n  === 所有属性定义 (${propDefs.length} 个) ===`);
    for (const p of propDefs.sort((a, b) => a.key.localeCompare(b.key))) {
      if (seen.has(p.key)) continue;
      seen.add(p.key);
      console.log(`  [${p.key.padEnd(22)}] ${p.name.padEnd(18)} format=${p.format}`);
      if (p.options && Array.isArray(p.options) && p.options.length > 0) {
        const items = p.options as Array<{ name?: string; key?: string }>;
        console.log(`    options: ${items.map((o: any) => `${o.name || o.key || "?"}`).join(", ")}`);
      }
    }
  }
  
  // 2. 抓一批实际任务对象，看它们身上真正有哪些属性
  console.log(`\n\n=== 2. 实际任务对象属性采样 (取前3个) ===`);
  const taskSearchRes = await fetch(new URL("/v1/search", creds.apiBaseUrl).toString(), {
    method: "POST",
    headers: {
      Authorization: `Bearer ${creds.apiKey}`,
      "Anytype-Version": creds.apiVersion,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ types: ["task"], limit: 3 }),
  });
  const taskData: any = await taskSearchRes.json();
  const tasks = taskData.data || [];
  
  for (const obj of tasks) {
    console.log(`\n  任务: "${obj.name}" (id=${obj.id.slice(0, 24)}...)`);
    console.log(`  layout: ${obj.layout}, archived: ${obj.archived}, space_id: ${obj.space_id.slice(0, 16)}...`);
    
    const fields = obj.properties || [];
    console.log(`  属性数: ${fields.length}`);
    for (const f of fields) {
      const value = getPropValueSummary(f);
      console.log(`    [${f.key.padEnd(22)}] ${f.name.padEnd(18)} ${f.format.padEnd(12)} = ${value}`);
    }
  }
}

function getPropValueSummary(p: AnytypeProp): string {
  switch (p.format) {
    case "checkbox":
      return String(p.checkbox);
    case "date":
      return p.date ? p.date.slice(0, 19) : "(null)";
    case "select":
      return p.select ? `${p.select.name} (key=${p.select.key})` : "(none)";
    case "multi_select":
      return p.multi_select ? p.multi_select.map((t: any) => t.name).join(", ") : "(none)";
    case "text":
      return p.text ? `"${p.text.slice(0, 40)}${p.text.length > 40 ? "..." : ""}"` : "(empty)";
    case "number":
      return String(p.number);
    case "objects":
      return p.objects ? `[${p.objects.length} refs]` : "(none)";
    case "url":
      return p.text ? p.text.slice(0, 40) : "(none)";
    case "email":
      return p.text ? p.text : "(none)";
    case "phone":
      return p.text ? p.text : "(none)";
    default:
      return JSON.stringify(p).slice(0, 60);
  }
}

main().catch(console.error);
