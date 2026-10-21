#!/usr/bin/env bun
// Test-only single-hop S1 broker for plugin-runtime tests (S2 contract
// docs/contracts/s2-plugins.md §8). Same fixture convention as s1-node.ts:
// all SDK traffic runs over real WebSocket connections, but this broker is
// in-process so tests can observe registers, unpublishes, disconnects and
// forward frames between instances without spawning a host binary.
//
// Forwards module/invoke, module/cancel, state/subscribe and
// state/unsubscribe to the link that owns the targeted registration, routes
// responses back to the caller, and routes state/update|end notifications to
// the consumer link. Unknown registrationId answers the S1 stale error so
// callers observe -32013 without a real host generation.
import type { ServerWebSocket } from "bun";
import { mintUlid } from "../../src/client";
import type { ModuleReference } from "../../src/capability";

export const S2_LIMITS = {
  maxFrameBytes: 262144, maxInflight: 16, maxQueuedBytes: 1048576,
  timeoutMs: 5000, maxSubscriptions: 16, maxModules: 16,
};
const METHODS = [
  "module/list", "module/invoke", "module/cancel", "state/subscribe", "state/unsubscribe",
  "module/register", "module/unregister",
];

interface Link { linkId: string; bindingId: string; registrations: Set<string>; ws: ServerWebSocket<Link> }
interface HostEvent { kind: "register" | "unregister" | "subscribe" | "unsubscribe" | "disconnect"; registrationId?: string; subscriptionId?: string; linkId?: string; providerEndpointId?: string }

/** Observable surface of the in-process S1 broker fixture. */
export interface S2Host {
  port: number;
  limits: typeof S2_LIMITS;
  events: HostEvent[];
  references(): ModuleReference[];
  closeRegistration(registrationId: string): void;
  stop(): void;
}

export function s2Host(): S2Host {
  const links = new Set<Link>();
  const registrations = new Map<string, { reference: ModuleReference; link: Link }>();
  const subscriptions = new Map<string, Link>();
  const callers = new Map<string, Link>();
  const events: HostEvent[] = [];

  const server = Bun.serve<Link>({
    hostname: "127.0.0.1", port: 0,
    fetch(request, bserver) {
      const link: Link = { linkId: mintUlid(), bindingId: mintUlid(), registrations: new Set(), ws: undefined as never };
      if (bserver.upgrade(request, { data: link })) return;
      return new Response("upgrade required", { status: 400 });
    },
    websocket: {
      open(ws) { ws.data.ws = ws; links.add(ws.data); },
      message(ws, data) {
        const link = ws.data;
        const frame = JSON.parse(String(data)) as {
          id?: string; method?: string;
          error?: { code: number; message: string };
          params?: { context?: Record<string, unknown>; input?: Record<string, unknown> };
        };
        const reply = (result: unknown) => ws.send(JSON.stringify({ jsonrpc: "2.0", id: frame.id, result }));
        const replyError = (code: number, message: string) => ws.send(JSON.stringify({ jsonrpc: "2.0", id: frame.id, error: { code, message } }));
        if (frame.method === "conex/hello") {
          reply({ bindingId: link.bindingId, expiresInMs: 60000, profileId: "conex-jsonrpc2-wss", plane: "PLANE_BROKER", provides: METHODS, limits: S2_LIMITS });
          return;
        }
        if (frame.method === "conex/ready") {
          reply({ negotiationId: link.bindingId, profileId: "conex-jsonrpc2-wss", plane: "PLANE_BROKER", provides: METHODS, limits: S2_LIMITS, linkIdentity: { linkId: link.linkId, peerId: "host", tenantId: "test" } });
          return;
        }
        const input = frame.params?.input ?? {};
        const contextRegistration = frame.params?.context?.registrationId;
        const registrationId = typeof contextRegistration === "string" ? contextRegistration : undefined;
        if (frame.method === "module/register") {
          const freshId = mintUlid();
          const reference: ModuleReference = {
            spaceId: String(input.spaceId), providerEndpointId: String(input.providerEndpointId),
            integrationId: String(input.integrationId), contractId: String(input.contractId),
            registrationId: freshId, methods: [], states: [],
          };
          registrations.set(freshId, { reference, link });
          link.registrations.add(freshId);
          events.push({ kind: "register", registrationId: freshId, providerEndpointId: reference.providerEndpointId, linkId: link.linkId });
          // ModuleRegisterResult schema rejects extra fields: reply exactly.
          reply({ registrationId: freshId, providerEndpointId: reference.providerEndpointId, spaceId: reference.spaceId, integrationId: reference.integrationId, contractId: reference.contractId });
          return;
        }
        if (frame.method === "module/unregister") {
          const registrationId = String(input.registrationId);
          const providerEndpointId = registrations.get(registrationId)?.reference.providerEndpointId;
          registrations.delete(registrationId);
          link.registrations.delete(registrationId);
          events.push({ kind: "unregister", registrationId, providerEndpointId, linkId: link.linkId });
          reply({ removed: true });
          return;
        }
        if (frame.method === "module/list") {
          const spaceId = input.spaceId ? String(input.spaceId) : undefined;
          reply({ modules: [...registrations.values()].map(entry => entry.reference).filter(reference => !spaceId || reference.spaceId === spaceId) });
          return;
        }
        if (frame.method === "state/update" || frame.method === "state/end") {
          const subscriptionId = String(input.subscriptionId);
          const consumer = subscriptions.get(subscriptionId);
          if (consumer) {
            if (frame.method === "state/end") subscriptions.delete(subscriptionId);
            consumer.ws.send(String(data));
          }
          return;
        }
        const owned = registrationId ? registrations.get(registrationId) : undefined;
        if (frame.method === "module/invoke" || frame.method === "module/cancel" || frame.method === "state/subscribe" || frame.method === "state/unsubscribe") {
          if (!owned) { replyError(-32013, "stale registration"); return; }
          if (frame.method === "state/subscribe") {
            subscriptions.set(String(input.subscriptionId), link);
            events.push({ kind: "subscribe", registrationId, subscriptionId: String(input.subscriptionId) });
          }
          if (frame.method === "state/unsubscribe") {
            subscriptions.delete(String(input.subscriptionId));
            events.push({ kind: "unsubscribe", registrationId, subscriptionId: String(input.subscriptionId) });
          }
          // A real host stamps the verified caller identity before forwarding.
          frame.params!.context = { ...frame.params!.context, principalId: link.linkId, tenantId: "test" };
          callers.set(frame.id!, link);
          owned.link.ws.send(JSON.stringify(frame));
          return;
        }
        // A response (or late error) to a forwarded request: give it back to the caller.
        if (frame.id !== undefined && callers.has(frame.id)) {
          const caller = callers.get(frame.id)!;
          callers.delete(frame.id);
          caller.ws.send(String(data));
        }
      },
      close(ws) {
        const link = ws.data;
        links.delete(link);
        for (const registrationId of link.registrations) registrations.delete(registrationId);
        for (const [subscriptionId, consumer] of subscriptions) if (consumer === link) subscriptions.delete(subscriptionId);
        events.push({ kind: "disconnect", linkId: link.linkId });
      },
    },
  });

  return {
    // bun-types types Server#port as optional, but a TCP serve() always binds one.
    port: server.port!, limits: S2_LIMITS, events,
    references(): ModuleReference[] { return [...registrations.values()].map(entry => entry.reference); },
    /** Close the transport owning a registration, as a real disconnect would. */
    closeRegistration(registrationId: string): void {
      const link = registrations.get(registrationId)?.link;
      if (link) link.ws.close();
    },
    stop(): void { server.stop(true); },
  };
}
