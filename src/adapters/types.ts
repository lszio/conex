// 通用适配器接口定义

export type SyncDirection = "forward" | "backward" | "bidirectional";

export interface SyncMapper<TSource, TTarget> {
  sourceType: string;
  targetType: string;
  direction: SyncDirection;

  /** 从 Anytype 到 Apple 的转换 */
  toTarget(source: TSource): TTarget;

  /** 从 Apple 到 Anytype 的转换 (双向时需要) */
  toSource(target: TTarget): TSource;

  /** 冲突检测: 返回两边的差异点 */
  detectConflict(source: TSource, target: TTarget): Conflict[];
}

export interface Conflict {
  field: string;
  sourceValue: unknown;
  targetValue: unknown;
  severity: "info" | "warning" | "error";
}

export interface AdapterStatus {
  connected: boolean;
  lastCheck: Date;
  version?: string;
  error?: string;
}

export interface SyncResult {
  success: boolean;
  created: number;
  updated: number;
  skipped: number;
  conflicts: number;
  errors: string[];
  duration: number; // ms
}

export interface ChangeEvent {
  id: string;
  type: "created" | "updated" | "deleted";
  timestamp: Date;
  payload: unknown;
}