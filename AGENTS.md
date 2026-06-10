# AGENTS.md — CONEX Agent 工作指引

> **最后更新**: 2026-06-10
> **状态**: Phase 0 (策划/骨架阶段)

---

## 项目定位

CONEX 是 Anytype ↔ Apple 生态的同步桥接层。不是通用同步器，而是**单向渐进式**的项目：先 Anytype Tasks → Apple Reminders，再逐步扩展。

## 当前状态

- ✅ **ARCHITECTURE.md** — 完整架构设计
- ✅ **README.md** — 项目说明
- ✅ **package.json** (骨架)
- 🔜 **Anytype API 连通性验证** — Phase 0 的第一步
- 🔜 **AppleScript Reminders 脚本** — Phase 0 的第二步

## 关键参考

| 参考 | 位置 | 用途 |
|------|------|------|
| 架构设计 | `ARCHITECTURE.md` | 全景图、映射模型、同步流 |
| Anytype MCP Server | `github.com/anyproto/anytype-mcp` | Anytype API 使用参考 |
| Anytype API Docs | `github.com/anyproto/anytype-api` | OpenAPI 规范 |
| 根项目 AGENTS.md | `../../AGENTS.md` | Labry 工程规范 |
| 根项目 ARCHITECTURE.md | `../../ARCHITECTURE.md` | Labry 生态全览 |

## 开发顺序

```
Phase 0 ─── 验证连通性
  Task 1: 调通 Anytype REST API (查询 Objects)
  Task 2: 编写 AppleScript 创建/查询 Reminders
  Task 3: 验证 SQLite 本地存储

Phase 1 ─── 单向同步 Anytype Tasks → Apple Reminders
  Task 4: 实现 Anytype Adapter 核心
  Task 5: 实现 Apple Reminders Adapter (AppleScript)
  Task 6: 实现 Task→Reminder Mapper
  Task 7: 实现 Sync Engine (单次)
  Task 8: CLI sync/status 命令
  Task 9: Hermes Skill 注册
```

## 代码约定

1. 遵循 Labry 根项目的 AGENTS.md (`../../AGENTS.md`) 所有约定
2. 适配器通过构造函数注入配置，无全局状态
3. 每个 Mapper 必须有单元测试
4. SQLite 使用 WAL 模式
5. 所有 Anytype 对象在 Apple 侧标记 `conex:{anytype_id}` 用于追踪
6. 操作日志使用 JSON lines 格式输出到 `~/.conex/logs/`

## 验证方式

```bash
# Phase 0 验证
bun run scripts/anytype-poll.ts    # 测试 Anytype API 连通性
bun run scripts/apple-test.ts      # 测试 AppleScript Reminders 脚本

# Phase 1 验证
bun run src/cli sync              # 执行一次同步
bun run src/cli status             # 查看同步状态
bun test                           # 所有单元测试
```