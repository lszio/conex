# CONEX — 跨平台连接器 (Cross-Platform Nexus)

> **状态**: 策划中 (v0.1-plan)
> **版本**: 0.1.0
> **语言**: TypeScript (Bun workspace)
> **目的**: 在不同个人知识/任务/日程平台之间建立双向同步和单向桥接的中间层

---

## 一、项目定位

CONEX 不是又一个"什么都能干的同步器"。它是**以 Anytype 为数据主锚点（Source of Truth），向 Apple 全家桶（待办事项、日历、笔记）单向/双向同步的轻量桥接层**。

### 核心理念

```
[Anytype]  ←→  [CONEX Adapter Layer]  ←→  [Apple 生态]
 数据主权               桥接               日常使用
 (语义丰富)                              (原生体验)
```

- **Anytype** 是你的知识/任务/日程的"语义中枢"——那里数据有类型、有关系、有上下文
- **Apple 生态** 是你日常交互的"操作面"——Apple Reminders 提提醒、Calendar 看日程、Notes 记灵感
- **CONEX** 在中间做格式转换、状态映射、冲突协商

### 为什么用 Anytype 而不是直接 iCloud

| | Anytype | Apple 原生 |
|--|---------|-----------|
| 数据所有权 | ✅ 本地优先 + E2E 加密 | ❌ iCloud 锁定 |
| 语义模型 | ✅ 类型/关系/模板 | ❌ 扁平列表/日历项 |
| API 可编程 | ✅ REST API + MCP | ⚠️ 有限（CalDAV / EKEventStore） |
| Agent 友好 | ✅ MCP 生态 | ❌ 无原生 MCP |
| 跨平台 | ✅ Win/Mac/Linux/iOS/Android | ❌ 仅 Apple |

---

## 二、全景架构

```
┌─────────────────────────────────────────────────────────────┐
│                        CONEX System                          │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌─────────────────────────────────────────────────────┐     │
│  │               Sync Engine (核心调度器)               │     │
│  │  ┌───────┐  ┌──────────┐  ┌────────┐  ┌────────┐  │     │
│  │  │Poller │  │Conflict  │  │State   │  │Mapper  │  │     │
│  │  │       │  │Resolver  │  │Tracker │  │Registry│  │     │
│  │  └───────┘  └──────────┘  └────────┘  └────────┘  │     │
│  └─────────────────────────────────────────────────────┘     │
│                                                               │
│  ┌────────────────────────┐  ┌────────────────────────┐      │
│  │   Anytype Adapter      │  │    Apple Adapter       │      │
│  │  ┌──────────────────┐  │  │  ┌──────────────────┐  │      │
│  │  │ Anytype API (REST)│  │  │  │ Reminders (MCP)  │  │      │
│  │  │ Anytype MCP Client│  │  │  │ Calendar (CalDAV)│  │      │
│  │  │ 连接管理 / 认证   │  │  │  │ Notes (AppleScript│  │      │
│  │  └──────────────────┘  │  │  │       / sqlite)   │  │      │
│  │                        │  │  └──────────────────┘  │      │
│  └────────────────────────┘  └────────────────────────┘      │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐     │
│  │                Agent 接口层                          │     │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────┐  │     │
│  │  │Hermes    │  │MCP Server│  │CLI (conex)      │  │     │
│  │  │  Skill   │  │          │  │sync/status/config│  │     │
│  │  └──────────┘  └──────────┘  └──────────────────┘  │     │
│  └─────────────────────────────────────────────────────┘     │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐     │
│  │  状态存储 (SQLite / ~/.conex/)                       │     │
│  │  - sync_state: 记录上一次同步偏移                    │     │
│  │  - id_map:     Anytype ID ↔ Apple ID 映射表          │     │
│  │  - conflict_log: 冲突历史                            │     │
│  │  - config:     用户偏好配置                          │     │
│  └─────────────────────────────────────────────────────┘     │
│                                                               │
└─────────────────────────────────────────────────────────────┘
```

---

## 三、同步流详解

### 3.1 Anytype → Apple Reminders (任务同步)

```
┌─────────┐          ┌─────────┐          ┌───────────┐
│ Anytype │  REST    │ CONEX   │  MCP/    │  Apple    │
│ Tasks   │ ───────→ │ Sync    │  Script  │  Reminders│
│         │          │ Engine  │ ───────→ │           │
│ type:   │          │         │          │ list:     │
│  Task   │          │ Mapper  │          │  CONEX-   │
│          │          │  │             │  Anytype  │
│ fields: │          │  ▼             │           │
│  name   │          │ id_map         │ fields:   │
│  status │          │  tx_id ↔       │  title    │
│  assign │          │  reminder_id   │  notes    │
│  duedate│          │                 │  due_date │
│  tags   │          │                 │  list     │
│  space  │          │                 │  priority │
└─────────┘          └─────────┘          └───────────┘
         ▲                                  │
         │           ┌─────────┐            │
         └───────────│ Conflict│◄───────────┘
                     │Resolver │  (Apple 侧
                     │         │   手动完成
                     │         │   写回 Anytype
                     └─────────┘   为 Phase 2)
```

**Phase 1 (只读)**: Anytype 的 Task 对象 → 单向同步到 Apple Reminders
**Phase 2 (双向)**: Apple Reminders 上完成任务 → 写回 Anytype 的 Task 状态

### 3.2 Anytype → Apple Calendar (日程同步)

```
┌─────────┐          ┌─────────┐          ┌───────────────┐
│ Anytype │  REST    │ CONEX   │  CalDAV  │ Apple Calendar│
│ Objects │ ───────→ │ Sync    │ ───────→ │               │
│ with    │          │ Engine  │          │ event:        │
│ date    │          │         │          │  CONEX-{id}   │
│ fields  │          │ Mapper  │          │               │
│         │          │  │             │  fields:        │
│ (type:  │          │  ▼             │  title          │
│  Task   │          │ id_map         │  notes          │
│  Event  │          │  _id ↔         │  start_date     │
│  Note   │          │  event_id      │  end_date       │
│  ...)   │          │                 │  location       │
└─────────┘          └─────────┘          └───────────────┘
```

**任何带有日期字段的 Anytype 对象**都可以投射为日历事件。通过 Anytype 的查询 API 筛选。

### 3.3 Anytype → Apple Notes (知识同步，单向)

```
┌─────────┐          ┌─────────┐          ┌───────────┐
│ Anytype │  REST    │ CONEX   │ Apple-  │ Apple     │
│ Objects │ ───────→ │ Sync    │ Script  │ Notes     │
│         │          │ Engine  │ ───────→ │           │
│ types:  │          │         │          │ folder:   │
│  Note   │          │ Mapper  │          │  CONEX-   │
│  Article│          │  │             │  Anytype  │
│  Doc    │          │  ▼             │           │
│         │          │ markdown       │ content   │
│         │          │ conversion     │ (rich txt)│
│         │          │                 │ tags      │
└─────────┘          └─────────┘          └───────────┘
```

**单向**: Anytype 知识对象 → Apple Notes 只读副本。Apple Notes 上不修改。

---

## 四、类型映射模型

### 4.1 类型注册表 (Mapper Registry)

每个同步方向对都需要一个 "Mapper" —— 定义 Anytype 实体 ↔ 目标平台实体的字段映射规则。

```typescript
// Mapper 接口定义
interface SyncMapper<TSource, TTarget> {
  sourceType: string;           // Anytype object type (e.g. "Task")
  targetType: string;           // Apple type (e.g. "Reminder")
  direction: SyncDirection;     // "forward" | "backward" | "bidirectional"

  // 从 Anytype 到 Apple 的转换
  toTarget(source: TSource): TTarget;

  // 从 Apple 到 Anytype 的转换 (双向时需要)
  toSource(target: TTarget): TSource;

  // 冲突检测: 返回两边的差异点
  detectConflict(source: TSource, target: TTarget): Conflict[];
}
```

### 4.2 Task (Anytype) ↔ Reminder (Apple Reminders)

| Anytype 字段 | Apple Reminders 字段 | 方向 | 说明 |
|-------------|---------------------|------|------|
| `name` | `title` | 双向 | 任务标题 |
| `description` | `notes` | 双向 | 备注/描述 |
| `deadline` | `dueDate` | 双向 | 截止日期 |
| `done` | `isCompleted` | 双向 | 完成状态 |
| `priority` | `priority` | 单向 Forward | 优先级映射 (1-5 ↔ low/med/high) |
| `tags[]` | `list` + 子目录 | 单向 Forward | 标签变成子列表 |
| `assignee` | — | 丢弃 | Apple Reminders 无此概念 |
| `id` | `conexId` (自定义字段) | — | 用于 ID 映射追踪 |

### 4.3 Anytype Date-Object ↔ Calendar Event

| Anytype 字段 | Calendar Event 字段 | 说明 |
|-------------|-------------------|------|
| `name` | `title` | 事件标题 |
| `description` | `notes` | 描述 |
| `date` / `startDate` | `startDate` | 开始时间 |
| `endDate` | `endDate` | 结束时间 (如无则 +1h) |
| `location` | `location` | 位置 |
| `tags[]` | `alarms` | 标签 → 提醒设置 |

### 4.4 Anytype Note ↔ Apple Notes

| Anytype 字段 | Apple Notes 字段 | 说明 |
|-------------|-----------------|------|
| `name` | `title` | 笔记标题 |
| `description` (Markdown) | `body` (富文本) | Markdown → HTML 转换 |
| `tags[]` | `tags` | 标签 (AppleNotes 支持) |
| `space` | `folder` | 空间 → 文件夹 |

---

## 五、适配器设计 (Adapter Layer)

### 5.1 Anytype 适配器

**依赖的外部项目**: `anytype-mcp` (GitHub: `anyproto/anytype-mcp`)

Anytype 提供：
- **REST API** (OpenAPI spec, 本地端口 `http://127.0.0.1:31009`)
- **MCP Server** (`@anyproto/anytype-mcp`)
- **API Key 认证** (Bearer token)

CONEX 中将：
1. 通过 REST API 直接读取对象数据（批量查询、按类型过滤、按时间排序）
2. 通过 `GET /objects` → 筛选 Task/Event/Note 类型
3. 使用 Anytype 的 "API version 2025-11-08"（当前最新）

```typescript
// Anytype Adapter 核心接口
interface AnytypeAdapter {
  // 查询对象
  queryObjects(params: {
    type: string;           // "Task" | "Event" | "Note" | ...
    spaceId?: string;
    limit?: number;
    offset?: number;
    lastSync?: Date;       // 增量同步用
  }): Promise<AnytypeObject[]>;

  // 更新对象状态 (双向同步)
  updateObject(id: string, data: Partial<AnytypeObject>): Promise<void>;

  // 空间列表
  listSpaces(): Promise<Space[]>;

  // 健康检查
  healthCheck(): Promise<boolean>;
}
```

### 5.2 Apple Reminders 适配器

**三种实现方案**（按推荐优先级）：

| 方案 | 难度 | 稳定性 | 说明 |
|------|------|--------|------|
| **EventKit CLI** (swift script) | 中 | ★★★ | 编译一个 Swift CLI 工具，使用 EventKit 框架处理 Reminders |
| **AppleScript** | 低 | ★★ | 通过 osascript 调用，适合原型 |
| **MCP Bridge** (第三方) | 中 | ★★ | 等待 Apple Reminders MCP Server 出现 |

**推荐**: Phase 1 使用 AppleScript 快速验证，Phase 2 写成 Swift CLI 工具。

```bash
# AppleScript 示例: 创建提醒
osascript -e '
tell application "Reminders"
    set newReminder to make new reminder
    set name of newReminder to "测试任务"
    set due date of newReminder to date "2026-06-15 10:00:00"
    set body of newReminder to "来自 Anytype 的任务描述"
end tell'
```

```typescript
interface AppleRemindersAdapter {
  // 创建/更新/完成提醒
  createReminder(data: ReminderData): Promise<string>;  // returns local id
  updateReminder(id: string, data: Partial<ReminderData>): Promise<void>;
  completeReminder(id: string): Promise<void>;

  // 查询提醒 (用于冲突检测)
  listReminders(list?: string): Promise<ReminderData[]>;
  getReminder(id: string): Promise<ReminderData | null>;

  // 获取最近变更 (增量同步用)
  getRecentChanges(since: Date): Promise<ChangeEvent[]>;
}
```

### 5.3 Apple Calendar 适配器

**方案**: **CalDAV** 协议通过 HTTP PUT/GET/PROPFIND 操作 iCloud Calendar

Apple Calendar 支持 CalDAV (RFC 4791) 和 iCalendar (RFC 5545) 格式。

```bash
# CalDAV PROPFIND 示例 (通过 curl)
curl -X PROPFIND \
  -H "Depth: 1" \
  -H "Content-Type: application/xml; charset=utf-8" \
  -u "$APPLE_ID:$APP_SPECIFIC_PASSWORD" \
  https://caldav.icloud.com/.../calendar/
```

**开发优先级**: 在 Reminders 适配器稳定后再开始。

### 5.4 Apple Notes 适配器

**方案**: **Apple Notes SQLite** (只读) 或 **AppleScript**

```bash
# AppleScript 示例: 创建笔记
osascript -e '
tell application "Notes"
    set newNote to make new note at folder "CONEX-Anytype"
    set name of newNote to "测试笔记"
    set body of newNote to "来自 Anytype 的内容..."
end tell'
```

---

## 六、核心 Sync Engine

### 6.1 同步循环

```
每次同步 tick:
  1. CONEX 启动 → 加载 config + state
  2. 轮询 Anytype API: GET /objects?lastSync={lastTimestamp}
  3. 对于每个变更的对象:
     a. 查找 id_map: Anytype_id → Apple_id 是否存在
     b. 如果不存在: 创建 Apple 实体
     c. 如果存在: 检测冲突 (对比 lastModified)
        - 无冲突 → 更新 Apple 实体
        - 有冲突 → 记录 conflict_log, 按策略处理
  4. 更新 sync_state (lastSync, 统计信息)
```

### 6.2 ID 映射存储

```sql
-- ~/.conex/sync.db
CREATE TABLE id_map (
    anytype_id    TEXT PRIMARY KEY,
    apple_id      TEXT NOT NULL,
    apple_type    TEXT NOT NULL,  -- 'reminder' | 'event' | 'note'
    last_sync_at  TEXT NOT NULL,  -- ISO 8601
    last_hash     TEXT,           -- 上一次同步的内容哈希
    direction     TEXT NOT NULL   -- 'forward' | 'bidirectional'
);

CREATE TABLE sync_state (
    id            INTEGER PRIMARY KEY DEFAULT 1,
    last_poll_at  TEXT NOT NULL,          -- 上一次轮询时间
    total_synced  INTEGER DEFAULT 0,
    last_error    TEXT,
    config_hash   TEXT                    -- 配置变更检测
);

CREATE TABLE conflict_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    anytype_id    TEXT NOT NULL,
    apple_id      TEXT NOT NULL,
    conflict_type TEXT NOT NULL,   -- 'content_mismatch' | 'deleted_on_one_side'
    resolution    TEXT,            -- 'use_anytype' | 'use_apple' | 'manual'
    resolved_at   TEXT,
    created_at    TEXT NOT NULL
);

CREATE TABLE config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

### 6.3 冲突策略

| 场景 | 默认策略 | 可配置? |
|------|---------|--------|
| Anytype 更新 → Apple 未改 | 推送 Anytype 版本 | 否 |
| Apple 完成 → Anytype 未改 | Phase 1: 忽略; Phase 2: 写回 Anytype | 是 |
| 两边同时改 | 保留 Anytype 版本，记录日志 | 是 |
| Apple 侧删除 | Phase 1: 忽略; Phase 2: 从 Anytype 重建 | 是 |

---

## 七、Agent 接口层

### 7.1 Hermes Skill

作为一个 Hermes Agent skill 注册，让 Agent 可以直接操作 CONEX。

```typescript
// Hermes 中自动可用
// "sync anytype to apple" → 触发同步
// "conex status" → 查看同步状态
// "conex config set reminders.enabled true"
```

### 7.2 MCP Server

提供标准 MCP 协议，让其他 Agent (Claude Code、Cursor) 也能操纵同步：

| Tool | 说明 |
|------|------|
| `conex_sync` | 触发一次同步 |
| `conex_status` | 查看各适配器状态 |
| `conex_config` | 读写配置 |
| `conex_history` | 查看同步历史/冲突日志 |

### 7.3 CLI

```bash
conex sync              # 执行同步
conex status            # 查看状态
conex config show       # 查看配置
conex config set <key> <value>
conex history           # 查看同步历史
conex daemon            # 后台守护模式（定时轮询）
```

---

## 八、项目结构

```
modules/conex/
├── AGENTS.md                 — Agent 工作指引
├── README.md                 — 项目 README
├── package.json              — Bun workspace package
├── tsconfig.json
│
├── src/
│   ├── index.ts              — 主入口 (CLI + Server)
│   │
│   ├── engine/               — 同步引擎
│   │   ├── sync-engine.ts    — 同步循环调度器
│   │   ├── poller.ts         — 轮询策略
│   │   ├── state-tracker.ts  — 状态追踪
│   │   └── conflict-resolver.ts  — 冲突解决
│   │
│   ├── adapters/             — 各平台适配器
│   │   ├── anytype/
│   │   │   ├── adapter.ts    — Anytype REST API 客户端
│   │   │   ├── types.ts      — Anytype 类型定义
│   │   │   └── auth.ts       — API Key 认证
│   │   ├── apple/
│   │   │   ├── reminders.ts  — Apple Reminders 适配器
│   │   │   ├── calendar.ts   — Apple Calendar (CalDAV) 适配器
│   │   │   ├── notes.ts      — Apple Notes 适配器
│   │   │   └── types.ts      — Apple 类型定义
│   │   └── types.ts          — 通用适配器接口
│   │
│   ├── mappers/              — 类型映射器
│   │   ├── registry.ts       — Mapper 注册表
│   │   ├── task-to-reminder.ts
│   │   ├── date-to-event.ts
│   │   └── note-to-note.ts
│   │
│   ├── store/                — 状态存储
│   │   ├── db.ts             — SQLite 数据库管理
│   │   ├── id-map.ts         — ID 映射表操作
│   │   └── config.ts         — 配置读写
│   │
│   ├── server/               — MCP 服务端
│   │   ├── mcp-server.ts     — MCP 协议实现
│   │   └── tools.ts          — MCP 工具定义
│   │
│   └── cli/                  — CLI 接口
│       ├── commands/
│       │   ├── sync.ts
│       │   ├── status.ts
│       │   ├── config.ts
│       │   └── daemon.ts
│       └── index.ts
│
├── scripts/                  — 辅助脚本
│   ├── anytype-poll.ts       — 手动轮测 Anytype API
│   └── apple-test.ts         — 测试 Apple Reminders 连通性
│
└── test/
    ├── adapters/
    ├── engine/
    ├── mappers/
    └── store/
```

---

## 九、实施路线图

### Phase 0 — 验证与骨架 (当前)

- 搭建项目骨架 + AGENTS.md
- 验证 Anytype API 连通性（手动调通 REST API）
- 验证 AppleScript 操作 Reminders 脚本
- 确认 CalDAV / Notes AppleScript 可行性

### Phase 1 — 单向同步 (Anytype→Apple)

- 实现 `anytype/adapter.ts` (REST API 查询)
- 实现 `apple/reminders.ts` (通过 AppleScript 创建/更新)
- 实现 Mapper: `task-to-reminder.ts`
- 实现 `store/db.ts` + `id-map.ts` (SQLite)
- 实现一次性的 `sync-engine.ts` 被动同步
- CLI: `conex sync` / `conex status`
- Hermes Skill 注册

### Phase 2 — 多适配器 + 双向同步

- Apple Calendar (CalDAV) 桥接
- Apple Notes (AppleScript) 桥接
- Apple Reminders 写的任务 → 写回 Anytype (双向)
- `conflict-resolver.ts` 冲突处理
- `conex daemon` 守护模式 + 定时轮询

### Phase 3 — 生产化

- Apple Reminders 适配器迁移到 Swift CLI (EventKit)
- MCP Server 发布
- 配置 UI / 同步状态仪表盘
- 错误处理 + 重试 + 通知
- 可配置的同步频率 + 筛选规则

### Phase 4 — 扩展

- 更多 Apple 服务：Siri Shortcuts 集成、Focus Mode 联动
- 更多数据平台适配（Notion、Obsidian 等）
- Anytype 侧更深度的类型映射（模板自动创建等）

---

## 十、技术选型理由

| 决策 | 选择 | 理由 |
|------|------|------|
| 语言 | TypeScript (Bun) | 与 Labry monorepo 一致，Anytype MCP 也是 TS |
| 包管理器 | Bun | 已有 Monorepo 基础设施 |
| 状态存储 | SQLite (via bun:sqlite) | 轻量、无外部依赖、零配置 |
| 与 Apple 交互 | AppleScript Phase1 → Swift CLI Phase3 | AppleScript 最快速验证，Swift 生产级 |
| 日历协议 | CalDAV | iCloud Calendar 原生支持，不需要额外授权 |
| Anytype 交互 | REST API (直接) | 比 MCP 更可控，支持批量查询 |
| 同步模式 | Polling | 简单可靠，Anytype API 无 Webhook |
| CLI 框架 | commander | 与 layerc 一致 |

---

## 十一、风险和已知问题

| 风险 | 影响 | 缓解 |
|------|------|------|
| AppleScript 稳定性 | Apple 可能变更脚本协议 | Phase 3 迁移到 Swift EventKit |
| CalDAV 认证 | 需要 App-Specific Password | 写文档指导用户生成 |
| Anytype API 版本升级 | 字段/路由变更 | 版本 pin + 迁移测试 |
| 同步冲突丢失数据 | 用户信息丢失 | 默认用 Anytype → 保留日志 + 手动解决 |
| 大同步量性能 | 首次全量同步慢 | 分页查询 + 进度显示 |
| Apple Notes 双向不可行 | Notes API 极度受限 | 保持单向 + 明显的只读标记 |

---

## 十二、开发约定

1. **不可变状态优先**: sync_state 使用 WAL 模式 SQLite
2. **错误可观测**: 每次同步产生结构化日志 (JSON lines)
3. **幂等操作**: 重复同步不产生重复提醒/事件
4. **Idempotency Key**: 每个 Anytype 对象生成唯一 `conexId` 存入 Apple 侧
5. **渐进复杂**: 先做最窄的单向同步，再扩展
6. **测试先行**: 每个 Mapper 必须有单元测试
7. **无全局状态**: 所有适配器通过构造函数注入配置

---

> **CONEX 不追求"万物互联"。它只做好一件事：让 Anytype 的知识在 Apple 设备上触手可及。**