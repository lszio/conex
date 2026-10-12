import { CID } from "multiformats/cid";
/** Content format version for P1 manifests (`ChunkManifest.format_version`). */
export declare const MANIFEST_FORMAT_VERSION = 1;
/** Maximum entries per manifest level (`ManifestEntries`, chunking.proto). */
export declare const MANIFEST_FANOUT = 1024;
/** Default broker chunk size: 256 KiB (design §5.4). */
export declare const DEFAULT_CHUNK_SIZE = 262144;
export declare class ContentAddressError extends Error {
}
export interface ManifestNode {
    cid: string;
    bytes: Uint8Array;
}
export interface ManifestTree {
    rootCid: string;
    /** Leaf manifests first, then each parent level, root manifest last. */
    manifests: ManifestNode[];
    leafCids: string[];
}
/** CIDv1 / raw / SHA-256, lowercase base32 text. */
export declare function cidForRaw(bytes: Uint8Array): Promise<string>;
/** Parse and normalize a CID text form; rejects non-raw / non-sha2-256 / wrong length. */
export declare function parseContentCid(text: string): CID;
/** Split bytes into fixed-size chunks (last chunk may be shorter). */
export declare function chunkPayload(bytes: Uint8Array, chunkSize?: number): Uint8Array[];
/** Leaf CIDs for a payload, in ascending chunk order, plus its total length. */
export declare function leafCidsFor(bytes: Uint8Array, chunkSize?: number): Promise<{
    leaves: string[];
    total: number;
}>;
/** Build the canonical manifest tree for chunked content. */
export declare function manifestTree(chunkSize: number, contentLength: number, leafCids: string[]): Promise<ManifestTree>;
/** Root CID for chunked content addressed by explicit leaf CIDs. */
export declare function contentCidForParts(chunkSize: number, contentLength: number, leafCids: string[]): Promise<string>;
/**
 * Unified content addressing function (design §5.3): raw CID for single-block
 * content, manifest root CID otherwise. Every `blob/*` root and `source/read`
 * CID must come from this function.
 */
export declare function contentCid(bytes: Uint8Array, chunkSize?: number): Promise<string>;
