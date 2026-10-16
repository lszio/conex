// Content addressing for the share scene, on the browser side of the wire.
//
// The host binds a staged upload to a *content*, not to a transport attempt:
// the client declares its chunk CIDs before sending any bytes, and the host
// derives the same root CID the SDK would (`conex_proto::cid::content_cid`).
// That is what makes resume meaningful — retrying the same file finds the same
// staged upload instead of starting over, and the client needs to remember
// nothing but the file it already picked.
//
// This reproduces `conex_proto::cid::cid_for_raw` exactly: CIDv1, raw codec,
// sha2-256, lowercase base32 multibase text. Verified against the frozen
// conformance vector in `cid.test.ts`. It deliberately does not build the
// manifest tree: that half lives on the host, where the single implementation
// already is, and shipping it here would mean two implementations of one rule.

/** Fixed chunk size for the share scene. Mirrors `conex_proto::cid::CHUNK_SIZE`
 *  and `crates/conex-host/src/share_blocks.rs::SHARE_CHUNK_SIZE`. */
export const SHARE_CHUNK_SIZE = 262_144;

const BASE32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

/** RFC 4648 base32, lowercase, **unpadded** — the multibase `b` text form. */
export function base32Lower(bytes: Uint8Array): string {
  let out = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += BASE32_ALPHABET[(buffer >>> bits) & 31];
    }
  }
  if (bits > 0) {
    out += BASE32_ALPHABET[(buffer << (5 - bits)) & 31];
  }
  return out;
}

/** CIDv1 / raw / sha2-256 of one chunk, as lowercase base32 text. */
export async function cidForRaw(bytes: Uint8Array): Promise<string> {
  // Typed as an ArrayBuffer view because the DOM's `digest` requires one: this
  // module only ever receives real ArrayBuffers, never a SharedArrayBuffer.
  const view = bytes as Uint8Array<ArrayBuffer>;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", view));
  // CIDv1 (0x01) + raw codec (0x55) + sha2-256 multihash prefix + digest.
  const cidBytes = new Uint8Array(4 + digest.length);
  cidBytes[0] = 0x01;
  cidBytes[1] = 0x55;
  cidBytes[2] = 0x12;
  cidBytes[3] = 0x20;
  cidBytes.set(digest, 4);
  return `b${base32Lower(cidBytes)}`;
}

/** One chunk's identity plus the bytes it covers. */
export interface DeclaredChunk {
  cid: string;
  bytes: Uint8Array;
}

/**
 * Hash a whole file into ordered chunk CIDs, without holding it in memory.
 *
 * Reads one chunk at a time, so a multi-gigabyte upload costs one chunk of
 * memory on the client and one pass of sha256. The CIDs are the only thing the
 * client keeps: they are what the host checks each arriving chunk against, and
 * what makes a retry resumable rather than a restart.
 */
export async function declareChunks(
  file: Blob,
  chunkSize: number = SHARE_CHUNK_SIZE,
): Promise<{ size: number; chunkCids: string[] }> {
  if (!Number.isInteger(chunkSize) || chunkSize <= 0) {
    throw new Error("chunkSize must be > 0");
  }
  const size = file.size;
  const chunkCids: string[] = [];
  for (let offset = 0; offset < size; offset += chunkSize) {
    const slice = file.slice(offset, Math.min(offset + chunkSize, size));
    chunkCids.push(await cidForRaw(new Uint8Array(await slice.arrayBuffer())));
  }
  return { size, chunkCids };
}

/** Chunk count for a size at this chunk size; the host computes the same one. */
export function chunkCount(size: number, chunkSize: number = SHARE_CHUNK_SIZE): number {
  return Math.ceil(size / chunkSize);
}
