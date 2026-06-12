# AGENTS.md — CONEX Agent 工作指引

> **最后更新**: 2026-06-12
> **状态**: Phase 1 (单向同步 Anytype Tasks → Apple Reminders) ✅

---

## 项目定位

CONEX 是 Anytype ↔ Apple 生态的同步桥接层。以 Anytype 为数据主锚点，向 Apple 生态单向同步。

## 当前状态

### ✅ Phase 0 — 验证与骨架
- ✅ 项目骨架搭建 (TypeScript + Bun + CLI)
- ✅ Anytype API 连通性验证 (`/v1/` 端点, `/v1/search` 查询)
- ✅ AppleScript Reminders 操作验证 (创建/查询/删除)
- ✅ SQLite 本地状态存储 (id_map, sync_state, conflict_log)

### ✅ Phase 1 — 单向同步 Anytype Tasks → Apple Reminders
- ✅ Anytype Adapter (search/CRUD)
- ✅ Apple Reminders Adapter (AppleScript, locale 安全的日期处理)
- ✅ Task→Reminder Mapper (含属性提取、优先级映射、冲突检测)
- ✅ Sync Engine (创建/更新/跳过/冲突日志)
- ✅ CLI sync/status/config/history 命令
- ✅ 端到端测试: **55 个 Anytype Task → 55 个 Apple Reminders (零错误)**

### ✅ Phase 2 — 守护模式 & 自动化
- ✅ Hermes Skill 注册 (`conex-sync`)
- ✅ 守护模式 (Hermes cronjob, 每 2 分钟自动同步, no_agent 模式)

### ✅ Phase 3 — 双向同步 & 深度链接
- ✅ **双向同步**: Apple Reminders 完成 → 写回 Anytype status=DONE
- ✅ **status 属性映射**: 使用 Anytype `status` select 属性（TODO/DONE/WILL/PEND/QUIT），不依赖 `done` checkbox
- ✅ **深层修复**: `setTaskDone` 只设置 `status: DONE`，不再设置 `done: checkbox:true`
- ✅ **深度链接**: 每个 Reminder 备注含 `anytype://{id}`，iOS 上可直达 Anytype 对象
- ✅ **守护进程模式**: `bun run daemon start --space <id>` 长期运行
- ✅ **Hermes Cron 定时同步**: 每 2 分钟自动双向同步（无变更静默，零 token 消耗）

### 🔜 Phase 4 — 更多适配器
- [ ] Apple Calendar 适配器 (CalDAV)
- [ ] Apple Notes 适配器 (AppleScript)
- [ ] MCP Server

## 关键参考

| 参考 | 位置 | 用途 |
|------|------|------|
| 架构设计 | `ARCHITECTURE.md` | 全景图、映射模型、同步流 |
| Anytype API | `developers.anytype.io` | OpenAPI v2025-11-08, `/v1/` 端点 |
| 验证脚本 | `scripts/anytype-poll.ts` | Anytype API 连通性检查 |
| 验证脚本 | `scripts/apple-test.ts` | Apple Reminders 操作测试 |

## 代码约定

1. 适配器通过构造函数注入配置，无全局状态
2. 每个 Mapper 必须有单元测试
3. SQLite 使用 WAL 模式
4. Anytype 对象在 Apple 侧标记 `conex:{anytype_id}` 用于追踪
5. 操作日志使用 JSON lines 格式输出到 `~/.conex/logs/`

## 验证方式

```bash
bun test                           # 单元测试 (13 个)
bun run verify:anytype             # Anytype API 连通性
bun run verify:apple               # Apple Reminders 操作
bun run sync -- --space <id>       # 执行一次双向同步
bun run status                     # 查看同步状态
bun run config show                # 查看配置
bun run history                    # 同步历史
```

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
