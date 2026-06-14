// CONEX — 跨平台连接器主入口
// Cross-Platform Nexus: Anytype ↔ Apple 生态同步桥接

export { AnytypeAdapter, AnytypeAdapterError } from "./adapters/anytype/adapter.js";
export type { AnytypeObject, AnytypeSpace } from "./adapters/anytype/types.js";
export { loadCredentials, saveCredentials, hasApiKey } from "./adapters/anytype/auth.js";

export { AppleRemindersAdapter, AppleRemindersAdapterError } from "./adapters/apple/reminders.js";
export type { ReminderData, ReminderList } from "./adapters/apple/types.js";

export type { SyncMapper, SyncDirection, Conflict, AdapterStatus, SyncResult, ChangeEvent } from "./adapters/types.js";

export { SyncEngine } from "./engine/sync-engine.js";
export type { SyncConfig } from "./engine/sync-engine.js";
export { BidirectionalDaemon } from "./engine/daemon.js";

export {
  mapperRegistry,
} from "./mappers/registry.js";
export { anytypeToCanonical, canonicalToAnytypePatch, computeAnytypeContentHash } from "./mappers/anytype-mapper.js";
export { reminderToCanonical, canonicalToReminder, extractConexIdFromNotes } from "./mappers/reminder-mapper.js";

export { getCanonicalStore, CanonicalStore } from "./store/canonical-store.js";
export type { CanonicalTask } from "./store/schema.js";
export {
  createTask,
  canonicalToApplePriority,
  appleToCanonicalPriority,
  priorityToQuadrantLabel,
} from "./store/schema.js";
