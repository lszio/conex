import { ConexError } from "./errors";
import type { SourceListRequest, SourceListResponse, SourceReadRequest, SourceReadResponse, SourceSearchRequest, SourceSearchResponse } from "./generated/conex/source";
export type ClientFetch = (url: string | URL, init?: RequestInit) => Promise<Response>;
export interface ClientOptions {
    url: string;
    tokenProvider: () => Promise<string>;
    fetch?: ClientFetch;
    profileId?: string;
    requires?: string[];
    timeoutMs?: number;
    maxResponseBytes?: number;
}
export interface SearchManyTarget {
    endpointId: string;
    input: SourceSearchRequest;
}
export interface SearchManyResult {
    endpointId: string;
    result?: SourceSearchResponse;
    error?: ConexError;
}
export declare function mintUlid(now?: number): string;
export declare class ConexClient {
    private readonly url;
    private readonly tokenProvider;
    private readonly fetchImpl;
    private readonly profileId;
    private readonly requires;
    private readonly timeoutMs;
    private readonly maxResponseBytes;
    private binding?;
    private closed;
    private readonly abort;
    private constructor();
    static connect(options: ClientOptions): Promise<ConexClient>;
    list(endpointId: string, input: SourceListRequest): Promise<SourceListResponse>;
    read(endpointId: string, input: SourceReadRequest): Promise<SourceReadResponse>;
    search(endpointId: string, input: SourceSearchRequest): Promise<SourceSearchResponse>;
    searchMany(targets: SearchManyTarget[]): Promise<SearchManyResult[]>;
    close(): void;
    private call;
    private ensureBinding;
    private envelope;
    private post;
    private parse;
    private readCapped;
}
