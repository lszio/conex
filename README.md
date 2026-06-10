# CONEX

> **Cross-Platform Nexus** — 以 Anytype 为数据主锚点，向 Apple 生态（Reminders / Calendar / Notes）同步的轻量桥接层。

CONEX 是 Labry 生态下的一个模块，定位为**个人知识/任务/日程的跨平台连接器**。

## 核心原则

- **Anytype 是 Source of Truth** — 你的数据在 Anytype 里最有语义（类型、关系、上下文）
- **Apple 是操作面** — Reminders 提提醒、Calendar 看日程、Notes 记灵感
- **CONEX 是转换层** — 只做格式映射、状态追踪、冲突处理，不存业务数据

## 快速开始

```bash
cd modules/conex
bun install
bun run dev          # 交互式调试
```

## 开发状态

| 里程碑 | 状态 | 说明 |
|--------|------|------|
| Phase 0: 骨架 + 验证 | 🔜 待启动 | 搭建项目 + 验证 Anytype API + AppleScript |
| Phase 1: 单向同步 | ⏳ 计划中 | Anytype Tasks → Apple Reminders |
| Phase 2: 多适配器 | 📅 未来 | Calendar / Notes / 双向同步 |
| Phase 3: 生产化 | 📅 未来 | Swift CLI / MCP Server / 守护模式 |

详见 [ARCHITECTURE.md](ARCHITECTURE.md)。