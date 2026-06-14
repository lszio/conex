// CONEX Web UI — Hono HTTP 服务器
// 默认端口: 3101 (与 Anytype API 31009 不同，避免冲突)

import { Hono } from "hono";
import { getCanonicalStore } from "../store/canonical-store.js";
import { getSyncStore } from "../store/db.js";
import { SyncEngine } from "../engine/sync-engine.js";
import { canonicalToApplePriority, priorityToQuadrantLabel } from "../store/schema.js";
import { canonicalToReminder } from "../mappers/reminder-mapper.js";

const app = new Hono();

// ──────────────────────────────────────────
// Layout
// ──────────────────────────────────────────

function layout(title: string, body: string): string {
  return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${title} — CONEX</title>
  <script src="https://unpkg.com/htmx.org@2"></script>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #f5f5f7; color: #1d1d1f; }
    .container { max-width: 960px; margin: 0 auto; padding: 0 16px; }
    nav { background: #fff; border-bottom: 1px solid #e5e5ea; padding: 12px 0; }
    nav .container { display: flex; align-items: center; gap: 24px; }
    nav a { text-decoration: none; color: #555; font-size: 14px; }
    nav a:hover { color: #007aff; }
    nav .brand { font-weight: 600; color: #1d1d1f; font-size: 16px; }
    h1 { font-size: 24px; font-weight: 700; margin: 24px 0 16px; }
    h2 { font-size: 18px; font-weight: 600; margin: 20px 0 12px; }
    .card { background: #fff; border-radius: 12px; padding: 20px; margin-bottom: 16px; box-shadow: 0 1px 3px rgba(0,0,0,.08); }
    .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; }
    .stat { text-align: center; padding: 12px; }
    .stat-value { font-size: 28px; font-weight: 700; color: #007aff; }
    .stat-label { font-size: 12px; color: #8e8e93; margin-top: 4px; }
    table { width: 100%; border-collapse: collapse; font-size: 14px; }
    th, td { text-align: left; padding: 10px 8px; border-bottom: 1px solid #e5e5ea; }
    th { color: #8e8e93; font-weight: 500; font-size: 12px; text-transform: uppercase; }
    tr:hover td { background: #f5f5f7; }
    .btn { display: inline-block; padding: 8px 16px; border-radius: 8px; font-size: 14px; font-weight: 500; cursor: pointer; border: none; text-decoration: none; }
    .btn-primary { background: #007aff; color: #fff; }
    .btn-primary:hover { background: #0066d6; }
    .btn-secondary { background: #e5e5ea; color: #1d1d1f; }
    .btn-secondary:hover { background: #d1d1d6; }
    .badge { display: inline-block; padding: 2px 8px; border-radius: 20px; font-size: 11px; font-weight: 600; }
    .badge-green { background: #d1fae5; color: #065f46; }
    .badge-red { background: #fee2e2; color: #991b1b; }
    .badge-blue { background: #dbeafe; color: #1e40af; }
    .badge-gray { background: #f3f4f6; color: #6b7280; }
    .badge-yellow { background: #fef3c7; color: #92400e; }
    .badge-purple { background: #f3e8ff; color: #6b21a8; }
    .tag { display: inline-block; padding: 1px 6px; border-radius: 4px; font-size: 11px; background: #f0f0f5; margin: 1px; }
    .filters { display: flex; gap: 10px; margin-bottom: 16px; flex-wrap: wrap; }
    .filters select { padding: 6px 10px; border: 1px solid #d1d1d6; border-radius: 8px; font-size: 13px; background: #fff; }
    .filters input { padding: 6px 10px; border: 1px solid #d1d1d6; border-radius: 8px; font-size: 13px; }
    .priority-Q4 { color: #8e8e93; }
    .priority-Q2 { color: #007aff; }
    .priority-Q3 { color: #ff9500; }
    .priority-Q1 { color: #ff3b30; }
    .form-row { margin-bottom: 12px; }
    .form-row label { display: block; font-size: 12px; color: #8e8e93; margin-bottom: 4px; }
    .form-row input, .form-row select, .form-row textarea { width: 100%; padding: 8px 12px; border: 1px solid #d1d1d6; border-radius: 8px; font-size: 14px; }
    .form-row textarea { min-height: 80px; resize: vertical; }
    .actions { display: flex; gap: 8px; margin-top: 16px; }
    .empty { text-align: center; padding: 40px 20px; color: #8e8e93; }
    .flash { padding: 10px 16px; border-radius: 8px; margin-bottom: 12px; font-size: 14px; }
    .flash-success { background: #d1fae5; color: #065f46; }
    .flash-error { background: #fee2e2; color: #991b1b; }
    .detail-grid dt { font-size: 12px; color: #8e8e93; margin-top: 12px; }
    .detail-grid dd { font-size: 14px; margin: 2px 0 0 8px; }
  </style>
</head>
<body>
  <nav>
    <div class="container">
      <span class="brand">🔗 CONEX</span>
      <a href="/">仪表盘</a>
      <a href="/tasks">任务列表</a>
      <a href="/sync">同步管理</a>
    </div>
  </nav>
  <div class="container">
    ${body}
  </div>
</body>
</html>`;
}

// ──────────────────────────────────────────
// Dashboard
// ──────────────────────────────────────────

app.get("/", async (c) => {
  const store = getCanonicalStore();
  const legacy = getSyncStore();

  let anytypeStatus = "❓";
  let appleStatus = "❓";
  try {
    const engine = await SyncEngine.create();
    const status = await engine.checkStatus();
    anytypeStatus = status.anytype.connected ? "✅ 已连接" : "❌ 未连接";
    appleStatus = status.reminders.available ? "✅ 可用" : "❌ 不可用";
  } catch { anytypeStatus = "⚠️ 错误"; }

  const all = store.list({ limit: 10000 });
  const completed = all.filter((t) => t.is_completed);
  const pending = all.filter((t) => !t.is_completed);
  const anytypeTasks = all.filter((t) => t.source === "anytype");
  const appleTasks = all.filter((t) => t.source === "apple");

  const byPriority = [0, 0, 0, 0];
  const priorityLabels = ["Q4", "Q2", "Q3", "Q1"];
  for (const t of all) byPriority[t.priority] = (byPriority[t.priority] || 0) + 1;

  const state = legacy.getSyncState();
  const lastSync = state.last_poll_at ? new Date(state.last_poll_at).toLocaleString("zh-CN") : "从未";

  const body = `
    <h1>🔍 仪表盘</h1>

    <div class="card">
      <div class="grid">
        <div class="stat"><div class="stat-value">${all.length}</div><div class="stat-label">总任务</div></div>
        <div class="stat"><div class="stat-value">${pending.length}</div><div class="stat-label">待办</div></div>
        <div class="stat"><div class="stat-value">${completed.length}</div><div class="stat-label">已完成</div></div>
        <div class="stat">
          <div class="stat-value">${anytypeTasks.length} / ${appleTasks.length}</div>
          <div class="stat-label">Anytype / Apple</div>
        </div>
      </div>
    </div>

    <div class="card">
      <h2>📡 连接状态</h2>
      <table>
        <tr><td>Anytype API</td><td>${anytypeStatus}</td></tr>
        <tr><td>Apple Reminders</td><td>${appleStatus}</td></tr>
        <tr><td>上次同步</td><td>${lastSync}</td></tr>
      </table>
    </div>

    <div class="card">
      <h2>🎯 优先级分布</h2>
      <table>
        ${priorityLabels.map((l, i) => `
          <tr>
            <td class="priority-${l}">${l} ${["(无)", "(计划)", "(委托)", "(执行)"][i]}</td>
            <td>${byPriority[i]}</td>
            <td><progress value="${byPriority[i]}" max="${Math.max(1, all.length)}" style="width:100%"></progress></td>
          </tr>
        `).join("")}
      </table>
    </div>

    <div class="actions">
      <button class="btn btn-primary" hx-post="/sync/trigger" hx-target="#sync-result" hx-swap="innerHTML">🔄 执行同步</button>
      <a href="/tasks" class="btn btn-secondary">📋 查看任务</a>
    </div>
    <div id="sync-result"></div>
  `;

  return c.html(layout("仪表盘", body));
});

// ──────────────────────────────────────────
// Task List
// ──────────────────────────────────────────

app.get("/tasks", async (c) => {
  const store = getCanonicalStore();
  const status = c.req.query("status");
  const prio = c.req.query("priority");
  const src = c.req.query("source");

  let all = store.list({ limit: 10000 });

  // Filters
  if (status === "completed") all = all.filter((t) => t.is_completed);
  else if (status === "pending") all = all.filter((t) => !t.is_completed);
  if (prio && ["0","1","2","3"].includes(prio)) all = all.filter((t) => t.priority === parseInt(prio));
  if (src) all = all.filter((t) => t.source === src);

  const priorityLabels = ["Q4", "Q2", "Q3", "Q1"];

  const rows = all.map((t) => `
    <tr onclick="window.location='/tasks/${t.id}'" style="cursor:pointer">
      <td>
        <span class="priority-${priorityLabels[t.priority]}">■</span>
        ${t.is_completed ? "✅" : "⬜"}
        ${t.name || "(无标题)"}
      </td>
      <td><span class="badge badge-${t.source === "anytype" ? "blue" : t.source === "apple" ? "green" : "gray"}">${t.source}</span></td>
      <td><span class="badge ${t.is_completed ? "badge-green" : "badge-yellow"}">${t.is_completed ? "已完成" : "待办"}</span></td>
      <td><span class="priority-${priorityLabels[t.priority]}">${priorityLabels[t.priority]}</span></td>
      <td style="color:#8e8e93;font-size:12px">${t.due_date ? new Date(t.due_date).toLocaleDateString("zh-CN") : "-"}</td>
    </tr>
  `).join("");

  const filterOptions = (key: string, current: string | undefined, options: [string, string][]) =>
    options.map(([v, label]) =>
      `<option value="${v}" ${current === v ? "selected" : ""}>${label}</option>`
    ).join("");

  const body = `
    <h1>📋 任务列表 <span style="font-size:14px;color:#8e8e93;font-weight:400">(${all.length})</span></h1>

    <form class="filters" hx-get="/tasks" hx-target="body" hx-push-url="true" hx-trigger="change">
      <select name="status">
        <option value="">全部状态</option>
        <option value="pending" ${status === "pending" ? "selected" : ""}>待办</option>
        <option value="completed" ${status === "completed" ? "selected" : ""}>已完成</option>
      </select>
      <select name="priority">
        <option value="">全部优先级</option>
        <option value="3" ${prio === "3" ? "selected" : ""}>Q1 执行</option>
        <option value="2" ${prio === "2" ? "selected" : ""}>Q3 委托</option>
        <option value="1" ${prio === "1" ? "selected" : ""}>Q2 计划</option>
        <option value="0" ${prio === "0" ? "selected" : ""}>Q4 不做</option>
      </select>
      <select name="source">
        <option value="">全部来源</option>
        <option value="anytype" ${src === "anytype" ? "selected" : ""}>Anytype</option>
        <option value="apple" ${src === "apple" ? "selected" : ""}>Apple</option>
        <option value="manual" ${src === "manual" ? "selected" : ""}>手动</option>
      </select>
      <noscript><button class="btn btn-primary" type="submit">筛选</button></noscript>
    </form>

    <div class="card" style="padding:0;overflow-x:auto">
      ${all.length === 0 ? '<div class="empty">暂无任务</div>' : `
        <table>
          <thead><tr>
            <th>名称</th><th>来源</th><th>状态</th><th>优先级</th><th>截止日期</th>
          </tr></thead>
          <tbody>${rows}</tbody>
        </table>
      `}
    </div>
  `;

  return c.html(layout("任务列表", body));
});

// ──────────────────────────────────────────
// Task Detail
// ──────────────────────────────────────────

app.get("/tasks/:id", async (c) => {
  const store = getCanonicalStore();
  const task = store.get(c.req.param("id"));

  if (!task) {
    return c.html(layout("未找到", '<div class="card"><h2>未找到任务</h2><a href="/tasks" class="btn btn-secondary">返回列表</a></div>'), 404);
  }

  const priorityLabels = ["Q4 (无)", "Q2 (计划)", "Q3 (委托)", "Q1 (执行)"];
  let tags = "";
  try { tags = task.tags ? JSON.parse(task.tags).join(", ") : "-"; } catch { tags = task.tags || "-"; }

  const body = `
    <h1>📝 ${task.name}</h1>

    <div class="card">
      <h2>基本字段</h2>
      <dl class="detail-grid">
        <dt>规范 ID</dt><dd><code>${task.id}</code></dd>
        <dt>名称</dt><dd>${task.name}</dd>
        <dt>描述</dt><dd>${task.description || "-"}</dd>
        <dt>截止日期</dt><dd>${task.due_date ? new Date(task.due_date).toLocaleString("zh-CN") : "-"}</dd>
        <dt>完成日期</dt><dd>${task.completion_date ? new Date(task.completion_date).toLocaleString("zh-CN") : "-"}</dd>
        <dt>优先级</dt><dd><span class="priority-${["Q4","Q2","Q3","Q1"][task.priority]}">${priorityLabels[task.priority]}</span></dd>
        <dt>已归档</dt><dd>${task.is_archived ? "✅ 是" : "❌ 否"}</dd>
      </dl>
    </div>

    <div class="card">
      <h2>同步信息</h2>
      <dl class="detail-grid">
        <dt>来源</dt><dd><span class="badge ${task.source === "anytype" ? "badge-blue" : task.source === "apple" ? "badge-green" : "badge-gray"}">${task.source}</span></dd>
        <dt>Anytype ID</dt><dd>${task.anytype_id ? `<code>${task.anytype_id.slice(0, 24)}...</code>` : "-"}</dd>
        <dt>Apple ID</dt><dd>${task.apple_id ? `<code>${task.apple_id}</code>` : "-"}</dd>
        <dt>空间 ID</dt><dd>${task.space_id ? `<code>${task.space_id.slice(0, 24)}...</code>` : "-"}</dd>
        <dt>版本</dt><dd>${task.version}</dd>
        <dt>最后同步</dt><dd>${task.last_synced ? new Date(task.last_synced).toLocaleString("zh-CN") : "未同步"}</dd>
      </dl>
    </div>

    <div class="card">
      <h2>Anytype 扩展字段</h2>
      <dl class="detail-grid">
        <dt>Status</dt><dd>${task.status_select || "-"}</dd>
        <dt>Status Tag Key</dt><dd>${task.status_tag_key || "-"}</dd>
        <dt>Done Checkbox</dt><dd>${task.done_checkbox !== null ? (task.done_checkbox ? "✅" : "⬜") : "无"}</dd>
        <dt>标签</dt><dd>${tags}</dd>
        <dt>Schedule</dt><dd>${task.schedule_date ? new Date(task.schedule_date).toLocaleString("zh-CN") : "-"}</dd>
      </dl>
    </div>

    <div class="actions">
      <button class="btn btn-primary" hx-post="/tasks/${task.id}/toggle" hx-target="body" hx-swap="outerHTML" hx-push-url="true">
        ${task.is_completed ? "↩️ 标记为待办" : "✅ 标记为完成"}
      </button>
      <a href="/tasks" class="btn btn-secondary">返回列表</a>
    </div>

    <div class="card" style="margin-top:16px">
      <h2>编辑任务</h2>
      <form hx-put="/tasks/${task.id}" hx-target="body" hx-swap="outerHTML" hx-push-url="true">
        <div class="form-row">
          <label>名称</label>
          <input name="name" value="${escHtml(task.name)}">
        </div>
        <div class="form-row">
          <label>描述</label>
          <textarea name="description">${escHtml(task.description)}</textarea>
        </div>
        <div class="form-row">
          <label>优先级</label>
          <select name="priority">
            <option value="0" ${task.priority === 0 ? "selected" : ""}>Q4 (不做)</option>
            <option value="1" ${task.priority === 1 ? "selected" : ""}>Q2 (计划)</option>
            <option value="2" ${task.priority === 2 ? "selected" : ""}>Q3 (委托)</option>
            <option value="3" ${task.priority === 3 ? "selected" : ""}>Q1 (执行)</option>
          </select>
        </div>
        <div class="form-row">
          <label>截止日期</label>
          <input name="due_date" type="datetime-local" value="${task.due_date ? task.due_date.slice(0, 16) : ""}">
        </div>
        <button class="btn btn-primary" type="submit">保存</button>
      </form>
    </div>
  `;

  return c.html(layout(task.name, body));
});

// ──────────────────────────────────────────
// Toggle completed
// ──────────────────────────────────────────

app.post("/tasks/:id/toggle", async (c) => {
  const store = getCanonicalStore();
  const task = store.get(c.req.param("id"));
  if (!task) return c.redirect("/tasks");

  store.update(task.id, {
    is_completed: task.is_completed ? 0 : 1,
    completion_date: task.is_completed ? null : new Date().toISOString(),
    last_synced: null, // 标记待推送
  });

  return c.redirect(`/tasks/${task.id}`);
});

// ──────────────────────────────────────────
// Update task (PUT)
// ──────────────────────────────────────────

app.put("/tasks/:id", async (c) => {
  const store = getCanonicalStore();
  const task = store.get(c.req.param("id"));
  if (!task) return c.redirect("/tasks");

  const body = await c.req.parseBody();
  const name = body.name as string;
  const description = body.description as string;
  const priority = parseInt(body.priority as string || "0", 10);
  const dueDateStr = body.due_date as string;

  store.update(task.id, {
    name: name || task.name,
    description: description ?? task.description,
    priority: isNaN(priority) ? task.priority : priority,
    due_date: dueDateStr ? new Date(dueDateStr).toISOString() : null,
    last_synced: null,
  });

  return c.redirect(`/tasks/${task.id}`);
});

// ──────────────────────────────────────────
// Sync Management
// ──────────────────────────────────────────

app.get("/sync", async (c) => {
  const legacy = getSyncStore();
  const state = legacy.getSyncState();
  const conflicts = legacy.listConflicts(20);

  const conflictRows = conflicts.map((c) => `
    <tr>
      <td style="font-size:12px">${c.created_at}</td>
      <td><code>${c.anytype_id.slice(0, 16)}...</code></td>
      <td><code>${c.apple_id.slice(0, 16)}...</code></td>
      <td><span class="badge ${c.resolution ? "badge-green" : "badge-yellow"}">${c.conflict_type}</span></td>
      <td>${c.resolution || "未解决"}</td>
    </tr>
  `).join("");

  const body = `
    <h1>🔄 同步管理</h1>

    <div class="card">
      <h2>手动触发同步</h2>
      <p style="color:#8e8e93;font-size:14px;margin-bottom:12px">当前运行模式：守护进程每 30 秒自动同步</p>
      <button class="btn btn-primary" hx-post="/sync/trigger" hx-target="#sync-result" hx-swap="innerHTML">🔄 执行一次同步</button>
      <div id="sync-result" style="margin-top:12px"></div>
    </div>

    <div class="card">
      <h2>同步状态</h2>
      <table>
        <tr><td>上次轮询</td><td>${state.last_poll_at ? new Date(state.last_poll_at).toLocaleString("zh-CN") : "从未"}</td></tr>
        <tr><td>累计同步</td><td>${state.total_synced} 个对象</td></tr>
        <tr><td>上次错误</td><td>${state.last_error || "无"}</td></tr>
      </table>
    </div>

    <div class="card" style="padding:0;overflow-x:auto">
      <h2 style="padding:20px 20px 0">冲突日志</h2>
      ${conflictRows.length === 0 ? '<div class="empty">暂无冲突</div>' : `
        <table>
          <thead><tr><th>时间</th><th>Anytype</th><th>Apple</th><th>类型</th><th>状态</th></tr></thead>
          <tbody>${conflictRows}</tbody>
        </table>
      `}
    </div>

    <div class="card">
      <h2>守护进程控制</h2>
      <div class="actions">
        <form action="/daemon/stop" method="post" style="display:inline">
          <button class="btn btn-secondary" type="submit">⏹ 停止守护进程</button>
        </form>
      </div>
    </div>
  `;

  return c.html(layout("同步管理", body));
});

// ──────────────────────────────────────────
// Sync Trigger (HTMX target)
// ──────────────────────────────────────────

app.post("/sync/trigger", async (c) => {
  try {
    const engine = await SyncEngine.create({ spaceId: "" });
    const result = await engine.sync();
    return c.html(`
      <div class="flash flash-success">
        ✅ 同步完成 — 新增 ${result.created}，更新 ${result.updated}，跳过 ${result.skipped}
        （${result.duration}ms）
        ${result.errors.length > 0 ? `<br>⚠️ ${result.errors.length} 个错误` : ""}
      </div>
    `);
  } catch (e: any) {
    return c.html(`
      <div class="flash flash-error">❌ 同步失败: ${escHtml(e.message)}</div>
    `);
  }
});

// ──────────────────────────────────────────
// Static files / health
// ──────────────────────────────────────────

app.get("/health", (c) => c.json({ status: "ok", tasks: getCanonicalStore().count() }));

// ──────────────────────────────────────────
// Start
// ──────────────────────────────────────────

function escHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

const PORT = parseInt(process.env.PORT || "3101", 10);
console.log(`🌐 CONEX Web UI: http://localhost:${PORT}`);

Bun.serve({
  fetch: app.fetch,
  port: PORT,
});
