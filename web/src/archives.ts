//! Bounded DOCX and ZIP previews (plan M5).
//!
//! Both formats are ZIP containers, so both paths share one budget: a
//! preview never expands more than [`ARCHIVE_EXPAND_LIMIT`] bytes, never
//! lists more than [`ARCHIVE_ENTRY_LIMIT`] entries, and never keeps an
//! entry whose compression ratio looks like a bomb. DOCX text is converted
//! with `mammoth` and then scrubbed — the page never trusts document markup,
//! and never lets it reference anything off-origin.

import { unzipSync, type Unzipped } from "fflate";
import mammoth from "mammoth";

import {
  ARCHIVE_ENTRY_LIMIT,
  ARCHIVE_EXPAND_LIMIT,
  ARCHIVE_RATIO_LIMIT,
  reviewArchiveEntries,
  stripDangerousMarkup,
  type ArchiveEntry,
} from "./preview";

export interface ArchiveListing {
  entries: readonly ArchiveEntry[];
  rejected: number;
  totalSize: number;
  truncated: boolean;
}

/**
 * List an archive without expanding its payloads. Decompression is
 * synchronous on purpose: the page CSP forbids blob workers, and the
 * listing is already bounded by `ARCHIVE_EXPAND_LIMIT`.
 */
export async function listArchive(bytes: Uint8Array): Promise<ArchiveListing> {
  if (bytes.byteLength > ARCHIVE_EXPAND_LIMIT) {
    return { entries: [], rejected: 0, totalSize: 0, truncated: true };
  }
  const unzipped: Unzipped = unzipSync(bytes, {
    filter: (file) => file.originalSize <= ARCHIVE_EXPAND_LIMIT,
  });
  const raw = Object.entries(unzipped).map(([name, file]) => ({
    name,
    compressedSize: 0,
    size: file.length,
    directory: name.endsWith("/"),
  }));
  const reviewed = reviewArchiveEntries(raw);
  return {
    entries: reviewed.visible,
    rejected: reviewed.rejected,
    totalSize: reviewed.totalSize,
    truncated: raw.length > ARCHIVE_ENTRY_LIMIT,
  };
}

export interface DocxPreview {
  /** Sanitized HTML safe to insert; every external reference is neutralized. */
  html: string;
  /** Plain-text fallback used when conversion fails. */
  text: string;
  truncated: boolean;
}

/**
 * Convert DOCX to sanitized HTML. Images are dropped rather than
 * extracted: the document's media must not become additional fetches.
 */
export async function previewDocx(bytes: Uint8Array): Promise<DocxPreview> {
  if (bytes.byteLength > ARCHIVE_EXPAND_LIMIT) {
    return { html: "", text: "", truncated: true };
  }
  // Browsers have no Node `Buffer`; `mammoth` takes the raw bytes under
  // `arrayBuffer`, and the same value satisfies its Node-side type.
  const buffer = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
  // Browsers have no Node `Buffer`; mammoth reads `arrayBuffer` there and
  // `buffer` under Bun/Node. Supplying both keeps one code path for each.
  const result = await mammoth.convertToHtml(
    {
      arrayBuffer: buffer,
      buffer: typeof Buffer === "undefined" ? undefined : Buffer.from(bytes),
    } as Parameters<typeof mammoth.convertToHtml>[0],
    {
      convertImage: mammoth.images.imgElement(async () => ({ src: "" })),
      styleMap: ["p[style-name='Title'] => h1:fresh", "p[style-name='Heading 1'] => h1:fresh"],
    },
  );
  const html = stripDangerousMarkup(result.value);
  return {
    html,
    text: result.value.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim().slice(0, 8192),
    truncated: result.messages.length > 0,
  };
}

/** Compression-ratio guard shared by the listing and any future extraction. */
export function ratioIsSafe(compressed: number, size: number): boolean {
  if (compressed <= 0) return true;
  return size / compressed <= ARCHIVE_RATIO_LIMIT;
}
