/**
 * EventFollower：持久事件单流消费与去重（计划 §7 状态机；
 * docs/contracts/event-recovery.md §2-§3）。不依赖 React。
 *
 * snapshot → replaceProjectionAndCursor；watchHead → start（显式新注册）；
 * read from cursor → validate stream/epoch/next position → apply → advance；
 * hasMore 或 latestHead>cursor → 续读（一次一个 read 在途）；transport lost →
 * stale（重认证/重发现由 transport 负责，recover() 重新订阅并续读）；
 * resume_unavailable → fresh snapshot（绝不重放命令）；forbidden → 停止并报告 revoked。
 */

import { ConexError } from "./errors";
import { ErrorCode } from "./generated/conex/common";
import type {
  AgentEvent,
  AgentEventCursor,
  AgentEventReadResult,
  AgentMessageDelta,
  AgentSnapshot,
} from "./generated/conex/events";
import type { AgentMessagePart } from "./generated/conex/entities";
import { isCanonicalUint64, isRecord, isUlid } from "./s1-wire";

/** 领域传输接口；由真实 SDK 消费者实现（不引入第二层 service locator）。 */
export interface AgentEventTransport {
  snapshot(conversationId: string): Promise<unknown>;
  read(cursor: AgentEventCursor, limit?: number): Promise<unknown>;
  watchHead(conversationId: string, onHead: (head: string) => void): void;
}

export type FollowerStatus = "idle" | "syncing" | "current" | "stale" | "revoked";

/** 协议故障：同键异内容、offset 缺口、跳号、epoch 变更——停止并要求重新同步。 */
export class ResyncRequiredError extends ConexError {
  constructor(message: string) {
    super(message, {
      code: ErrorCode.ERROR_CODE_RESUME_UNAVAILABLE,
      execution: "not_started",
      retry: "safe",
    });
    this.name = "ResyncRequiredError";
  }
}

function resync(message: string): never {
  throw new ResyncRequiredError(message);
}

const encoder = new TextEncoder();

function requireString(value: unknown, what: string): string {
  if (typeof value !== "string" || value === "") resync(`${what} is missing`);
  return value;
}

function upsert<T extends { id?: string | undefined }>(
  list: T[] | undefined,
  record: T | undefined,
  entityId: unknown,
): void {
  if (!list || !record) resync("entity payload is missing");
  const id = requireString(record.id ?? entityId, "entity id");
  const index = list.findIndex((item) => item.id === id);
  if (index >= 0) list[index] = record;
  else list.push(record);
}

export class EventFollower {
  /** 最近一次快照投影；未取得完整快照前为 null（质量不得置 current）。 */
  projection: AgentSnapshot | null = null;
  /** 持久游标：{streamId, epoch, after=最后已应用位置}。 */
  cursor: AgentEventCursor | null = null;
  /** 订阅/页头提示的最新 head；仅加速，不是唯一真相。 */
  head: string | null = null;
  status: FollowerStatus = "idle";
  /** 最近一次导致 stale/revoked 的错误。 */
  lastError: unknown = null;

  /** 去重键 `${epoch}:${position}` → 事件完整 JSON（同键异内容必须可判）。 */
  readonly #applied = new Map<string, string>();
  readonly #transport: AgentEventTransport;
  readonly #conversationId: string;
  readonly #limit: number;
  #more = false;
  #drain: Promise<void> | null = null;

  constructor(transport: AgentEventTransport, conversationId: string, limit = 32) {
    this.#transport = transport;
    this.#conversationId = conversationId;
    this.#limit = limit;
  }

  /** 首次连接：snapshot 原子替换投影 → 订阅 head → 从 cursor 立即续读。 */
  async start(): Promise<void> {
    this.applySnapshot(await this.#transport.snapshot(this.#conversationId));
    this.#watch();
    await this.pump();
  }

  /** 断线恢复：transport 完成重认证/重发现后调用；显式重新订阅 head 并续读旧 cursor。 */
  async recover(): Promise<void> {
    if (!this.cursor) return this.start();
    this.status = "syncing";
    this.#watch();
    await this.pump();
  }

  #watch(): void {
    this.#transport.watchHead(this.#conversationId, (head) => {
      this.head = head;
      this.pump().catch(() => { /* 状态已置 stale/revoked；观察者读 status/lastError */ });
    });
  }
  /** 快照原子替换投影与 cursor；游标必须指向同一流。 */
  applySnapshot(raw: unknown): void {
    const snapshot = (isRecord(raw) && isRecord(raw.snapshot) ? raw.snapshot : isRecord(raw) ? raw : undefined) as AgentSnapshot | undefined;
    if (!snapshot || !isRecord(snapshot.cursor)) resync("snapshot is missing its cursor");
    const cursor = snapshot.cursor as AgentEventCursor;
    const streamId = requireString(cursor.streamId, "snapshot.cursor.streamId");
    const epoch = requireString(cursor.epoch, "snapshot.cursor.epoch");
    const after = requireString(cursor.after, "snapshot.cursor.after");
    if (streamId !== this.#conversationId) {
      resync(`snapshot stream ${streamId} does not match conversation`);
    }
    if (!isUlid(epoch) || !isCanonicalUint64(after)) resync("snapshot cursor is malformed");
    this.projection = snapshot;
    if (!Array.isArray(snapshot.executions)) snapshot.executions = [];
    if (!Array.isArray(snapshot.interactions)) snapshot.interactions = [];
    if (!Array.isArray(snapshot.messages)) snapshot.messages = [];
    if (!Array.isArray(snapshot.relationships)) snapshot.relationships = [];
    if (!Array.isArray(snapshot.gaps)) snapshot.gaps = [];
    this.cursor = { streamId, epoch, after };
    this.head = after;
    this.#more = true; // 契约 §3 步骤 3：snapshot 后立即 read 一次
    this.#applied.clear();
    this.status = "syncing";
  }

  /** 一次一个 read 在途；并发调用共享同一个追赶 Promise（head 提示与 start 竞争时
   * 也不会提前返回）。 */
  pump(): Promise<void> {
    if (!this.#drain) {
      this.#drain = this.#runDrain().finally(() => { this.#drain = null; });
    }
    return this.#drain;
  }

  /** 状态机错误分类：forbidden → 停止并报告 revoked；resume_unavailable →
   * 换新快照（绝不重放命令）；reducer 协议故障原样抛出；其余 → stale 后抛出。 */
  async #runDrain(): Promise<void> {
    try {
      await this.#pumpAll();
    } catch (error) {
      const code = error instanceof ConexError ? error.code : undefined;
      if (code === ErrorCode.ERROR_CODE_FORBIDDEN) {
        this.status = "revoked"; // 停止；在途读取随 drain 清空，经 status 报告撤权
        this.lastError = error;
        return;
      }
      if (!(error instanceof ResyncRequiredError) && code === ErrorCode.ERROR_CODE_RESUME_UNAVAILABLE) {
        this.applySnapshot(await this.#transport.snapshot(this.#conversationId));
        await this.#pumpAll();
        return;
      }
      this.status = "stale";
      this.lastError = error;
      throw error;
    }
  }

  async #pumpAll(): Promise<void> {
    // 外层循环吸收读取 await 期间新到的 head 提示。
    do {
      while (this.readDue()) {
        const page = await this.#transport.read(this.cursor as AgentEventCursor, this.#limit);
        this.#more = this.applyPage(page);
        if (isRecord(page) && typeof page.head === "string") this.head = page.head;
      }
    } while (this.readDue());
    // 只有完整快照后的投影才允许回到 current。
    if (this.projection && this.status === "syncing") this.status = "current";
  }

  private readDue(): boolean {
    if (!this.cursor) return false;
    if (this.#more) return true;
    return this.head !== null && BigInt(this.head) > BigInt(this.cursor.after as string);
  }

  /** 校验页信封并按序应用事件；返回该页的 hasMore。 */
  applyPage(raw: unknown): boolean {
    const cursor = this.cursor;
    if (!cursor || !isRecord(raw)) resync("page delivered without a cursor");
    const page = raw as AgentEventReadResult;
    const next = page.nextCursor;
    if (isRecord(next)) {
      if (next.streamId !== cursor.streamId || next.epoch !== cursor.epoch) {
        resync("page cursor stream/epoch does not match");
      }
    }
    if (!Array.isArray(page.events)) resync("page.events must be an array");
    for (const event of page.events as unknown[]) this.applyEvent(event);
    if (isRecord(next) && typeof next.after === "string" && next.after !== cursor.after) {
      resync(`page nextCursor ${next.after} disagrees with applied position ${cursor.after}`);
    }
    return page.hasMore === true;
  }

  /** 同一应用步骤内更新投影与 cursor；重复页按 (streamId, epoch, position) 去重。 */
  applyEvent(raw: unknown): void {
    const cursor = this.cursor;
    if (!cursor || !isRecord(raw)) resync("event delivered without a cursor");
    const event = raw as AgentEvent;
    const epoch = requireString(event.epoch, "event.epoch");
    const position = requireString(event.position, "event.position");
    if (!isCanonicalUint64(position)) resync(`event position ${position} is not canonical`);
    if (epoch !== cursor.epoch) resync(`epoch changed from ${cursor.epoch} to ${epoch}`);
    const key = `${epoch}:${position}`;
    const content = JSON.stringify(event);
    const seen = this.#applied.get(key);
    if (seen !== undefined) {
      if (seen === content) return; // 重复交付：已应用，跳过
      resync(`duplicate position ${position} with different content`);
    }
    const expected = BigInt(cursor.after as string) + 1n;
    if (BigInt(position) !== expected) {
      resync(`position gap: expected ${expected}, got ${position}`);
    }
    this.applyData(event);
    cursor.after = position;
    this.#applied.set(key, content);
  }

  private applyData(event: AgentEvent): void {
    const projection = this.projection as AgentSnapshot;
    switch (event.type) {
      case "conversation.updated":
        if (event.conversationUpdated) projection.conversation = event.conversationUpdated;
        return;
      case "execution.updated":
        upsert(projection.executions, event.executionUpdated, event.entityId);
        return;
      case "interaction.created":
      case "interaction.updated":
        upsert(projection.interactions, event.interactionCreated ?? event.interactionUpdated, event.entityId);
        return;
      case "message.created":
        if (event.messageCreated) projection.messages?.push(event.messageCreated);
        return;
      case "message.delta":
        this.appendDelta(event.messageDelta);
        return;
      case "message.completed": {
        const completed = event.messageCompleted;
        const id = requireString(completed?.messageId ?? event.entityId, "message.completed target");
        const message = projection.messages?.find((item) => item.id === id);
        if (!message) resync(`completed for unknown message ${id}`);
        if (completed?.state) message.state = completed.state;
        if (completed?.revision) message.revision = completed.revision;
        return;
      }
      case "relationship.updated":
        upsert(projection.relationships, event.relationshipUpdated, event.entityId);
        return;
      case "recovery.gap":
        if (event.recoveryGap) projection.gaps?.push(event.recoveryGap);
        return;
      default:
        resync(`unknown event type ${String(event.type)}`);
    }
  }

  /** 文本仅经 delta 追加；offsetBytes 必须等于该 part 当前已知 totalBytes。 */
  private appendDelta(delta: AgentMessageDelta | undefined): void {
    if (!delta) resync("message.delta payload is missing");
    const messageId = requireString(delta.messageId, "delta.messageId");
    // proto3 JSON omits default values: absent partIndex is 0, absent
    // offsetBytes is "0". Treating them as absent-and-unknown would resync on
    // every legitimate first delta.
    const partIndex = delta.partIndex === undefined ? 0 : delta.partIndex;
    if (typeof partIndex !== "number" || !Number.isInteger(partIndex) || partIndex < 0) {
      resync("delta.partIndex is invalid");
    }
    const offset = delta.offsetBytes === undefined
      ? "0"
      : requireString(delta.offsetBytes, "delta.offsetBytes");
    if (typeof delta.text !== "string") resync("delta.text is missing");
    const message = (this.projection as AgentSnapshot).messages?.find((item) => item.id === messageId);
    if (!message) resync(`delta for unknown message ${messageId}`);
    if (!Array.isArray(message.parts)) message.parts = [];
    const parts = message.parts as AgentMessagePart[];
    if (partIndex > parts.length) resync(`delta partIndex ${partIndex} leaves a part gap`);
    let part = parts[partIndex];
    if (!part) {
      if (partIndex !== parts.length) resync(`delta partIndex ${partIndex} leaves a part gap`);
      part = { text: { text: "", offsetBytes: "0", totalBytes: "0" } };
      parts.push(part);
    }
    const textPart = part.text;
    if (!textPart) resync(`delta targets non-text part ${partIndex}`);
    const total = textPart.totalBytes ?? "0";
    if (offset !== total) {
      resync(`delta offset ${offset} does not continue part at ${total}`);
    }
    textPart.text = (textPart.text ?? "") + delta.text;
    if (textPart.offsetBytes === undefined) textPart.offsetBytes = "0";
    textPart.totalBytes = (BigInt(total) + BigInt(encoder.encode(delta.text).length)).toString();
  }
}
