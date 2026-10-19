/**
 * S1 能力边界编解码（docs/contracts/s1-capabilities.md §4-§9）。
 *
 * 镜像 crates/conex-proto/src/capability.rs 与 crates/conex-proto/src/wire.rs：
 * 相同的方法目录、任意深度重复键拒绝、按方向的 context 规则、input/result
 * 形状规则、规范 uint64 / ULID / state/end Error JSON 规则。一致由共享向量
 * conformance/vectors/s1/capabilities.json 仲裁（两端读同一份文件）。
 *
 * 结构字段来自随 SDK 打包的生成 JSON Schema；此处只补协议语义约束，
 * 不维护第二份消息结构或错误编号目录。
 */

import { Error, ErrorCode, errorCodeFromJSON, errorCodeToJSON } from "./generated/conex/common";
import schemaDocument from "./generated/schema.json";

export const METHOD_MODULE_REGISTER = "module/register";
export const METHOD_MODULE_UNREGISTER = "module/unregister";
export const METHOD_MODULE_LIST = "module/list";
export const METHOD_MODULE_INVOKE = "module/invoke";
export const METHOD_MODULE_CANCEL = "module/cancel";
export const METHOD_STATE_SUBSCRIBE = "state/subscribe";
export const METHOD_STATE_UNSUBSCRIBE = "state/unsubscribe";
export const METHOD_STATE_UPDATE = "state/update";
export const METHOD_STATE_END = "state/end";

export const S1_METHODS = [
  METHOD_MODULE_REGISTER,
  METHOD_MODULE_UNREGISTER,
  METHOD_MODULE_LIST,
  METHOD_MODULE_INVOKE,
  METHOD_MODULE_CANCEL,
  METHOD_STATE_SUBSCRIBE,
  METHOD_STATE_UNSUBSCRIBE,
  METHOD_STATE_UPDATE,
  METHOD_STATE_END,
] as const;

export function isS1Method(method: string): boolean {
  return (S1_METHODS as readonly string[]).includes(method);
}

export function isS1Request(method: string): boolean {
  return isS1Method(method) && method !== METHOD_STATE_UPDATE && method !== METHOD_STATE_END;
}

export function isS1Notification(method: string): boolean {
  return method === METHOD_STATE_UPDATE || method === METHOD_STATE_END;
}

/** 帧来源方向：client = 不受信链路，host = 中心盖章转发。 */
export type S1Boundary = "client" | "host";

export const RPC_PARSE_ERROR = -32700;
export const RPC_BAD_REQUEST = -32602;
export const RPC_PLANE_MISMATCH = -32009;

/** 边界错误：rpcCode 与 Rust ProtocolError.rpc_code 对齐。 */
export class S1WireError extends globalThis.Error {
  readonly rpcCode: number;
  constructor(rpcCode: number, message: string) {
    super(message);
    this.name = "S1WireError";
    this.rpcCode = rpcCode;
  }
}

/** Crockford base32 ULID：26 字符、首字符至多 '7'（与 wire.rs is_ulid 相同）。 */
export function isUlid(value: string): boolean {
  const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  if (value.length !== 26 || value.charCodeAt(0) > 55 /* '7' */) return false;
  for (const c of value) {
    if (!ALPHABET.includes(c)) return false;
  }
  return true;
}

/** 规范 uint64 十进制字符串："0" 或不带前导零、不超过 u64::MAX（契约 §2）。 */
export function isCanonicalUint64(s: string): boolean {
  if (!/^[0-9]+$/.test(s)) return false;
  if (s !== "0" && s.startsWith("0")) return false;
  return BigInt(s) <= 18446744073709551615n;
}

/**
 * 严格 JSON 解析：任意深度的重复对象键都拒绝（serde_json 的 Map 是 last-wins，
 * Rust 边界用 StrictValue 在解析期拒绝；此处扫描原始文本做等价检测）。
 */
export function parseStrict(text: string): unknown {
  try {
    rejectDuplicateKeys(text);
    const value: unknown = JSON.parse(text, (_key, value: unknown) => {
      if (typeof value === "number" && !Number.isFinite(value)) throw new S1WireError(RPC_PARSE_ERROR, "non-finite JSON number");
      return value;
    });
    return value;
  } catch (e) {
    if (e instanceof S1WireError) throw e;
    throw new S1WireError(RPC_PARSE_ERROR, `invalid JSON: ${String(e)}`);
  }
}

function rejectDuplicateKeys(text: string): void {
  const stack: Set<string>[] = [];
  let inString = false;
  let i = 0;
  let depth = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (inString) {
      if (c === "\\") {
        i += 2;
        continue;
      }
      if (c === '"') inString = false;
      i += 1;
      continue;
    }
    if (c === '"') {
      // 字符串字面量；对象顶层且后随 ':' 的字符串是键。
      let j = i + 1;
      while (j < n && text[j] !== '"') {
        if (text[j] === "\\") j += 1;
        j += 1;
      }
      let k = j + 1;
      while (k < n && (text[k] === " " || text[k] === "\t" || text[k] === "\n" || text[k] === "\r")) {
        k += 1;
      }
      if (stack.length > 0 && k < n && text[k] === ":") {
        const key = JSON.parse(text.slice(i, j + 1)) as string; // 转义后语义键
        const top = stack[stack.length - 1];
        if (top.has(key)) throw new S1WireError(RPC_PARSE_ERROR, `duplicate field ${key}`);
        top.add(key);
      }
      inString = true;
      i += 1;
      continue;
    }
    if (c === "{" || c === "[") {
      if (++depth >= 128) throw new S1WireError(RPC_PARSE_ERROR, "JSON nesting limit exceeded");
    } else if (c === "}" || c === "]") depth -= 1;
    if (c === "{") stack.push(new Set());
    else if (c === "}") stack.pop();
    i += 1;
  }
}

export interface S1Context {
  providerEndpointId?: string;
  plane?: string;
  bindingId?: string;
  principalId?: string;
  tenantId?: string;
  spaceId?: string;
  registrationId?: string;
  [key: string]: unknown;
}

export interface S1Params {
  context: S1Context;
  timeoutBudgetMs?: number;
  input?: unknown;
}

export interface S1Frame {
  kind: "request" | "notification" | "success" | "failure";
  id?: string | null;
  method?: string;
  params?: S1Params;
  result?: unknown;
  error?: unknown;
}

const PARAM_KEYS: Record<string, true> = { context: true, timeoutBudgetMs: true, input: true };
const CONTEXT_KEYS: Record<string, true> = {
  providerEndpointId: true,
  plane: true,
  bindingId: true,
  principalId: true,
  tenantId: true,
  spaceId: true,
  registrationId: true,
};

/** 本包边界的规范对象守卫（仅此一处定义，供 s1-wire 与测试共用）。 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * 信封解码（镜像 wire.rs decode_wire）：重复键、ULID id、无 batch、
 * params/context 未知字段与 S1 通知省略 timeoutBudgetMs 都在此拒绝。
 */
export function decodeS1Frame(text: string): S1Frame {
  const value = parseStrict(text);
  if (!isRecord(value)) throw new S1WireError(RPC_PARSE_ERROR, "jsonrpc envelope must be an object");
  if (value.jsonrpc !== "2.0") throw new S1WireError(RPC_BAD_REQUEST, "jsonrpc must be the string 2.0");
  const envelopeKeys = ["jsonrpc", "id", "method", "params", "result", "error"];
  for (const key of Object.keys(value)) {
    if (!envelopeKeys.includes(key)) throw new S1WireError(RPC_PARSE_ERROR, `unknown envelope field ${key}`);
  }
  if (Object.hasOwn(value, "method") && typeof value.method !== "string") {
    throw new S1WireError(RPC_PARSE_ERROR, "method must be a string");
  }
  const hasId = Object.hasOwn(value, "id");
  const hasMethod = typeof value.method === "string";
  const hasResult = Object.hasOwn(value, "result");
  const hasError = Object.hasOwn(value, "error");
  if (hasMethod) {
    if (hasResult || hasError) throw new S1WireError(RPC_BAD_REQUEST, "method cannot appear with result or error");
    const method = value.method as string;
    if (hasId) {
      if (typeof value.id !== "string" || !isUlid(value.id)) {
        throw new S1WireError(RPC_BAD_REQUEST, "request id must be a ULID string");
      }
      return { kind: "request", id: value.id, method, params: decodeParams(method, value.params) };
    }
    return { kind: "notification", method, params: decodeParams(method, value.params) };
  }
  if (hasResult && hasError) throw new S1WireError(RPC_BAD_REQUEST, "success and error cannot both be present");
  const id = value.id;
  if (id === undefined) throw new S1WireError(RPC_BAD_REQUEST, "response requires an id");
  if (typeof id === "string") {
    if (!isUlid(id)) throw new S1WireError(RPC_BAD_REQUEST, "response id must be a ULID");
  } else if (id !== null) {
    throw new S1WireError(RPC_BAD_REQUEST, "response id must be a ULID string or null");
  }
  if (hasResult) {
    if (id === null) throw new S1WireError(RPC_BAD_REQUEST, "success requires a non-null id");
    return { kind: "success", id: id as string, result: value.result };
  }
  if (hasError) return { kind: "failure", id: id as string | null, error: value.error };
  throw new S1WireError(RPC_BAD_REQUEST, "response has neither result nor error");
}

function decodeParams(method: string, params: unknown): S1Params {
  if (!isRecord(params)) throw new S1WireError(RPC_BAD_REQUEST, "invalid params");
  for (const key of Object.keys(params)) {
    if (!Object.hasOwn(PARAM_KEYS, key)) throw new S1WireError(RPC_BAD_REQUEST, `unknown params field ${key}`);
  }
  const context = params.context;
  if (!isRecord(context)) throw new S1WireError(RPC_BAD_REQUEST, "params requires context");
  for (const key of Object.keys(context)) {
    if (!Object.hasOwn(CONTEXT_KEYS, key)) throw new S1WireError(RPC_BAD_REQUEST, `unknown context field ${key}`);
    if (typeof context[key] !== "string") throw new S1WireError(RPC_BAD_REQUEST, `context.${key} must be a string`);
  }
  if (typeof context.plane !== "string") throw new S1WireError(RPC_BAD_REQUEST, "context requires plane");
  const budget = params.timeoutBudgetMs;
  if (budget !== undefined && (typeof budget !== "number" || !Number.isInteger(budget) || budget < 0 || budget > 4294967295)) {
    throw new S1WireError(RPC_BAD_REQUEST, "timeoutBudgetMs must be a u32");
  }
  if (isS1Notification(method)) {
    if (budget !== undefined) throw new S1WireError(RPC_BAD_REQUEST, "S1 notifications must omit timeoutBudgetMs");
    if (params.input === undefined) throw new S1WireError(RPC_BAD_REQUEST, "S1 notifications require input");
  }
  return { context: context as S1Context, timeoutBudgetMs: budget, input: params.input };
}

/**
 * S1 帧总闸（镜像 validate_message + capability::validate_frame）：
 * 形态（请求/通知）、预算、context 与 input。非 S1 方法原样放行。
 */
export function validateFrame(frame: S1Frame, boundary: S1Boundary = "client"): void {
  if (frame.kind !== "request" && frame.kind !== "notification") return;
  const method = frame.method ?? "";
  if (!isS1Method(method)) return;
  const isRequest = frame.kind === "request";
  if (isS1Request(method) !== isRequest) {
    throw new S1WireError(
      RPC_BAD_REQUEST,
      isRequest ? `${method} is a notification and must not carry an id` : `${method} requires the request form (with a JSON-RPC id)`,
    );
  }
  if (frame.params === undefined) throw new S1WireError(RPC_BAD_REQUEST, "S1 frames require params");
  if (isRequest && (typeof frame.params.timeoutBudgetMs !== "number" || !Number.isInteger(frame.params.timeoutBudgetMs) || frame.params.timeoutBudgetMs <= 0 || frame.params.timeoutBudgetMs > 4294967295)) {
    throw new S1WireError(RPC_BAD_REQUEST, "timeoutBudgetMs must be greater than 0");
  }
  if (!isRequest && frame.params.timeoutBudgetMs !== undefined) {
    throw new S1WireError(RPC_BAD_REQUEST, "S1 notifications must omit timeoutBudgetMs");
  }
  validateContext(method, frame.params.context, boundary);
  if (frame.params.input === undefined) throw new S1WireError(RPC_BAD_REQUEST, "S1 frames require input");
  validateInput(method, frame.params.input);
}

/** context 字段规则（契约 §5.2、§6.3、§7.3）。 */
export function validateContext(method: string, context: S1Context | undefined, boundary: S1Boundary = "client"): void {
  if (context === undefined) throw new S1WireError(RPC_BAD_REQUEST, "S1 frames require context");
  for (const key of Object.keys(context)) {
    if (!Object.hasOwn(CONTEXT_KEYS, key) || typeof context[key] !== "string") {
      throw new S1WireError(RPC_BAD_REQUEST, `invalid context field ${key}`);
    }
  }
  if (context.plane !== "broker") throw new S1WireError(RPC_PLANE_MISMATCH, "S1 requires the broker plane");
  if (boundary === "client") {
    if (Object.hasOwn(context, "principalId") || Object.hasOwn(context, "tenantId")) {
      throw new S1WireError(RPC_BAD_REQUEST, "clients must not supply principalId or tenantId");
    }
  }
  const endpoint = typeof context.providerEndpointId === "string" ? context.providerEndpointId : "";
  const space = typeof context.spaceId === "string" ? context.spaceId : "";
  const registration = typeof context.registrationId === "string" ? context.registrationId : "";
  const isControl =
    method === METHOD_MODULE_REGISTER ||
    method === METHOD_MODULE_UNREGISTER ||
    method === METHOD_MODULE_LIST;
  if (isControl) {
    if (["providerEndpointId", "spaceId", "registrationId", "principalId", "tenantId"].some((key) => Object.hasOwn(context, key))) {
      throw new S1WireError(RPC_BAD_REQUEST, "control method context must not carry providerEndpointId/spaceId/registrationId");
    }
    return;
  }
  if (endpoint === "") throw new S1WireError(RPC_BAD_REQUEST, "context requires providerEndpointId");
  if (space === "") throw new S1WireError(RPC_BAD_REQUEST, "context requires spaceId");
  if (typeof context.registrationId !== "string" || !isUlid(context.registrationId)) {
    throw new S1WireError(RPC_BAD_REQUEST, "context registrationId must be a ULID");
  }
}

const INPUT_MESSAGES: Record<string, string> = {
  [METHOD_MODULE_REGISTER]: "conex.ModuleRegisterInput",
  [METHOD_MODULE_UNREGISTER]: "conex.ModuleUnregisterInput",
  [METHOD_MODULE_LIST]: "conex.ModuleListInput",
  [METHOD_MODULE_INVOKE]: "conex.ModuleInvokeInput",
  [METHOD_MODULE_CANCEL]: "conex.ModuleCancelInput",
  [METHOD_STATE_SUBSCRIBE]: "conex.StateSubscribeInput",
  [METHOD_STATE_UNSUBSCRIBE]: "conex.StateUnsubscribeInput",
  [METHOD_STATE_UPDATE]: "conex.StateUpdateInput",
  [METHOD_STATE_END]: "conex.StateEndInput",
};

const RESULT_MESSAGES: Record<string, string> = {
  [METHOD_MODULE_REGISTER]: "conex.ModuleRegisterResult",
  [METHOD_MODULE_UNREGISTER]: "conex.ModuleUnregisterResult",
  [METHOD_MODULE_LIST]: "conex.ModuleListResult",
  [METHOD_MODULE_CANCEL]: "conex.ModuleCancelResult",
  [METHOD_STATE_SUBSCRIBE]: "conex.StateSubscribeResult",
  [METHOD_STATE_UNSUBSCRIBE]: "conex.StateUnsubscribeResult",
};

/** Descriptor-derived schema subset, matching Rust validation.rs. */
function schemaAccepts(schema: unknown, value: unknown): boolean {
  if (typeof schema === "boolean") return schema;
  if (!isRecord(schema)) return false;
  if (typeof schema.$ref === "string") {
    const key = schema.$ref.replace(/^#\/\$defs\//, "");
    const defs: Record<string, unknown> = schemaDocument.$defs;
    return Object.hasOwn(defs, key) && schemaAccepts(defs[key], value);
  }
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (!types.some((type) => {
      switch (type) {
        case "object": return isRecord(value);
        case "array": return Array.isArray(value);
        case "integer": return typeof value === "number" && Number.isInteger(value);
        case "number": return typeof value === "number" && Number.isFinite(value);
        case "null": return value === null;
        default: return typeof value === type;
      }
    })) return false;
  }
  if (Array.isArray(schema.enum) && !schema.enum.includes(value)) return false;
  if (typeof value === "number" &&
      ((typeof schema.minimum === "number" && value < schema.minimum) ||
       (typeof schema.maximum === "number" && value > schema.maximum))) return false;
  if (typeof value === "string") {
    if (typeof schema.pattern === "string" && !new RegExp(schema.pattern).test(value)) return false;
    if (schema.format === "uint64" && !isCanonicalUint64(value)) return false;
    if (schema.format === "int64" &&
        (!/^-?[0-9]+$/.test(value) || BigInt(value) < -9223372036854775808n || BigInt(value) > 9223372036854775807n)) return false;
  }
  if (isRecord(value)) {
    if (Array.isArray(schema.required) && schema.required.some((key) => typeof key !== "string" || !Object.hasOwn(value, key))) return false;
    if (isRecord(schema.properties)) {
      for (const key of Object.keys(value)) {
        if (Object.hasOwn(schema.properties, key)) {
          if (!schemaAccepts(schema.properties[key], value[key])) return false;
        } else if (schema.additionalProperties === false ||
            (isRecord(schema.additionalProperties) && !schemaAccepts(schema.additionalProperties, value[key]))) return false;
      }
    }
  }
  if (Array.isArray(value) && schema.items !== undefined && !value.every((item) => schemaAccepts(schema.items, item))) return false;
  if (Array.isArray(schema.anyOf) && !schema.anyOf.some((item) => schemaAccepts(item, value))) return false;
  if (Array.isArray(schema.oneOf) && schema.oneOf.filter((item) => schemaAccepts(item, value)).length !== 1) return false;
  if (Array.isArray(schema.allOf) && !schema.allOf.every((item) => schemaAccepts(item, value))) return false;
  if (schema.not !== undefined && schemaAccepts(schema.not, value)) return false;
  return true;
}

/** Validate installed domain payloads without coercion or default insertion. */
export function validateDomainSchema(message: string, value: unknown): void {
  const defs: Record<string, unknown> = schemaDocument.$defs;
  if (!Object.hasOwn(defs, message) || !schemaAccepts(defs[message], value)) {
    throw new S1WireError(RPC_BAD_REQUEST, `invalid ${message}`);
  }
}

/** S1 input: generated structure plus method semantics. */
export function validateInput(method: string, input: unknown): void {
  if (!Object.hasOwn(INPUT_MESSAGES, method)) throw new S1WireError(RPC_BAD_REQUEST, `${method} has no S1 input type`);
  validateDomainSchema(INPUT_MESSAGES[method], input);
  if (!isRecord(input)) throw new S1WireError(RPC_BAD_REQUEST, "input must be an object");
  for (const [key, value] of Object.entries(input)) {
    if (["spaceId", "integrationId", "providerEndpointId", "contractId", "method", "state"].includes(key) && value === "") {
      throw new S1WireError(RPC_BAD_REQUEST, `${key} must be non-empty`);
    }
    if (["registrationId", "requestId", "subscriptionId"].includes(key) && (typeof value !== "string" || !isUlid(value))) {
      throw new S1WireError(RPC_BAD_REQUEST, `${key} must be a ULID`);
    }
  }
  if ((method === METHOD_MODULE_INVOKE || method === METHOD_STATE_SUBSCRIBE) && !Object.hasOwn(input, "argument")) {
    throw new S1WireError(RPC_BAD_REQUEST, "input.argument is required");
  }
  if (method === METHOD_STATE_UPDATE) validateSample("sample", input.sample);
  if (method === METHOD_STATE_END) parseStateEndError(input.error);
}

/** Invoke results are validated separately against their installed domain contract. */
export function validateResult(method: string, result: unknown): void {
  if (!Object.hasOwn(RESULT_MESSAGES, method)) return;
  validateDomainSchema(RESULT_MESSAGES[method], result);
  if (!isRecord(result)) throw new S1WireError(RPC_BAD_REQUEST, "result must be an object");
  if (method === METHOD_MODULE_REGISTER) {
    for (const key of ["providerEndpointId", "spaceId", "integrationId", "contractId"]) {
      if (result[key] === "") throw new S1WireError(RPC_BAD_REQUEST, `${key} must be non-empty`);
    }
    if (typeof result.registrationId !== "string" || !isUlid(result.registrationId)) throw new S1WireError(RPC_BAD_REQUEST, "invalid registrationId");
  }
  if (method === METHOD_MODULE_LIST) validateModules("modules", result.modules);
  if (method === METHOD_STATE_SUBSCRIBE) {
    for (const key of ["subscriptionId", "registrationId"]) {
      if (typeof result[key] !== "string" || !isUlid(result[key])) throw new S1WireError(RPC_BAD_REQUEST, `invalid ${key}`);
    }
    validateSample("snapshot", result.snapshot);
  }
}

/** StateSample 规范规则：revision/sampledAtMs 规范 uint64、quality 词表。 */
export function validateSample(path: string, value: unknown): void {
  validateDomainSchema("conex.StateSample", value);
  if (!isRecord(value)) throw new S1WireError(RPC_BAD_REQUEST, `${path} must be an object`);
  if (!Object.hasOwn(value, "value")) throw new S1WireError(RPC_BAD_REQUEST, `${path}.value is required`);
  const quality = value.quality;
  if (quality !== "current" && quality !== "stale") {
    throw new S1WireError(RPC_BAD_REQUEST, `${path}.quality must be current or stale`);
  }
}

function validateModules(path: string, value: unknown): void {
  if (!Array.isArray(value)) throw new S1WireError(RPC_BAD_REQUEST, `${path} must be an array`);
  value.forEach((module, index) => {
    const itemPath = `${path}[${index}]`;
    if (!isRecord(module)) throw new S1WireError(RPC_BAD_REQUEST, `${itemPath} must be an object`);
    validateDomainSchema("conex.ModuleInfo", module);
    if (typeof module.providerEndpointId !== "string" || module.providerEndpointId === "") {
      throw new S1WireError(RPC_BAD_REQUEST, `${itemPath}.providerEndpointId must be a non-empty string`);
    }
    if (typeof module.registrationId !== "string" || !isUlid(module.registrationId)) {
      throw new S1WireError(RPC_BAD_REQUEST, `${itemPath}.registrationId must be a ULID`);
    }
    for (const key of ["spaceId", "integrationId", "contractId"]) {
      if (typeof module[key] !== "string" || module[key] === "") {
        throw new S1WireError(RPC_BAD_REQUEST, `${itemPath}.${key} must be a non-empty string`);
      }
    }
    for (const key of ["methods", "states"]) {
      const list = module[key];
      if (!Array.isArray(list) || list.some((item) => typeof item !== "string")) {
        throw new S1WireError(RPC_BAD_REQUEST, `${itemPath}.${key} must be an array of strings`);
      }
    }
  });
}

/** S1 limits: positive u32 bounds and reserved control capacity (§9). */
export function validateLimits(limits: Record<string, unknown>): void {
  validateDomainSchema("conex.Limits", limits);
  for (const key of ["maxFrameBytes", "maxInflight", "maxQueuedBytes", "timeoutMs", "maxSubscriptions", "maxModules"] as const) {
    const v = limits[key];
    if (!Object.hasOwn(limits, key) || typeof v !== "number" || !Number.isInteger(v) || v <= 0 || v > 4294967295) {
      throw new S1WireError(RPC_BAD_REQUEST, `${key} must be a positive u32`);
    }
  }
  if ((limits.maxInflight as number) < 2 || (limits.maxQueuedBytes as number) < 2 * (limits.maxFrameBytes as number)) {
    throw new S1WireError(RPC_BAD_REQUEST, "limits must reserve control capacity");
  }
}

/** Apply HTTP binding restrictions before dispatch; binding ownership is a Host concern. */
export function validateHttpFrame(frame: S1Frame): void {
  const method = frame.method ?? "";
  if (!isS1Method(method)) return;
  if (![METHOD_MODULE_LIST, METHOD_MODULE_INVOKE, METHOD_MODULE_CANCEL].includes(method)) {
    throw new S1WireError(ErrorCode.ERROR_CODE_UNSUPPORTED_CAPABILITY, "S1 method requires WSS");
  }
  if (frame.kind !== "request" || typeof frame.params?.context.bindingId !== "string" || frame.params.context.bindingId === "") {
    throw new S1WireError(RPC_BAD_REQUEST, "HTTP S1 requests require bindingId");
  }
}

const STATE_END_ERROR_KEYS: Record<string, true> = {
  code: true,
  message: true,
  diagnosticId: true,
  execution: true,
  retry: true,
  details: true,
};

/**
 * 解析 state/end 的 error（契约 §8.3）：生成 Error JSON（code 为枚举名，如
 * ERROR_CODE_SLOW_CONSUMER），不是 JSON-RPC Failure 的数值 code + data 形状；
 * execution/retry are required subscription termination metadata (§8.3).
 */
export function parseStateEndError(value: unknown): Error {
  if (!isRecord(value)) throw new S1WireError(RPC_BAD_REQUEST, "state/end error must be an object");
  for (const key of Object.keys(value)) {
    if (!Object.hasOwn(STATE_END_ERROR_KEYS, key)) throw new S1WireError(RPC_BAD_REQUEST, `unknown error field ${key}`);
  }
  const code = value.code;
  if (typeof code !== "string") throw new S1WireError(RPC_BAD_REQUEST, "error.code must be a string enum name");
  const parsed = errorCodeFromJSON(code);
  if (parsed === ErrorCode.ERROR_CODE_UNSPECIFIED || parsed === ErrorCode.UNRECOGNIZED) {
    throw new S1WireError(RPC_BAD_REQUEST, "error.code must be an ERROR_CODE_* enum name other than UNSPECIFIED");
  }
  for (const key of ["message", "diagnosticId", "execution", "retry"]) {
    if (!Object.hasOwn(value, key) || typeof value[key] !== "string") {
      throw new S1WireError(RPC_BAD_REQUEST, `error.${key} must be a string`);
    }
  }
  const retry = parsed === ErrorCode.ERROR_CODE_FORBIDDEN ? "never" : "safe";
  if (![ErrorCode.ERROR_CODE_UNAVAILABLE, ErrorCode.ERROR_CODE_STALE_REVISION, ErrorCode.ERROR_CODE_FORBIDDEN, ErrorCode.ERROR_CODE_SLOW_CONSUMER, ErrorCode.ERROR_CODE_SESSION_LOST].includes(parsed) ||
      value.execution !== "completed" || value.retry !== retry) {
    throw new S1WireError(RPC_BAD_REQUEST, "invalid subscription termination metadata");
  }
  return Error.fromJSON(value);
}

/** 生成 Error JSON：code 用枚举名，省略默认字段。 */
export function stateEndErrorToJson(error: Pick<Error, "code" | "message" | "diagnosticId" | "execution" | "retry" | "details">): Record<string, unknown> {
  if (error.code === undefined || error.code === ErrorCode.ERROR_CODE_UNSPECIFIED || error.code === ErrorCode.UNRECOGNIZED) {
    throw new S1WireError(RPC_BAD_REQUEST, "error code must not be unspecified");
  }
  const out: Record<string, unknown> = { code: errorCodeToJSON(error.code) };
  out.message = error.message ?? "";
  out.diagnosticId = error.diagnosticId ?? "";
  if (error.execution) out.execution = error.execution;
  if (error.retry) out.retry = error.retry;
  if (error.details !== undefined) out.details = error.details;
  parseStateEndError(out);
  return out;
}

/** StateSample -> 线上 JSON（revision/sampledAtMs 为十进制字符串）。 */
export function stateSampleToJson(sample: {
  revision: string;
  value?: unknown;
  sampledAtMs: string;
  quality: string;
}): Record<string, unknown> {
  return {
    revision: sample.revision,
    value: sample.value ?? null,
    sampledAtMs: sample.sampledAtMs,
    quality: sample.quality,
  };
}

/** 构造 state/update 的 params.input（契约 §11.2），并自检同一套规范规则。 */
export function stateUpdateInput(
  subscriptionId: string,
  seq: string,
  sample: { revision: string; value?: unknown; sampledAtMs: string; quality: string },
): Record<string, unknown> {
  const input = { subscriptionId, seq, sample: stateSampleToJson(sample) };
  validateInput(METHOD_STATE_UPDATE, input);
  return input;
}

/** 构造 state/end 的 params.input，error 为生成 Error JSON（§8.3），并自检。 */
export function stateEndInput(
  subscriptionId: string,
  error: Pick<Error, "code" | "message" | "diagnosticId" | "execution" | "retry" | "details">,
): Record<string, unknown> {
  const input = { subscriptionId, error: stateEndErrorToJson(error) };
  validateInput(METHOD_STATE_END, input);
  return input;
}
