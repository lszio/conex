//! @conex/sdk public surface.
export * from "./client";
export * from "./content";
export * from "./errors";
export { ErrorCode, Limits, Plane } from "./generated/conex/common";
export { HelloRequest, HelloResponse } from "./generated/conex/control";
export {
  ResourceSummary,
  SearchHit,
  SourceListRequest,
  SourceListResponse,
  SourceReadRequest,
  SourceReadResponse,
  SourceSearchRequest,
  SourceSearchResponse,
} from "./generated/conex/source";
export {
  AttachmentKind,
  AuthorizedScope,
  ConnectionState,
  EndpointListRequest,
  EndpointListResult,
  EndpointSummary,
} from "./generated/conex/endpoint";
export {
  ClientHelloRequest,
  ClientHelloResult,
  ClientListRequest,
  ClientListResponse,
  ClientProfile,
  ClientProfileRequest,
  ClientProfileResponse,
  ClientStatusRequest,
  ClientStatusResponse,
  ClientSummary,
  ConnectionListRequest,
  ConnectionListResponse,
  GroupStatus,
  UiLinkSummary,
  AgentLinkSummary,
} from "./generated/conex/dashboard";
export * from "./ws-client";
export * from "./plugin";
export type {
  CapabilityContract, CapabilityOptions, ExecutionClass, MethodContract, StateContract,
  ModuleReference, ModuleProvider, VerifiedContext, Invocation, InvokeOptions,
} from "./capability";
export { AtomicState } from "./state";
export type { Sample, StateObserver, StateOpening, StateProvider, StateSubscription } from "./state";
export type {
  ModuleRegisterInput, ModuleRegisterResult, ModuleInfo, ModuleListInput, ModuleListResult,
  ModuleInvokeInput, ModuleCancelInput, ModuleCancelResult, StateSample,
  StateSubscribeInput, StateSubscribeResult, StateUpdateInput, StateEndInput,
} from "./generated/conex/capability";
