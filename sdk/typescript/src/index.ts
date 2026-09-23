//! @conex/sdk public surface.
export * from "./client";
export * from "./content";
export * from "./errors";
export { ErrorCode, Limits, Plane } from "./generated/conex/v1/common";
export { HelloRequest, HelloResponse } from "./generated/conex/v1/control";
export {
  ResourceSummary,
  SearchHit,
  SourceListRequest,
  SourceListResponse,
  SourceReadRequest,
  SourceReadResponse,
  SourceSearchRequest,
  SourceSearchResponse,
} from "./generated/conex/v1/source";
export {
  AttachmentKind,
  AuthorizedScope,
  ConnectionState,
  EndpointListRequest,
  EndpointListResult,
  EndpointSummary,
} from "./generated/conex/v1/endpoint";
export {
  ConnectionListRequest,
  ConnectionListResponse,
  UiLinkSummary,
  AgentLinkSummary,
} from "./generated/conex/v1/dashboard";
export * from "./ws-client";
