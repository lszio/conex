/**
 * Agent 领域边界与 EventFollower reducer 测试：共享协议向量
 * （conformance/vectors/agent-domain.json）+ 游标/delta 边界。不测试 mock 透传。
 */

import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  AGENT_CONVERSATIONS_CREATE, AGENT_ENDPOINTS_LIST, AGENT_EXECUTIONS_SUBMIT,
  AGENT_METHODS, EVENTS_SNAPSHOT, agentExecutionClass, expandVectorValue,
  requiredPermission, validateAgentInput, validateAgentOutput,
} from "../src/agent-domain";
import { EventFollower, ResyncRequiredError } from "../src/events";
import type { AgentEventTransport } from "../src/events";
import { parseStrict } from "../src/s1-wire";
import type {
  AgentEvent, AgentEventCursor, AgentEventReadResult, AgentSnapshot,
} from "../src/generated/conex/events";

const vectorsUrl = new URL("../../../conformance/vectors/agent-domain.json", import.meta.url);
const vectors = JSON.parse(readFileSync(vectorsUrl, "utf8")) as {
  accept: Array<{ name: string; method: string; direction: string; value: unknown }>;
  reject: Array<{ name: string; method: string; direction: string; value: unknown }>;
  rawReject: Array<{ name: string; rawJson: string; reason: string }>;
};

function validateCase(direction: string, method: string, value: unknown): void {
  const expanded = expandVectorValue(value);
  if (direction === "input") validateAgentInput(method, expanded);
  else validateAgentOutput(method, expanded);
}

test("agent method catalog mirrors the contract operation table", () => {
  expect(vectors.accept.length + vectors.reject.length).toBeGreaterThan(0);
  expect(AGENT_METHODS.length).toBe(18);
  expect(new Set(AGENT_METHODS).size).toBe(18);
  expect(agentExecutionClass(AGENT_ENDPOINTS_LIST)).toBe("read_only");
  expect(agentExecutionClass(EVENTS_SNAPSHOT)).toBe("read_only");
  expect(agentExecutionClass(AGENT_EXECUTIONS_SUBMIT)).toBe("non_replayable");
  expect(agentExecutionClass(AGENT_CONVERSATIONS_CREATE)).toBe("non_replayable");
  expect(agentExecutionClass("agent/executions/nope")).toBeUndefined();
  expect(requiredPermission(AGENT_EXECUTIONS_SUBMIT)).toBe("agent.execute");
  expect(requiredPermission(AGENT_ENDPOINTS_LIST)).toBe("agent.read");
});

test("shared accept vectors pass strict validation", () => {
  for (const c of vectors.accept) {
    expect(() => validateCase(c.direction, c.method, c.value), c.name).not.toThrow();
  }
});

test("shared reject vectors throw", () => {
  for (const c of vectors.reject) {
    expect(() => validateCase(c.direction, c.method, c.value), c.name).toThrow();
  }
});

test("raw duplicate keys are rejected before object conversion", () => {
  for (const c of vectors.rawReject) {
    expect(() => JSON.parse(c.rawJson), c.name).not.toThrow(); // JSON.parse alone is last-wins
    expect(() => parseStrict(c.rawJson), c.name).toThrow();
  }
});

const CID = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const EPOCH = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const EX = "01ARZ3NDEKTSV4RRFFQ69G5FAX";
const MSG = "01ARZ3NDEKTSV4RRFFQ69G5FAW";

let position = 0;

function resetPositions(): void {
  position = 0;
}

function event(type: string, data: Partial<AgentEvent>): AgentEvent {
  position += 1;
  return {
    streamId: CID, epoch: EPOCH, position: String(position),
    recordedAtMs: "1730000000000", type, entityId: EX, entityRevision: "1", ...data,
  };
}

function page(events: AgentEvent[], hasMore: boolean, head: string): AgentEventReadResult {
  return {
    events,
    nextCursor: { streamId: CID, epoch: EPOCH, after: events[events.length - 1].position },
    head, firstAvailable: "1", hasMore,
  };
}

function snapshotFixture(): AgentSnapshot {
  return {
    conversation: {
      id: CID, endpointId: CID, workspaceId: CID, createdAtMs: "1730000000000",
      revision: "1", quality: "current", activeExecutionId: EX,
    },
    executions: [{
      id: EX, conversationId: CID, endpointId: CID, moduleInstanceId: CID,
      generation: "1", state: "unknown", revision: "1",
      createdAtMs: "1730000000000", updatedAtMs: "1730000000000",
    }],
    interactions: [], messages: [], relationships: [], gaps: [],
    cursor: { streamId: CID, epoch: EPOCH, after: "0" },
    firstAvailable: "1", quality: "current",
  };
}

function transportFor(snapshot: AgentSnapshot, pages: AgentEventReadResult[]): AgentEventTransport {
  let index = 0;
  const head = String(pages.length > 0 ? pages[pages.length - 1].head : snapshot.cursor?.after ?? "0");
  return {
    async snapshot() {
      return { snapshot };
    },
    async read(cursor: AgentEventCursor) {
      const page = pages[index];
      index += 1;
      return page ?? { events: [], nextCursor: { ...cursor }, head, firstAvailable: "1", hasMore: false };
    },
    watchHead(_conversationId: string, onHead: (head: string) => void) {
      onHead(head);
    },
  };
}

function messageEvents(): AgentEvent[] {
  return [
    event("execution.updated", {
      executionUpdated: {
        id: EX, state: "unknown", revision: "2", generation: "1",
        conversationId: CID, endpointId: CID, moduleInstanceId: CID,
        createdAtMs: "1730000000000", updatedAtMs: "1730000000001",
      },
    }),
    event("message.created", {
      messageCreated: {
        id: MSG, conversationId: CID, executionId: EX, role: "assistant",
        state: "streaming", revision: "1", parts: [],
      },
    }),
    event("message.delta", {
      messageDelta: { messageId: MSG, partIndex: 0, offsetBytes: "0", text: "Hello " },
    }),
    event("message.delta", {
      messageDelta: { messageId: MSG, partIndex: 0, offsetBytes: "6", text: "world" },
    }),
    event("message.completed", {
      messageCompleted: { messageId: MSG, revision: "2", state: "complete" },
    }),
  ];
}

test("reducer: snapshot then pages advance cursor and append text once", async () => {
  resetPositions();
  const events = messageEvents();
  const pages = [
    page(events.slice(0, 2), true, "5"),
    page(events.slice(2, 4), true, "5"),
    page(events.slice(4, 5), false, "5"),
  ];
  const follower = new EventFollower(transportFor(snapshotFixture(), pages), CID);
  await follower.start();
  const message = follower.projection?.messages?.[0];
  expect(message?.parts?.[0]?.text?.text).toBe("Hello world");
  expect(message?.parts?.[0]?.text?.totalBytes).toBe("11");
  expect(message?.state).toBe("complete");
  expect(follower.cursor?.after).toBe("5");
  expect(follower.cursor?.epoch).toBe(EPOCH);
  expect(follower.head).toBe("5");
  expect(follower.status).toBe("current");
  // unknown 执行仍占会话槽
  expect(follower.projection?.conversation?.activeExecutionId).toBe(EX);
  expect(follower.projection?.executions?.[0]?.state).toBe("unknown");
});

test("reducer: duplicate page delivered twice appends text once", async () => {
  resetPositions();
  const events = messageEvents();
  const duplicate = page(events.slice(0, 2), true, "5");
  const pages = [
    duplicate,
    page(events.slice(0, 2), true, "5"), // 同一页重复交付
    page(events.slice(2, 4), true, "5"),
    page(events.slice(4, 5), false, "5"),
  ];
  const follower = new EventFollower(transportFor(snapshotFixture(), pages), CID);
  await follower.start();
  expect(follower.projection?.messages?.[0]?.parts?.[0]?.text?.text).toBe("Hello world");
  expect(follower.cursor?.after).toBe("5");
});

test("reducer: duplicate position with different content requires resync", async () => {
  resetPositions();
  const events = messageEvents();
  const conflicted = structuredClone(events.slice(0, 2));
  const execution = conflicted[0].executionUpdated;
  if (execution) execution.revision = "9"; // 同键异内容
  const pages = [
    page(events.slice(0, 2), true, "5"),
    page(conflicted, true, "5"),
  ];
  const follower = new EventFollower(transportFor(snapshotFixture(), pages), CID);
  await expect(follower.start()).rejects.toBeInstanceOf(ResyncRequiredError);
});

test("reducer: delta with wrong offsetBytes requires resync", async () => {
  resetPositions();
  const events = messageEvents();
  const bad = events[3].messageDelta;
  if (bad) bad.offsetBytes = "5"; // part 已知 total 是 "6"
  const pages = [
    page(events.slice(0, 3), true, "5"),
    page(events.slice(3, 5), false, "5"),
  ];
  const follower = new EventFollower(transportFor(snapshotFixture(), pages), CID);
  await expect(follower.start()).rejects.toBeInstanceOf(ResyncRequiredError);
  expect(follower.projection?.messages?.[0]?.parts?.[0]?.text?.text).toBe("Hello ");
  expect(follower.status).not.toBe("current");
});

test("reducer: epoch change requires resync", async () => {
  resetPositions();
  const events = messageEvents();
  const otherEpoch = structuredClone(events.slice(2, 5));
  for (const e of otherEpoch) e.epoch = "01ARZ3NDEKTSV4RRFFQ69G5FBV";
  const pages = [
    page(events.slice(0, 2), true, "5"),
    page(otherEpoch, false, "5"),
  ];
  const follower = new EventFollower(transportFor(snapshotFixture(), pages), CID);
  await expect(follower.start()).rejects.toBeInstanceOf(ResyncRequiredError);
});

test("reducer: position gap requires resync", async () => {
  resetPositions();
  const events = messageEvents();
  events[2].position = "4"; // 跳过 position 3
  const pages = [
    page(events.slice(0, 2), true, "5"),
    page(events.slice(2, 5), false, "5"),
  ];
  const follower = new EventFollower(transportFor(snapshotFixture(), pages), CID);
  await expect(follower.start()).rejects.toBeInstanceOf(ResyncRequiredError);
});

test("reducer: never current without a complete snapshot", async () => {
  resetPositions();
  const follower = new EventFollower(transportFor(snapshotFixture(), []), CID);
  await follower.pump(); // 无 cursor、无快照：不动投影，也不得置 current
  expect(follower.projection).toBeNull();
  expect(follower.status).toBe("idle");
});
