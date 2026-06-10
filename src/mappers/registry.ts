// Mapper 注册表 — 管理所有 SyncMapper 实例

import type { SyncMapper } from "../adapters/types.js";

class MapperRegistry {
  private mappers = new Map<string, SyncMapper<unknown, unknown>>();

  register(mapper: SyncMapper<unknown, unknown>): void {
    const key = `${mapper.sourceType}→${mapper.targetType}`;
    this.mappers.set(key, mapper);
  }

  get(sourceType: string, targetType: string): SyncMapper<unknown, unknown> | undefined {
    const key = `${sourceType}→${targetType}`;
    return this.mappers.get(key);
  }

  list(): string[] {
    return Array.from(this.mappers.keys());
  }

  /** 获取所有支持指定 source 类型的 Mapper */
  findBySource(sourceType: string): SyncMapper<unknown, unknown>[] {
    return Array.from(this.mappers.values()).filter(
      (m) => m.sourceType === sourceType,
    );
  }
}

export const mapperRegistry = new MapperRegistry();