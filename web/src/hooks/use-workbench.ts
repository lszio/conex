// 工作台状态机（P7）：hub 会话/目录轮询、节点直连通道（每契约一条）、
// Agent 领域调用与 EventFollower 流式投影。
//
// 拓扑事实（P5/P7 契约）：
// - 私有模块全部 direct-only：对话正文不过 hub；浏览器对每个模块持有独立
//   grant（scope 绑定单模块代际）→ 单次票据 → 单条 WSS。
// - hub 不下发节点地址：地址由用户按 nodeId 配置（已知地址直连）。
// - 节点重启 = 新 epoch，旧 grant 全部失效：通道自动重建（重新取 grant），
//   目录 ≤10 秒内带来新代际；grantProvider 永远读最新目录。
// - 直连入口没有服务端推送：事件流用有界轮询驱动（每 800ms 强迫一次
//   events/read；读结果携带真实 head，空闲时每轮只发生一次空读）。

import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";

import {
  AgentClient,
  EventFollower,
  NodeDirectClient,
  mintUlid,
  type AgentEventTransport,
  type AgentInvokeTransport,
  type Directory,
  type DirectoryInstance,
  type DirectoryModule,
  type DirectoryNode,
  type GrantScopeBody,
  type PrivateHubClient,
} from "@conex/sdk";
import { loadNodeAddress, saveNodeAddress } from "@/domain/session";

const CONTRACT_AGENT = "conex.agent";
const CONTRACT_METRICS = "conex.metrics";
const CONTRACT_RELATIONS = "conex.relations";
const CONTRACT_LLM = "conex.llm";

export interface ModuleTarget {
  instance: DirectoryInstance;
  module: DirectoryModule;
}

export function resolveModule(
  directory: Directory | null,
  nodeId: string,
  contractId: string,
): ModuleTarget | null {
  const node = directory?.nodes.find((entry) => entry.nodeId === nodeId);
  if (!node) return null;
  for (const instance of node.instances) {
    const module = instance.modules.find(
      (candidate) => candidate.contract?.contractId === contractId,
    );
    if (module) return { instance, module };
  }
  return null;
}

export type WorkbenchStatus = "idle" | "connecting" | "ready" | "reconnecting" | "closed";

export interface ChannelInfo {
  state: WorkbenchStatus;
  detail?: string | undefined;
}

function scopeFor(module: DirectoryModule): GrantScopeBody {
  return {
    spaceId: "private",
    providerEndpointId: `${module.name}@${module.generation}`,
    contractId: module.contract?.contractId ?? "",
    moduleGeneration: module.generation,
    methods: module.contract?.methods ?? [],
    states: [],
    workspaceIds: [],
    conversationIds: [],
    purpose: "direct",
  };
}

export function useWorkbench(hub: PrivateHubClient) {
  const [, bump] = useReducer((count: number) => count + 1, 0);
  const [directory, setDirectory] = useState<Directory | null>(null);
  const [directoryError, setDirectoryError] = useState<string | null>(null);
  const [nodeId, setNodeId] = useState<string | null>(null);
  const [address, setAddress] = useState<string>("http://127.0.0.1:7451");
  const [view, setView] = useState<"conversation" | "servers" | "graph" | "models">("conversation");
  const [channelInfo, setChannelInfo] = useState<Record<string, ChannelInfo>>({});

  const directoryRef = useRef<Directory | null>(null);
  const addressRef = useRef(address);
  addressRef.current = address;
  const channelsRef = useRef<Map<string, NodeDirectClient>>(new Map());
  const followerRef = useRef<EventFollower | null>(null);

  const node: DirectoryNode | null = useMemo(
    () => directory?.nodes.find((entry) => entry.nodeId === nodeId) ?? null,
    [directory, nodeId],
  );

  // --- 目录轮询：epoch/代际的唯一权威来源 -------------------------------
  useEffect(() => {
    let stopped = false;
    const refresh = async () => {
      try {
        const next = await hub.directory();
        if (stopped) return;
        directoryRef.current = next;
        setDirectoryError(null);
        setDirectory(next);
        setNodeId((current) => current ?? next.nodes[0]?.nodeId ?? null);
      } catch (cause) {
        if (stopped) return;
        setDirectoryError(cause instanceof Error ? cause.message : String(cause));
      }
    };
    void refresh();
    const timer = setInterval(refresh, 10_000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [hub]);

  // --- 节点地址记忆 ------------------------------------------------------
  useEffect(() => {
    if (nodeId) setAddress(loadNodeAddress(nodeId));
  }, [nodeId]);

  const updateAddress = useCallback(
    (next: string) => {
      setAddress(next);
      if (nodeId) saveNodeAddress(nodeId, next);
    },
    [nodeId],
  );

  // --- 直连通道：按契约一条，grant 永远用最新目录里的代际 -----------------
  const ensureChannel = useCallback(
    (contractId: string): NodeDirectClient => {
      const existing = channelsRef.current.get(contractId);
      if (existing) return existing;
      const channel = new NodeDirectClient(contractId, "0", {
        nodeOrigin: addressRef.current,
        grantProvider: async () => {
          const current = directoryRef.current;
          const target = resolveModule(current, nodeId ?? "", contractId);
          if (!target) {
            throw new Error(`node has no ${contractId} module (inventory pending)`);
          }
          return hub.grant(nodeId ?? "", scopeFor(target.module));
        },
        onState: (state, detail) => {
          setChannelInfo((previous) => ({
            ...previous,
            [contractId]: { state, detail },
          }));
        },
      });
      channelsRef.current.set(contractId, channel);
      return channel;
    },
    [hub, nodeId],
  );

  // 地址或节点变化：关闭旧通道，让下一次调用按新地址重建。
  useEffect(() => {
    const channels = channelsRef.current;
    return () => {
      for (const channel of channels.values()) channel.close();
      channels.clear();
      setChannelInfo({});
    };
  }, [nodeId, address]);

  const invokeModule = useCallback(
    async (contractId: string, method: string, argument: unknown): Promise<unknown> => {
      const channel = ensureChannel(contractId);
      const target = resolveModule(directoryRef.current, nodeId ?? "", contractId);
      if (!target) {
        throw new Error(`node has no ${contractId} module (inventory pending)`);
      }
      if (channel.state !== "ready") {
        await channel.connect();
      }
      return channel.invoke({
        instanceId: target.instance.id,
        module: target.module.name,
        generation: target.module.generation,
        method,
        argument,
      });
    },
    [ensureChannel, nodeId],
  );

  const invokeAgent = useCallback(
    (method: string, argument: unknown) => invokeModule(CONTRACT_AGENT, method, argument),
    [invokeModule],
  );

  const agent = useMemo(
    () => new AgentClient({ invoke: invokeAgent } as AgentInvokeTransport),
    [invokeAgent],
  );

  // --- 事件传输：无推送通道，轮询即传输（契约允许：head 提示只是加速） ---
  const eventTransport = useMemo<AgentEventTransport>(
    () => ({
      snapshot: (conversationId: string) => invokeAgent("events/snapshot", { conversationId }),
      read: (cursor: { streamId: string; epoch: string; after: string }, limit?: number) =>
        invokeAgent("events/read", { cursor, limit }),
      watchHead: () => {
        // 由下面的全局轮询代替；这里无事可做。
      },
    }),
    [invokeAgent],
  );

  // 有界事件轮询：强迫 head=after+1 触发一次 read；真实 head 由读结果修正。
  useEffect(() => {
    const timer = setInterval(() => {
      const follower = followerRef.current;
      if (!follower || follower.status === "revoked" || !follower.cursor) return;
      follower.head = (BigInt(follower.cursor.after ?? "0") + 1n).toString();
      follower
        .pump()
        .then(() => bump())
        .catch(() => bump());
    }, 800);
    return () => clearInterval(timer);
  }, []);

  // --- 对话管理 ----------------------------------------------------------
  const [endpoints, setEndpoints] = useState<Array<Record<string, unknown>>>([]);
  const [workspaces, setWorkspaces] = useState<Array<Record<string, unknown>>>([]);
  const [conversations, setConversations] = useState<Array<Record<string, unknown>>>([]);
  const [endpointId, setEndpointId] = useState<string | null>(null);
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const refreshEndpoints = useCallback(async () => {
    try {
      const result = (await agent.invoke("agent/endpoints/list", {})) as {
        items?: Array<Record<string, unknown>>;
      };
      // 只展示当前代际的对话型端点：指标/关系是 node-managed 内置端点，历史
      // 代际行是节点重启留下的过期投影——两者都不是会话入口。代际来自目录
      // inventory（节点自报），不是本地猜测。
      const live = resolveModule(directoryRef.current, nodeId ?? "", CONTRACT_AGENT);
      const agentAdapters = new Set(["omp", "codex"]);
      const items = (result.items ?? []).filter(
        (endpoint) =>
          agentAdapters.has(String(endpoint.adapter)) &&
          (!live || String(endpoint.generation) === live.module.generation),
      );
      setEndpoints(items);
      setLoadError(null);
    } catch (cause) {
      setLoadError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [agent, nodeId]);

  const refreshWorkspaces = useCallback(
    async (targetEndpointId: string) => {
      const result = (await agent.invoke("agent/workspaces/list", {
        endpointId: targetEndpointId,
      })) as { items?: Array<Record<string, unknown>> };
      setWorkspaces(result.items ?? []);
      setWorkspaceId((current) => current ?? (result.items?.[0]?.id as string | undefined) ?? null);
    },
    [agent],
  );

  const refreshConversations = useCallback(
    async (targetEndpointId: string, targetWorkspaceId: string | null) => {
      const result = (await agent.invoke("agent/conversations/list", {
        endpointId: targetEndpointId,
        ...(targetWorkspaceId ? { workspaceId: targetWorkspaceId } : {}),
        limit: 100,
      })) as { items?: Array<Record<string, unknown>> };
      setConversations(result.items ?? []);
    },
    [agent],
  );

  useEffect(() => {
    if (!node) return;
    void refreshEndpoints();
  }, [node, refreshEndpoints]);

  useEffect(() => {
    if (!endpointId) return;
    void refreshWorkspaces(endpointId).catch((cause) =>
      setLoadError(cause instanceof Error ? cause.message : String(cause)),
    );
  }, [endpointId, refreshWorkspaces]);

  useEffect(() => {
    if (!endpointId) return;
    void refreshConversations(endpointId, workspaceId).catch((cause) =>
      setLoadError(cause instanceof Error ? cause.message : String(cause)),
    );
  }, [endpointId, workspaceId, refreshConversations]);

  useEffect(() => {
    if (!conversationId) {
      followerRef.current = null;
      bump();
      return;
    }
    const follower = new EventFollower(eventTransport, conversationId, 32);
    followerRef.current = follower;
    bump();
    follower
      .start()
      .then(() => bump())
      .catch((cause) => {
        setLoadError(cause instanceof Error ? cause.message : String(cause));
        bump();
      });
  }, [conversationId, eventTransport]);

  const createConversation = useCallback(
    async (title?: string) => {
      if (!endpointId || !workspaceId) return;
      const result = (await agent.invoke("agent/conversations/create", {
        endpointId,
        workspaceId,
        idempotencyKey: mintUlid(),
        ...(title ? { title } : {}),
      })) as { conversation?: { id?: string } };
      await refreshConversations(endpointId, workspaceId);
      if (result.conversation?.id) setConversationId(result.conversation.id);
    },
    [agent, endpointId, workspaceId, refreshConversations],
  );

  const submit = useCallback(
    async (content: string) => {
      if (!conversationId) return;
      const result = (await agent.invoke("agent/executions/submit", {
        conversationId,
        idempotencyKey: mintUlid(),
        content,
      })) as { execution?: { id?: string } };
      bump();
      return result.execution?.id;
    },
    [agent, conversationId],
  );

  // 快照是有界的：完成态消息的正文窗口为空，完整文本经 agent/messages/read
  // 分页取回（契约 §4：单窗口 ≤65536 字节，UTF-8 边界分页）。
  const readMessage = useCallback(
    async (targetConversation: string, messageId: string): Promise<string> => {
      let offset = "0";
      let full = "";
      for (let page_ = 0; page_ < 16; page_ += 1) {
        const page = (await agent.invoke("agent/messages/read", {
          conversationId: targetConversation,
          messageId,
          offsetBytes: offset,
          maxBytes: 65536,
        })) as { text?: string; nextOffsetBytes?: string; totalBytes?: string };
        full += page.text ?? "";
        const next = page.nextOffsetBytes ?? offset;
        if (BigInt(next) <= BigInt(offset)) break;
        if (page.totalBytes !== undefined && BigInt(next) >= BigInt(page.totalBytes)) break;
        offset = next;
      }
      return full;
    },
    [agent],
  );

  const cancelExecution = useCallback(
    async (executionId: string, expectedGeneration: string) => {
      await agent.invoke("agent/executions/cancel", {
        executionId,
        expectedGeneration,
      });
      bump();
    },
    [agent],
  );

  const respondInteraction = useCallback(
    async (
      interactionId: string,
      expectedRevision: string,
      decision: Record<string, unknown>,
    ) => {
      await agent.invoke("agent/interactions/respond", {
        interactionId,
        expectedRevision,
        decision,
      });
      bump();
    },
    [agent],
  );

  // --- 服务器指标 --------------------------------------------------------
  const [metrics, setMetrics] = useState<Array<Record<string, unknown>>>([]);
  useEffect(() => {
    if (view !== "servers") return;
    let stopped = false;
    const poll = async () => {
      try {
        const result = (await invokeModule(CONTRACT_METRICS, "metrics/read", {})) as {
          items?: Array<Record<string, unknown>>;
        };
        if (!stopped) {
          setMetrics(result.items ?? []);
          setLoadError(null);
        }
      } catch (cause) {
        if (!stopped) {
          setLoadError(cause instanceof Error ? cause.message : String(cause));
        }
      }
    };
    void poll();
    const timer = setInterval(poll, 5_000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [view, invokeModule]);

  // --- 模型服务（P10）：节点直连读取 provider/模型/健康投影 ---------------
  const [llmProviders, setLlmProviders] = useState<LlmProviderInfo[]>([]);
  useEffect(() => {
    if (view !== "models") return;
    let stopped = false;
    const poll = async () => {
      try {
        const result = (await invokeModule(CONTRACT_LLM, "llm/providers/list", {})) as {
          items?: LlmProviderInfo[];
        };
        if (!stopped) {
          setLlmProviders(result.items ?? []);
          setLoadError(null);
        }
      } catch (cause) {
        if (!stopped) {
          setLoadError(cause instanceof Error ? cause.message : String(cause));
        }
      }
    };
    void poll();
    const timer = setInterval(poll, 10_000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [view, invokeModule]);

  // --- 关系与标注 --------------------------------------------------------
  const [tags, setTags] = useState<Array<Record<string, unknown>>>([]);
  const refreshTags = useCallback(
    async (entityKind: string, entityId: string) => {
      const result = (await invokeModule(CONTRACT_RELATIONS, "tags/list", {
        entityKind,
        entityId,
      })) as { items?: Array<Record<string, unknown>> };
      setTags(result.items ?? []);
    },
    [invokeModule],
  );

  const putTag = useCallback(
    async (entityKind: string, entityId: string, name: string) => {
      await invokeModule(CONTRACT_RELATIONS, "tags/put", { entityKind, entityId, name });
      await refreshTags(entityKind, entityId);
    },
    [invokeModule, refreshTags],
  );

  const deleteTag = useCallback(
    async (entityKind: string, entityId: string, name: string) => {
      await invokeModule(CONTRACT_RELATIONS, "tags/delete", { entityKind, entityId, name });
      await refreshTags(entityKind, entityId);
    },
    [invokeModule, refreshTags],
  );

  useEffect(() => {
    if (view !== "graph" || !conversationId) return;
    void refreshTags("conversation", conversationId).catch((cause) =>
      setLoadError(cause instanceof Error ? cause.message : String(cause)),
    );
  }, [view, conversationId, refreshTags]);

  const follower = followerRef.current;

  return {
    hub,
    directory,
    directoryError,
    node,
    nodeId,
    address,
    updateAddress,
    channelInfo,
    view,
    setView,
    follower,
    projection: follower?.projection ?? null,
    followerStatus: follower?.status ?? "idle",
    endpoints,
    workspaces,
    conversations,
    endpointId,
    setEndpointId,
    workspaceId,
    setWorkspaceId,
    conversationId,
    setConversationId,
    createConversation,
    submit,
    readMessage,
    cancelExecution,
    respondInteraction,
    refreshConversations: () => {
      if (endpointId) void refreshConversations(endpointId, workspaceId);
    },
    metrics,
    llmProviders,
    tags,
    refreshTags,
    putTag,
    deleteTag,
    loadError,
    agentGeneration: resolveModule(directory, nodeId ?? "", CONTRACT_AGENT)?.module.generation ?? null,
  };
}

export type Workbench = ReturnType<typeof useWorkbench>;

// LLM 提供者投影（schema/conex/llm.proto 的 JSON 形状；未知信息保持节点
// 原样：health=live/stale/unreachable，tools=unknown 不冒充支持）。
export interface LlmModelInfo {
  id: string;
  ownedBy: string;
  tools: string;
  contextWindow?: string | undefined;
}

export interface LlmProviderInfo {
  id: string;
  kind: string;
  baseUrl: string;
  health: string;
  checkedAtMs?: string | undefined;
  latencyMs?: number | undefined;
  detail?: string | undefined;
  models: LlmModelInfo[];
}
