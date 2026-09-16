//! Canonical content addressing for P1 chunked content (design §5.3/§5.4).
//!
//! This is the TypeScript side of the single canonical rule frozen in
//! `docs/contracts/p1-chunking.md` §3: multi-block content is addressed by the
//! CID of a `ChunkManifest` encoded through the generated ts-proto types, so the
//! bytes are identical to the Rust `conex_proto::cid` implementation (verified
//! against `conformance/vectors/p1/chunking.json`).
import { CID } from "multiformats/cid";
import { sha256 } from "multiformats/hashes/sha2";
import * as raw from "multiformats/codecs/raw";

import { ChunkEntry, ChunkManifest, ManifestEntries } from "./generated/conex/v1/chunking";

/** Content format version for P1 manifests (`ChunkManifest.format_version`). */
export const MANIFEST_FORMAT_VERSION = 1;
/** Maximum entries per manifest level (`ManifestEntries`, chunking.proto). */
export const MANIFEST_FANOUT = 1024;
/** Default broker chunk size: 256 KiB (design §5.4). */
export const DEFAULT_CHUNK_SIZE = 262_144;

export class ContentAddressError extends Error {}

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
export async function cidForRaw(bytes: Uint8Array): Promise<string> {
  return CID.createV1(raw.code, await sha256.digest(bytes)).toString();
}

/** Parse and normalize a CID text form; rejects non-raw / non-sha2-256 / wrong length. */
export function parseContentCid(text: string): CID {
  let cid: CID;
  try {
    cid = CID.parse(text);
  } catch (error) {
    throw new ContentAddressError(`invalid CID: ${text} (${error})`);
  }
  if (cid.code !== raw.code) {
    throw new ContentAddressError(`unsupported content codec ${cid.code}`);
  }
  if (cid.multihash.code !== sha256.code) {
    throw new ContentAddressError(`unsupported multihash code ${cid.multihash.code}`);
  }
  if (cid.multihash.digest.length !== 32) {
    throw new ContentAddressError(
      `sha2-256 digest must be 32 bytes, got ${cid.multihash.digest.length}`,
    );
  }
  return cid;
}

/** Split bytes into fixed-size chunks (last chunk may be shorter). */
export function chunkPayload(bytes: Uint8Array, chunkSize = DEFAULT_CHUNK_SIZE): Uint8Array[] {
  if (!Number.isInteger(chunkSize) || chunkSize <= 0) {
    throw new ContentAddressError("chunkSize must be > 0");
  }
  const chunks: Uint8Array[] = [];
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    chunks.push(bytes.subarray(offset, Math.min(offset + chunkSize, bytes.length)));
  }
  return chunks;
}

/** Leaf CIDs for a payload, in ascending chunk order, plus its total length. */
export async function leafCidsFor(
  bytes: Uint8Array,
  chunkSize = DEFAULT_CHUNK_SIZE,
): Promise<{ leaves: string[]; total: number }> {
  const leaves = await Promise.all(chunkPayload(bytes, chunkSize).map(cidForRaw));
  return { leaves, total: bytes.length };
}

function leafCount(contentLength: number, chunkSize: number): number {
  return Math.ceil(contentLength / chunkSize);
}

function leafLength(contentLength: number, chunkSize: number, index: number): number {
  return Math.min(chunkSize, contentLength - index * chunkSize);
}

function leafManifestBytes(
  chunkSize: number,
  contentLength: number,
  leaves: string[],
  base: number,
): Uint8Array {
  const entries: ChunkEntry[] = leaves.map((cid, offset) => ({
    chunkCid: cid,
    chunkLength: String(leafLength(contentLength, chunkSize, base + offset)),
  }));
  return ChunkManifest.encode({
    formatVersion: MANIFEST_FORMAT_VERSION,
    chunkSize,
    contentLength: String(contentLength),
    root: { leaves: entries, childManifestCids: [] },
  }).finish();
}

function parentManifestBytes(
  chunkSize: number,
  contentLength: number,
  children: string[],
): Uint8Array {
  const root: ManifestEntries = { leaves: [], childManifestCids: children };
  return ChunkManifest.encode({
    formatVersion: MANIFEST_FORMAT_VERSION,
    chunkSize,
    contentLength: String(contentLength),
    root,
  }).finish();
}

/** Build the canonical manifest tree for chunked content. */
export async function manifestTree(
  chunkSize: number,
  contentLength: number,
  leafCids: string[],
): Promise<ManifestTree> {
  if (!Number.isInteger(chunkSize) || chunkSize <= 0) {
    throw new ContentAddressError("chunkSize must be > 0");
  }
  if (contentLength <= chunkSize) {
    throw new ContentAddressError(
      `contentLength ${contentLength} fits in one chunk (${chunkSize}); single-block content uses the raw CID`,
    );
  }
  const expected = leafCount(contentLength, chunkSize);
  if (leafCids.length !== expected) {
    throw new ContentAddressError(
      `expected ${expected} leaves for ${contentLength} bytes at chunkSize ${chunkSize}, got ${leafCids.length}`,
    );
  }
  for (const cid of leafCids) {
    parseContentCid(cid);
  }

  const manifests: ManifestNode[] = [];
  let level: string[] = [];
  for (let groupIndex = 0; groupIndex * MANIFEST_FANOUT < leafCids.length; groupIndex += 1) {
    const group = leafCids.slice(
      groupIndex * MANIFEST_FANOUT,
      (groupIndex + 1) * MANIFEST_FANOUT,
    );
    const bytes = leafManifestBytes(
      chunkSize,
      contentLength,
      group,
      groupIndex * MANIFEST_FANOUT,
    );
    const cid = await cidForRaw(bytes);
    level.push(cid);
    manifests.push({ cid, bytes });
  }
  while (level.length > 1) {
    const next: string[] = [];
    for (let offset = 0; offset < level.length; offset += MANIFEST_FANOUT) {
      const group = level.slice(offset, offset + MANIFEST_FANOUT);
      const bytes = parentManifestBytes(chunkSize, contentLength, group);
      const cid = await cidForRaw(bytes);
      next.push(cid);
      manifests.push({ cid, bytes });
    }
    level = next;
  }
  return { rootCid: level[0]!, manifests, leafCids: [...leafCids] };
}

/** Root CID for chunked content addressed by explicit leaf CIDs. */
export async function contentCidForParts(
  chunkSize: number,
  contentLength: number,
  leafCids: string[],
): Promise<string> {
  return (await manifestTree(chunkSize, contentLength, leafCids)).rootCid;
}

/**
 * Unified content addressing function (design §5.3): raw CID for single-block
 * content, manifest root CID otherwise. Every `blob/*` root and `source/read`
 * CID must come from this function.
 */
export async function contentCid(
  bytes: Uint8Array,
  chunkSize = DEFAULT_CHUNK_SIZE,
): Promise<string> {
  if (bytes.length <= chunkSize) {
    return cidForRaw(bytes);
  }
  const { leaves, total } = await leafCidsFor(bytes, chunkSize);
  return contentCidForParts(chunkSize, total, leaves);
}
