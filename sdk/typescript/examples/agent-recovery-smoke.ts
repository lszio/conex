/**
 * 真实 reducer smoke：读取 Rust recovery_smoke 输出的 NDJSON（argv[2]），
 * 每行 {"kind":"meta"|"snapshot"|"page",...}；snapshot 行携带 {"snapshot":...}，
 * page 行携带 {"page":<AgentEventReadResult>}。对同一输入运行与 SDK 相同的
 * EventFollower reducer，断言：assistant 文本恰好追加一次（重复页不重复追加）、
 * unknown 执行仍占会话槽（activeExecutionId 置位；无未终态执行时必须已清槽）、
 * epoch/position 严格连续、重复读取相同页是 no-op。失败非零退出并给出明确
 * 消息；成功打印一行 {"ok":true,...}。不启动模型，不伪装 Adapter。
 */

import { readFileSync } from "node:fs";
import { EventFollower } from "../src/events";
import type { AgentEventTransport } from "../src/events";
import type {
  AgentEvent, AgentEventCursor, AgentEventReadResult, AgentSnapshot,
} from "../src/generated/conex/events";
import { isRecord, isUlid, isCanonicalUint64 } from "../src/s1-wire";

function fail(message: string): never {
  console.error(`agent-recovery-smoke: ${message}`);
  process.exit(1);
}

const input = process.argv[2];
if (!input) fail("usage: bun examples/agent-recovery-smoke.ts <recovery.ndjson>");

let snapshot: AgentSnapshot | undefined;
const pages: AgentEventReadResult[] = [];
let metaHead: string | undefined;

for (const [index, line] of readFileSync(input, "utf8").split("\n").entries()) {
  const text = line.trim();
  if (!text) continue;
  let record: unknown;
  try {
    record = JSON.parse(text);
  } catch (error) {
    fail(`line ${index + 1} is not valid JSON: ${error instanceof Error ? error.message : error}`);
  }
  if (!isRecord(record)) fail(`line ${index + 1} is not a JSON object`);
  if (record.kind === "meta") {
    if (typeof record.head === "string") metaHead = record.head;
  } else if (record.kind === "snapshot") {
    if (!isRecord(record.snapshot)) fail(`line ${index + 1}: snapshot line carries no snapshot`);
    snapshot = record.snapshot as AgentSnapshot;
  } else if (record.kind === "page") {
    if (!isRecord(record.page)) fail(`line ${index + 1}: page line carries no page`);
    pages.push(record.page as AgentEventReadResult);
  } else {
    fail(`line ${index + 1}: unknown kind ${String(record.kind)}`);
  }
}

if (!snapshot) fail("no snapshot line in input");
const snap = snapshot;
const snapCursor = snap.cursor;
if (!isRecord(snapCursor)) fail("snapshot carries no cursor");
const { streamId, epoch, after } = snapCursor as Required<AgentEventCursor>;
if (typeof streamId !== "string" || !isUlid(epoch) || !isCanonicalUint64(after)) {
  fail("snapshot cursor is malformed");
}
const lastHead = metaHead
  ?? String(pages.length > 0 ? pages[pages.length - 1].head : after);
if (!isCanonicalUint64(lastHead)) fail(`head ${lastHead} is not a canonical uint64 string`);

// 传输：按序供应 NDJSON 页；队列耗尽后按原样重放最后一页（hasMore 置位以驱动
// 重读，事件内容逐字节相同），再给终止空页——重放由 reducer 去重，正文不得重复。
let next = 0;
let replayed = false;
const transport: AgentEventTransport = {
  async snapshot() {
    return { snapshot: snap };
  },
  async read(cursor: AgentEventCursor) {
    if (next < pages.length) return pages[next++];
    if (!replayed && pages.length > 0) {
      replayed = true;
      return { ...pages[pages.length - 1], hasMore: true };
    }
    return { events: [], nextCursor: { ...cursor }, head: lastHead, firstAvailable: "1", hasMore: false };
  },
  watchHead(_conversationId: string, onHead: (head: string) => void) {
    onHead(lastHead);
  },
};

try {
  const follower = new EventFollower(transport, streamId);
  // reducer 会原地更新投影；期望值必须从运行前的快照副本播种。
  const pristine = structuredClone(snap);
  await follower.start();

  if (follower.status !== "current") fail(`follower ended as ${follower.status}, not current`);
  if (!follower.projection || !follower.cursor) fail("projection or cursor missing after run");
  const cursor = follower.cursor;
  if (cursor.streamId !== streamId) fail(`cursor stream ${cursor.streamId} != ${streamId}`);
  if (cursor.epoch !== epoch) fail(`epoch changed from ${epoch} to ${cursor.epoch}`);
  // epoch/position 严格连续：应用到的最后位置必须等于 head（reducer 已拒绝跳号）
  if (cursor.after !== lastHead) {
    fail(`cursor after ${cursor.after} != head ${lastHead}`);
  }

  // 期望正文：快照窗口 + 每个去重键只计一次的 delta，按文件顺序链接。
  const encoder = new TextEncoder();
  const expected = new Map<string, { text: string; bytes: bigint }>();
  for (const message of pristine.messages ?? []) {
    (message.parts ?? []).forEach((part, partIndex) => {
      if (part.text) {
        expected.set(`${message.id}:${partIndex}`, {
          text: part.text.text ?? "",
          bytes: BigInt(part.text.totalBytes ?? "0"),
        });
      }
    });
  }
  const seenDelta = new Set<string>();
  for (const page of pages) {
    for (const event of (page.events ?? []) as AgentEvent[]) {
      if (event.type !== "message.delta") continue;
      const delta = event.messageDelta;
      if (!delta) continue;
      // proto3 JSON omits default values: absent partIndex/offsetBytes are 0.
      const partIndex = delta.partIndex ?? 0;
      const offsetBytes = delta.offsetBytes ?? "0";
      const key = `${delta.messageId}:${partIndex}:${offsetBytes}`;
      if (seenDelta.has(key)) continue;
      seenDelta.add(key);
      const slot = expected.get(`${delta.messageId}:${partIndex}`) ?? { text: "", bytes: 0n };
      slot.text += delta.text ?? "";
      slot.bytes += BigInt(encoder.encode(delta.text ?? "").length);
      expected.set(`${delta.messageId}:${partIndex}`, slot);
    }
  }
  if (seenDelta.size === 0) fail("fixture contains no message.delta events; smoke proves nothing");

  // 文本恰好一次：每个 part 的正文与 totalBytes 必须精确等于期望，重复页不得加长。
  for (const [key, want] of expected) {
    const [messageId, partIndex] = key.split(":");
    const message = follower.projection.messages?.find((item) => item.id === messageId);
    const part = message?.parts?.[Number(partIndex)]?.text;
    if (!part) fail(`projection is missing message ${messageId} part ${partIndex}`);
    if (part.text !== want.text) {
      fail(`message ${messageId} part ${partIndex} text mismatch: got ${JSON.stringify(part.text)}, want ${JSON.stringify(want.text)}`);
    }
    if (part.totalBytes !== want.bytes.toString()) {
      fail(`message ${messageId} part ${partIndex} totalBytes ${part.totalBytes} != expected ${want.bytes}`);
    }
  }

  // unknown/未终态执行占槽；全部终态时槽必须已清。
  const terminal = new Set(["succeeded", "failed", "cancelled"]);
  const active = (follower.projection.executions ?? []).find(
    (execution) => !terminal.has(execution.state ?? ""),
  );
  const slot = follower.projection.conversation?.activeExecutionId;
  if (active && slot !== active.id) {
    fail(`execution ${active.id} (${active.state}) does not occupy the slot (activeExecutionId=${String(slot)})`);
  }
  if (!active && slot) {
    fail(`no active execution remains but activeExecutionId=${String(slot)} was not cleared`);
  }

  // 重复读取相同页是 no-op：投影逐字节不变。
  const before = JSON.stringify(follower.projection);
  if (pages.length > 0) follower.applyPage(pages[pages.length - 1]);
  if (JSON.stringify(follower.projection) !== before) {
    fail("re-reading the same page changed the projection");
  }

  console.log(JSON.stringify({
    ok: true,
    conversationId: streamId,
    epoch,
    cursorAfter: cursor.after,
    head: lastHead,
    events: seenDelta.size,
    messages: follower.projection.messages?.length ?? 0,
    replayChecked: pages.length > 0,
  }));
} catch (error) {
  fail(`${error instanceof Error ? error.message : String(error)}`);
}
