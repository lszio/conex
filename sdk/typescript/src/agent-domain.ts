/**
 * Agent 领域边界：方法目录、执行类别与业务 input/output 严格校验
 * （docs/contracts/agent-domain.md §1-§4、§9；docs/contracts/event-recovery.md §2-§3）。
 *
 * 与 crates/conex-proto/src/agent_domain.rs 实现同一规则集，由
 * conformance/vectors/agent-domain.json 共享向量仲裁一致。结构形状复用
 * s1-wire 的单一 JSON Schema 源（validateDomainSchema）；本模块追加语义规则：
 * ULID 标识符、规范 uint64 十进制字符串、状态词表、UTF-8 字节长度上限、
 * decision 互斥与 cursor 上限（SQLite 首版 0..i64::MAX）。
 */

import { ConexError } from "./errors";
import { ErrorCode } from "./generated/conex/common";
import { isCanonicalUint64, isRecord, isUlid, validateDomainSchema } from "./s1-wire";
import type { ExecutionClass } from "./capability";

export const AGENT_ENDPOINTS_LIST = "agent/endpoints/list";
export const AGENT_WORKSPACES_LIST = "agent/workspaces/list";
export const AGENT_CONVERSATIONS_LIST = "agent/conversations/list";
export const AGENT_CONVERSATIONS_CREATE = "agent/conversations/create";
export const AGENT_CONVERSATIONS_GET = "agent/conversations/get";
export const AGENT_CONVERSATIONS_HISTORY = "agent/conversations/history";
export const AGENT_MESSAGES_READ = "agent/messages/read";
export const AGENT_EXECUTIONS_SUBMIT = "agent/executions/submit";
export const AGENT_EXECUTIONS_GET = "agent/executions/get";
export const AGENT_EXECUTIONS_CANCEL = "agent/executions/cancel";
export const AGENT_EXECUTIONS_STEER = "agent/executions/steer";
export const AGENT_INTERACTIONS_LIST = "agent/interactions/list";
export const AGENT_INTERACTIONS_RESPOND = "agent/interactions/respond";
export const AGENT_RELATIONSHIPS_LIST = "agent/relationships/list";
export const AGENT_SUBAGENTS_LIST = "agent/subagents/list";
export const AGENT_SUBAGENTS_CANCEL = "agent/subagents/cancel";
export const EVENTS_SNAPSHOT = "events/snapshot";
export const EVENTS_READ = "events/read";

/** 完整 agent 领域方法目录（契约 §4 操作表，18 个）。经现有
 * module/invoke 的 input:{method,argument} 分派，不是顶层 JSON-RPC 方法。 */
export const AGENT_METHODS = [
  AGENT_ENDPOINTS_LIST,
  AGENT_WORKSPACES_LIST,
  AGENT_CONVERSATIONS_LIST,
  AGENT_CONVERSATIONS_CREATE,
  AGENT_CONVERSATIONS_GET,
  AGENT_CONVERSATIONS_HISTORY,
  AGENT_MESSAGES_READ,
  AGENT_EXECUTIONS_SUBMIT,
  AGENT_EXECUTIONS_GET,
  AGENT_EXECUTIONS_CANCEL,
  AGENT_EXECUTIONS_STEER,
  AGENT_INTERACTIONS_LIST,
  AGENT_INTERACTIONS_RESPOND,
  AGENT_RELATIONSHIPS_LIST,
  AGENT_SUBAGENTS_LIST,
  AGENT_SUBAGENTS_CANCEL,
  EVENTS_SNAPSHOT,
  EVENTS_READ,
] as const;

export type AgentMethod = (typeof AGENT_METHODS)[number];

export function isAgentMethod(method: string): method is AgentMethod {
  return (AGENT_METHODS as readonly string[]).includes(method);
}

/** 契约 §4：read_only 为列表/get/history/events，其余为 non_replayable。 */
export function agentExecutionClass(method: string): ExecutionClass | undefined {
  if (!isAgentMethod(method)) return undefined;
  switch (method) {
    case AGENT_CONVERSATIONS_CREATE:
    case AGENT_EXECUTIONS_SUBMIT:
    case AGENT_EXECUTIONS_CANCEL:
    case AGENT_EXECUTIONS_STEER:
    case AGENT_INTERACTIONS_RESPOND:
    case AGENT_SUBAGENTS_CANCEL:
      return "non_replayable";
    default:
      return "read_only";
  }
}

/** 领域权限词表（契约 §1）；权限还须约束节点、模块、Workspace、Conversation。 */
export function requiredPermission(method: string): string | undefined {
  switch (method) {
    case AGENT_CONVERSATIONS_CREATE: return "agent.create";
    case AGENT_EXECUTIONS_SUBMIT:
    case AGENT_EXECUTIONS_STEER: return "agent.execute";
    case AGENT_EXECUTIONS_CANCEL:
    case AGENT_SUBAGENTS_CANCEL: return "agent.cancel";
    case AGENT_INTERACTIONS_RESPOND: return "agent.interact";
    default: return isAgentMethod(method) ? "agent.read" : undefined;
  }
}

const MAX_PAGE_BYTES = 128;
const MAX_CONTENT_BYTES = 65536;
const MAX_MESSAGE_READ_BYTES = 65536;
/// SQLite 首版只使用 0..i64::MAX 的位置（恢复契约 §2）。
const MAX_POSITION = 9223372036854775807n;

const EXECUTION_STATES = ["accepted", "dispatching", "running", "unknown", "succeeded", "failed", "cancelled"];
const INTERACTION_STATES = ["pending", "responding", "resolved", "expired", "invalidated", "unknown"];
const MESSAGE_STATES = ["streaming", "complete", "incomplete"];
const ROLES = ["user", "assistant", "tool", "system"];
const QUALITIES = ["current", "stale", "unknown", "historical"];
const SOURCES = ["backend", "config", "user"];
const DECISION_KINDS = ["approval", "selection", "input"];
const EFFECTS = ["allow_once", "deny", "select", "submit"];
const AVAILABILITY = ["ready", "unreachable", "unknown", "stopped"];
const CATALOG_SCOPES = ["node-managed", "backend"];
const ADAPTERS = ["omp", "codex"];

/** 规范 uint64 字段名：输出侧逐键校验十进制规范形（无前导零、<=u64::MAX）。 */
const UINT64_KEYS = [
  "generation", "revision", "createdAtMs", "updatedAtMs", "cancelRequestedAtMs",
  "expiresAtMs", "expectedRevision", "after", "position", "head", "firstAvailable",
  "offsetBytes", "nextOffsetBytes", "totalBytes", "recordedAtMs",
];

const VOCAB: Record<string, string[][]> = {
  state: [EXECUTION_STATES, INTERACTION_STATES, MESSAGE_STATES],
  kind: [DECISION_KINDS],
  effect: [EFFECTS],
  role: [ROLES],
  quality: [QUALITIES],
  source: [SOURCES],
  availability: [AVAILABILITY],
  catalogScope: [CATALOG_SCOPES],
  adapter: [ADAPTERS],
};

function fail(message: string): never {
  throw new ConexError(message, {
    code: ErrorCode.ERROR_CODE_BAD_REQUEST,
    execution: "not_started",
    retry: "never",
  });
}

function asObject(value: unknown, what: string): Record<string, unknown> {
  if (!isRecord(value)) fail(`${what} must be a JSON object`);
  return value;
}

/** 缺失与 null 等价（镜像 Rust get() 的 null 过滤）。 */
function get(map: Record<string, unknown>, key: string): unknown {
  const value = map[key];
  return value === null ? undefined : value;
}

function requireUlid(map: Record<string, unknown>, key: string): void {
  const value = get(map, key);
  if (value === undefined) fail(`${key} is required`);
  if (typeof value !== "string") fail(`${key} must be a string`);
  if (!isUlid(value)) fail(`${key} must be a ULID`);
}

function requireBounded(map: Record<string, unknown>, key: string, maxBytes: number): void {
  const value = get(map, key);
  if (value === undefined) fail(`${key} is required`);
  if (typeof value !== "string") fail(`${key} must be a string`);
  if (value === "") fail(`${key} must not be empty`);
  if (Buffer.byteLength(value, "utf8") > maxBytes) {
    fail(`${key} exceeds ${maxBytes} UTF-8 bytes`);
  }
}

function optionalBounded(map: Record<string, unknown>, key: string, maxBytes: number): void {
  if (get(map, key) !== undefined) requireBounded(map, key, maxBytes);
}

function requireUint64(value: unknown, what: string, max = 18446744073709551615n): void {
  if (typeof value !== "string" || !isCanonicalUint64(value) || BigInt(value) > max) {
    fail(`${what} must be a canonical uint64 string`);
  }
}

/** 可选页参数：limit 默认 50、范围 1–100；pageToken 上限 128 UTF-8 字节。 */
function validatePage(map: Record<string, unknown>): void {
  const limit = get(map, "limit");
  if (limit !== undefined) {
    if (typeof limit !== "number" || !Number.isInteger(limit) || limit < 0) {
      fail("limit must be a non-negative integer");
    }
    if (limit < 1 || limit > 100) fail("limit must be within 1..=100");
  }
  optionalBounded(map, "pageToken", MAX_PAGE_BYTES);
}

function validateDecision(decision: unknown): void {
  const map = asObject(decision, "decision");
  const kind = get(map, "kind");
  if (typeof kind !== "string") fail("decision.kind is required");
  if (!DECISION_KINDS.includes(kind)) {
    fail("decision.kind must be approval/selection/input");
  }
  const choiceId = get(map, "choiceId");
  const choiceIds = get(map, "choiceIds");
  const text = get(map, "text");
  if (kind === "approval") {
    if (choiceId === undefined || choiceIds !== undefined || text !== undefined) {
      fail("approval decision requires exactly choiceId");
    }
    requireBounded(map, "choiceId", MAX_PAGE_BYTES);
  } else if (kind === "selection") {
    if (choiceIds === undefined || choiceId !== undefined || text !== undefined) {
      fail("selection decision requires exactly choiceIds");
    }
    if (!Array.isArray(choiceIds)) fail("choiceIds must be an array");
    if (choiceIds.length === 0) {
      fail("selection decision requires at least one choiceId");
    }
    for (const id of choiceIds) {
      if (typeof id !== "string" || id === "") {
        fail("choiceIds entries must be non-empty strings");
      }
    }
  } else {
    if (text === undefined || choiceId !== undefined || choiceIds !== undefined) {
      fail("input decision requires exactly text");
    }
    requireBounded(map, "text", MAX_CONTENT_BYTES);
  }
}

function validateCursor(cursor: unknown): void {
  const map = asObject(cursor, "cursor");
  requireUlid(map, "streamId");
  requireUlid(map, "epoch");
  const after = get(map, "after");
  if (after === undefined) fail("cursor.after is required");
  requireUint64(after, "cursor.after", MAX_POSITION);
}

const INPUT_MESSAGES: Record<AgentMethod, string> = {
  [AGENT_ENDPOINTS_LIST]: "conex.AgentEndpointListInput",
  [AGENT_WORKSPACES_LIST]: "conex.AgentWorkspaceListInput",
  [AGENT_CONVERSATIONS_LIST]: "conex.AgentConversationListInput",
  [AGENT_CONVERSATIONS_CREATE]: "conex.AgentConversationCreateInput",
  [AGENT_CONVERSATIONS_GET]: "conex.AgentConversationGetInput",
  [AGENT_CONVERSATIONS_HISTORY]: "conex.AgentConversationHistoryInput",
  [AGENT_MESSAGES_READ]: "conex.AgentMessageReadInput",
  [AGENT_EXECUTIONS_SUBMIT]: "conex.AgentSubmitInput",
  [AGENT_EXECUTIONS_GET]: "conex.AgentExecutionGetInput",
  [AGENT_EXECUTIONS_CANCEL]: "conex.AgentExecutionCancelInput",
  [AGENT_EXECUTIONS_STEER]: "conex.AgentExecutionSteerInput",
  [AGENT_INTERACTIONS_LIST]: "conex.AgentInteractionListInput",
  [AGENT_INTERACTIONS_RESPOND]: "conex.AgentInteractionRespondInput",
  [AGENT_RELATIONSHIPS_LIST]: "conex.AgentRelationshipListInput",
  [AGENT_SUBAGENTS_LIST]: "conex.AgentSubagentListInput",
  [AGENT_SUBAGENTS_CANCEL]: "conex.AgentSubagentCancelInput",
  [EVENTS_SNAPSHOT]: "conex.AgentSnapshotInput",
  [EVENTS_READ]: "conex.AgentEventReadInput",
};

const RESULT_MESSAGES: Record<AgentMethod, string> = {
  [AGENT_ENDPOINTS_LIST]: "conex.AgentEndpointListResult",
  [AGENT_WORKSPACES_LIST]: "conex.AgentWorkspaceListResult",
  [AGENT_CONVERSATIONS_LIST]: "conex.AgentConversationListResult",
  [AGENT_CONVERSATIONS_CREATE]: "conex.AgentConversationCreateResult",
  [AGENT_CONVERSATIONS_GET]: "conex.AgentConversationGetResult",
  [AGENT_CONVERSATIONS_HISTORY]: "conex.AgentConversationHistoryResult",
  [AGENT_MESSAGES_READ]: "conex.AgentMessageReadResult",
  [AGENT_EXECUTIONS_SUBMIT]: "conex.AgentSubmitResult",
  [AGENT_EXECUTIONS_GET]: "conex.AgentExecutionGetResult",
  [AGENT_EXECUTIONS_CANCEL]: "conex.AgentExecutionCancelResult",
  [AGENT_EXECUTIONS_STEER]: "conex.AgentExecutionSteerResult",
  [AGENT_INTERACTIONS_LIST]: "conex.AgentInteractionListResult",
  [AGENT_INTERACTIONS_RESPOND]: "conex.AgentInteractionRespondResult",
  [AGENT_RELATIONSHIPS_LIST]: "conex.AgentRelationshipListResult",
  [AGENT_SUBAGENTS_LIST]: "conex.AgentSubagentListResult",
  [AGENT_SUBAGENTS_CANCEL]: "conex.AgentSubagentCancelResult",
  [EVENTS_SNAPSHOT]: "conex.AgentSnapshotResult",
  [EVENTS_READ]: "conex.AgentEventReadResult",
};

/** Agent 领域 input 严格校验：生成 Schema 形状（含未知字段拒绝）+ 契约语义规则。 */
export function validateAgentInput(method: string, argument: unknown): void {
  const message = INPUT_MESSAGES[method as AgentMethod];
  if (!message) fail(`${method} is not an agent method`);
  validateDomainSchema(message, argument);
  const map = asObject(argument, "argument");
  switch (method) {
    case AGENT_ENDPOINTS_LIST:
      validatePage(map);
      return;
    case AGENT_WORKSPACES_LIST:
    case AGENT_CONVERSATIONS_LIST:
      requireUlid(map, "endpointId");
      if (get(map, "workspaceId") !== undefined) requireUlid(map, "workspaceId");
      validatePage(map);
      return;
    case AGENT_CONVERSATIONS_CREATE:
      requireUlid(map, "endpointId");
      requireUlid(map, "workspaceId");
      requireBounded(map, "idempotencyKey", MAX_PAGE_BYTES);
      optionalBounded(map, "title", MAX_PAGE_BYTES);
      return;
    case AGENT_CONVERSATIONS_GET:
    case EVENTS_SNAPSHOT:
      requireUlid(map, "conversationId");
      return;
    case AGENT_CONVERSATIONS_HISTORY:
      requireUlid(map, "conversationId");
      if (get(map, "beforeMessageId") !== undefined) requireUlid(map, "beforeMessageId");
      validatePage(map);
      return;
    case AGENT_MESSAGES_READ: {
      requireUlid(map, "conversationId");
      requireUlid(map, "messageId");
      const offset = get(map, "offsetBytes");
      if (offset === undefined) fail("offsetBytes is required");
      requireUint64(offset, "offsetBytes");
      const max = get(map, "maxBytes");
      if (max !== undefined) {
        if (typeof max !== "number" || !Number.isInteger(max) || max < 0) {
          fail("maxBytes must be a non-negative integer");
        }
        if (max === 0 || max > MAX_MESSAGE_READ_BYTES) {
          fail("maxBytes must be within 1..=65536");
        }
      }
      return;
    }
    case AGENT_EXECUTIONS_SUBMIT:
      requireUlid(map, "conversationId");
      requireBounded(map, "idempotencyKey", MAX_PAGE_BYTES);
      requireBounded(map, "content", MAX_CONTENT_BYTES);
      return;
    case AGENT_EXECUTIONS_GET:
      requireUlid(map, "executionId");
      return;
    case AGENT_EXECUTIONS_CANCEL:
    case AGENT_SUBAGENTS_CANCEL:
      requireUlid(map, "executionId");
      requireUint64(get(map, "expectedGeneration"), "expectedGeneration");
      return;
    case AGENT_EXECUTIONS_STEER:
      requireUlid(map, "executionId");
      requireUint64(get(map, "expectedGeneration"), "expectedGeneration");
      requireBounded(map, "idempotencyKey", MAX_PAGE_BYTES);
      requireBounded(map, "content", MAX_CONTENT_BYTES);
      return;
    case AGENT_INTERACTIONS_LIST:
      requireUlid(map, "conversationId");
      if (get(map, "executionId") !== undefined) requireUlid(map, "executionId");
      validatePage(map);
      return;
    case AGENT_INTERACTIONS_RESPOND:
      requireUlid(map, "interactionId");
      requireUint64(get(map, "expectedRevision"), "expectedRevision");
      if (get(map, "decision") === undefined) fail("decision is required");
      validateDecision(get(map, "decision"));
      return;
    case AGENT_RELATIONSHIPS_LIST:
      requireUlid(map, "conversationId");
      validatePage(map);
      return;
    case AGENT_SUBAGENTS_LIST:
      requireUlid(map, "executionId");
      validatePage(map);
      return;
    case EVENTS_READ: {
      const cursor = get(map, "cursor");
      if (cursor === undefined) fail("cursor is required");
      validateCursor(cursor);
      const limit = get(map, "limit");
      if (limit !== undefined) {
        if (typeof limit !== "number" || !Number.isInteger(limit) || limit < 0) {
          fail("limit must be a non-negative integer");
        }
        if (limit < 1 || limit > 32) fail("events/read limit must be within 1..=32");
      }
      return;
    }
    default:
      fail(`${method} is not an agent method`);
  }
}

/** 词表与规范 uint64 逐键检查；形状已由生成 Schema 保证，这里只追加语义。 */
function checkSemantics(value: unknown): void {
  if (Array.isArray(value)) {
    for (const item of value) checkSemantics(item);
    return;
  }
  if (!isRecord(value)) return;
  for (const [key, child] of Object.entries(value)) {
    if (child === null || child === undefined) continue;
    if (UINT64_KEYS.includes(key)) {
      if (typeof child !== "string" || !isCanonicalUint64(child)) {
        fail(`${key} must be a canonical uint64 string`);
      }
    }
    const vocab = VOCAB[key];
    if (vocab && typeof child === "string" && !vocab.some((list) => list.includes(child))) {
      fail(`${key} value ${JSON.stringify(child)} is outside the contract vocabulary`);
    }
    checkSemantics(child);
  }
}

/** Agent 领域 output 严格校验：生成 Schema 形状 + 规范 uint64 与状态词表。 */
export function validateAgentOutput(method: string, result: unknown): void {
  const message = RESULT_MESSAGES[method as AgentMethod];
  if (!message) fail(`${method} is not an agent method`);
  validateDomainSchema(message, result);
  checkSemantics(result);
}

/** 共享向量值展开：{"repeat":{"char":c,"count":n},"append":s} → c×n 后接 s；
 * 按字符重复，UTF-8 字节长度由验证器按编码后长度测量。 */
export function expandVectorValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(expandVectorValue);
  if (!isRecord(value)) return value;
  const repeat = value.repeat;
  if (isRecord(repeat) && typeof repeat.char === "string" && typeof repeat.count === "number") {
    const append = typeof value.append === "string" ? value.append : "";
    return repeat.char.repeat(repeat.count) + append;
  }
  const out: Record<string, unknown> = {};
  for (const [key, child] of Object.entries(value)) out[key] = expandVectorValue(child);
  return out;
}

/** invoke 钩子：CapabilitySession.invoke 的最小形态；不引入第二套 service locator。 */
export interface AgentInvokeTransport {
  invoke(method: string, argument: unknown): Promise<unknown>;
}

/** Agent 领域消费者：所有公共操作经严格 input/output 校验后走现有 invoke 路径。 */
export class AgentClient {
  constructor(private readonly transport: AgentInvokeTransport) {}

  async invoke<I, O>(method: AgentMethod, argument: I): Promise<O> {
    validateAgentInput(method, argument);
    const result = await this.transport.invoke(method, argument);
    validateAgentOutput(method, result);
    return result as O;
  }
}
