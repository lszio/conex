import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { BlobRef } from "./chunking";
export declare const protobufPackage = "conex";
/**
 * P0 只读 source 方法。ResourceId 是 UTF-8、斜杠分段、无空段/./../NUL/反斜杠的
 * 相对资源名；根目录用空字符串，仅允许出现在 list/search 的 root。
 * M0 元数据规则（docs/contracts/connected-landing.md §8）：
 * - title 是文件名（basename），不是路径；resource_id 才是定位。
 * - mime 未知时必须填 application/octet-stream；MIME 不等于允许执行内容。
 * - size_bytes / 长度一律十进制字符串，避免 JS Number 丢精度。
 * - revision 是不透明定位令牌，只做等值比较；provider 无法提供时缺省。
 * - kind 描述目录项类型；本轮 list 只产出常规文件。
 */
export declare enum EntryKind {
    ENTRY_KIND_UNSPECIFIED = 0,
    ENTRY_KIND_FILE = 1,
    ENTRY_KIND_DIRECTORY = 2,
    UNRECOGNIZED = -1
}
export declare function entryKindFromJSON(object: any): EntryKind;
export declare function entryKindToJSON(object: EntryKind): string;
export interface ResourceSummary {
    resourceId?: string | undefined;
    title?: string | undefined;
    mime?: string | undefined;
    sizeBytes?: string | undefined;
    revision?: string | undefined;
    kind?: EntryKind | undefined;
}
export interface SourceListRequest {
    root?: string | undefined;
    limit?: number | undefined;
    cursor?: string | undefined;
}
export interface SourceListResponse {
    items?: ResourceSummary[] | undefined;
    nextCursor?: string | undefined;
}
export interface SourceReadRequest {
    resourceId?: string | undefined;
}
/**
 * M0：读取结果二选一。
 * - 小型受支持 UTF-8 文本：内联 text（可附已计算 cid）。
 * - 附件与大文本：content 引用（BlobRef），不读入整文件、不伪造 cid。
 * text 与 content 互斥；cid 仅在字节已按 CID 格式计算时出现，可变远端文件缺省。
 */
export interface SourceReadResponse {
    resource?: ResourceSummary | undefined;
    text?: string | undefined;
    cid?: string | undefined;
    content?: BlobRef | undefined;
}
export interface SearchHit {
    resource?: ResourceSummary | undefined;
    excerpt?: string | undefined;
}
export interface SourceSearchRequest {
    root?: string | undefined;
    query?: string | undefined;
    limit?: number | undefined;
    cursor?: string | undefined;
}
export interface SourceSearchResponse {
    items?: SearchHit[] | undefined;
    nextCursor?: string | undefined;
}
export declare const ResourceSummary: MessageFns<ResourceSummary>;
export declare const SourceListRequest: MessageFns<SourceListRequest>;
export declare const SourceListResponse: MessageFns<SourceListResponse>;
export declare const SourceReadRequest: MessageFns<SourceReadRequest>;
export declare const SourceReadResponse: MessageFns<SourceReadResponse>;
export declare const SearchHit: MessageFns<SearchHit>;
export declare const SourceSearchRequest: MessageFns<SourceSearchRequest>;
export declare const SourceSearchResponse: MessageFns<SourceSearchResponse>;
type Builtin = Date | Function | Uint8Array | string | number | boolean | undefined;
export type DeepPartial<T> = T extends Builtin ? T : T extends globalThis.Array<infer U> ? globalThis.Array<DeepPartial<U>> : T extends ReadonlyArray<infer U> ? ReadonlyArray<DeepPartial<U>> : T extends {} ? {
    [K in keyof T]?: DeepPartial<T[K]>;
} : Partial<T>;
type KeysOfUnion<T> = T extends T ? keyof T : never;
export type Exact<P, I extends P> = P extends Builtin ? P : P & {
    [K in keyof P]: Exact<P[K], I[K]>;
} & {
    [K in Exclude<keyof I, KeysOfUnion<P>>]: never;
};
export interface MessageFns<T> {
    encode(message: T, writer?: BinaryWriter): BinaryWriter;
    decode(input: BinaryReader | Uint8Array, length?: number): T;
    fromJSON(object: any): T;
    toJSON(message: T): unknown;
    create<I extends Exact<DeepPartial<T>, I>>(base?: I): T;
    fromPartial<I extends Exact<DeepPartial<T>, I>>(object: I): T;
}
export {};
