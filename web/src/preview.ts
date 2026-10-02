//! Format classification and safe preview plumbing for the landing page
//! (plan M5).
//!
//! Classification is decided by the resource metadata the Host already
//! returns; the browser never sniffs bytes to decide what a file "is".
//! Every preview is bounded: a preview budget caps bytes fetched, expanded
//! archive members, compression ratio and DOM size, so a hostile document
//! cannot exhaust the page.

/** Formats the page can render natively with same-origin primitives. */
export type PreviewKind =
  | "text"
  | "markdown"
  | "image"
  | "video"
  | "docx"
  | "zip"
  | "binary";

/** Maximum bytes a preview may download; larger resources stay download-only. */
export const PREVIEW_BYTE_LIMIT = 8 * 1024 * 1024;
/** Maximum expanded bytes when unpacking an archive. */
export const ARCHIVE_EXPAND_LIMIT = 32 * 1024 * 1024;
/** Maximum number of entries listed from an archive. */
export const ARCHIVE_ENTRY_LIMIT = 2_000;
/** Maximum compression ratio accepted before an entry is refused. */
export const ARCHIVE_RATIO_LIMIT = 200;
/** Maximum text characters rendered in one pane. */
export const TEXT_CHAR_LIMIT = 512 * 1024;

const TEXT_EXT: Record<string, true> = {
  md: true, org: true, txt: true, log: true, csv: true, json: true, yaml: true, yml: true,
};
const IMAGE_MIME: Record<string, true> = {
  "image/png": true, "image/jpeg": true, "image/gif": true,
  "image/webp": true, "image/avif": true, "image/bmp": true, "image/svg+xml": true,
};
const VIDEO_MIME: Record<string, true> = {
  "video/mp4": true, "video/webm": true, "video/ogg": true, "video/quicktime": true,
};

export interface ResourceFacts {
  resourceId: string;
  title: string;
  mime: string;
  sizeBytes?: number;
  revision?: string;
}

/**
 * Classify by MIME first, then extension: the Host's MIME is authoritative
 * for content type, and the extension only disambiguates when the MIME is
 * the octet-stream fallback (M0 rule).
 */
export function classify(facts: ResourceFacts): PreviewKind {
  const mime = facts.mime.toLowerCase().split(";")[0].trim();
  const ext = extensionOf(facts.resourceId);
  if (mime === "text/markdown" || ext === "md") return "markdown";
  if (mime === "text/plain" || mime === "text/org" || (TEXT_EXT[ext] === true && mime.startsWith("text/"))) {
    return "text";
  }
  if (IMAGE_MIME[mime] === true) return "image";
  if (VIDEO_MIME[mime] === true) return "video";
  if (
    mime ===
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document" ||
    ext === "docx"
  ) {
    return "docx";
  }
  if (mime === "application/zip" || mime === "application/gzip" || ext === "zip") return "zip";
  return "binary";
}

function extensionOf(resourceId: string): string {
  const name = resourceId.split("/").pop() ?? resourceId;
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

/**
 * SVG is an image we refuse to render inline: it can carry script and
 * external references, and same-origin inline execution is forbidden by the
 * contract (M3.2/M5). It downloads as an attachment instead.
 */
export function rendersInline(kind: PreviewKind, mime: string): boolean {
  if (kind !== "image") return kind === "text" || kind === "markdown" || kind === "video";
  return !mime.toLowerCase().includes("svg");
}

export interface ArchiveEntry {
  name: string;
  compressedSize: number;
  size: number;
  directory: boolean;
  /** Entries rejected by the safety rules, surfaced instead of hidden. */
  rejected?: "ratio" | "size" | "entry-count";
}

/**
 * Sanitize archive entries against traversal and budget rules. Entry names
 * are treated as data: `..` segments, absolute paths and control characters
 * are refused outright rather than normalized.
 */
export function reviewArchiveEntries(
  entries: readonly { name: string; compressedSize: number; size: number; directory?: boolean }[],
): { visible: ArchiveEntry[]; rejected: number; totalSize: number } {
  const visible: ArchiveEntry[] = [];
  let rejected = 0;
  let totalSize = 0;
  for (const entry of entries) {
    if (visible.length >= ARCHIVE_ENTRY_LIMIT) {
      rejected += 1;
      continue;
    }
    if (unsafeEntryName(entry.name)) {
      rejected += 1;
      continue;
    }
    const ratio = entry.compressedSize > 0 ? entry.size / entry.compressedSize : 1;
    if (entry.compressedSize > 0 && ratio > ARCHIVE_RATIO_LIMIT) {
      visible.push({
        name: entry.name,
        compressedSize: entry.compressedSize,
        size: entry.size,
        directory: Boolean(entry.directory),
        rejected: "ratio",
      });
      rejected += 1;
      continue;
    }
    if (totalSize + entry.size > ARCHIVE_EXPAND_LIMIT) {
      visible.push({
        name: entry.name,
        compressedSize: entry.compressedSize,
        size: entry.size,
        directory: Boolean(entry.directory),
        rejected: "size",
      });
      rejected += 1;
      continue;
    }
    totalSize += entry.size;
    visible.push({
      name: entry.name,
      compressedSize: entry.compressedSize,
      size: entry.size,
      directory: Boolean(entry.directory),
    });
  }
  return { visible, rejected, totalSize };
}

function unsafeEntryName(name: string): boolean {
  if (name.length === 0 || name.length > 512) return true;
  if (name.startsWith("/") || name.startsWith("\\")) return true;
  if (/^[a-zA-Z]:/.test(name)) return true;
  // Control characters (NUL, CR, LF, …) in an entry name are data, not text.
  if (new RegExp("[\\u0000-\\u001f]").test(name)) return true;
  return name.split(/[\\/]/).some((segment) => segment === "..");
}

/**
 * Escape a decoded-HTML fragment is NOT what we do for Markdown: instead the
 * renderer below only ever emits text nodes for untrusted input. This helper
 * exists for the DOCX path, where a library returns an HTML string that must
 * be scrubbed before insertion.
 */
export function stripDangerousMarkup(html: string): string {
  return html
    // Script/style/iframe/object/embed and friends never survive.
    .replace(/<\s*(script|style|iframe|object|embed|link|meta|form|base|svg|math)\b[\s\S]*?<\s*\/\s*\1\s*>/gi, "")
    .replace(/<\s*(script|style|iframe|object|embed|link|meta|form|base)\b[^>]*\/?>/gi, "")
    // Every attribute that can fetch or execute is dropped.
    .replace(/\s(?:on[a-z]+|srcset|formaction|xlink:href|xmlns:xlink)\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "")
    .replace(/\s(?:href|src)\s*=\s*("javascript:[^"]*"|'javascript:[^']*'|javascript:[^\s>]*)/gi, ' href="#"')
    // External references (images, links) are neutralized: the preview must
    // not reach outside the Host.
    .replace(/\s(?:href|src)\s*=\s*("https?:\/\/[^"]*"|'https?:\/\/[^']*'|https?:\/\/[^\s>]*)/gi, ' href="#"');
}

/** Plain-text rendering keeps the document verbatim and bounded. */
export function clampText(text: string): { text: string; truncated: boolean } {
  if (text.length <= TEXT_CHAR_LIMIT) return { text, truncated: false };
  return { text: text.slice(0, TEXT_CHAR_LIMIT), truncated: true };
}

/** Human-readable byte size; used for display only, never for policy. */
export function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined || Number.isNaN(bytes)) return "大小未知";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

/** Map Host error codes to distinct, user-facing failure classes (M5). */
export type FailureKind =
  | "permission"
  | "missing"
  | "offline"
  | "stale"
  | "unsupported"
  | "too-large"
  | "cancelled"
  | "unknown";

export function failureKind(code: number | undefined, message: string): FailureKind {
  if (code === -32001 || code === -32002 || code === 401 || code === 403) return "permission";
  if (code === 400 && /not found|not present|resource not found/i.test(message)) return "missing";
  if (code === -32005 || /offline|unavailable|link closed/i.test(message)) return "offline";
  if (code === -32013 || /revision/i.test(message)) return "stale";
  if (code === -32004 || /unsupported|not enabled/i.test(message)) return "unsupported";
  if (code === -32007 || /exceeds|too large|budget/i.test(message)) return "too-large";
  if (code === -32011 || /cancel/i.test(message)) return "cancelled";
  return "unknown";
}

export const FAILURE_TEXT: Record<FailureKind, string> = {
  permission: "权限不足：该资源未公开给当前身份",
  missing: "资源不存在或已被移动",
  offline: "提供该内容的主机当前离线",
  stale: "内容在读取期间发生变化，请重新打开",
  unsupported: "当前环境不支持预览该格式",
  "too-large": "内容超出预览上限，请下载原文件",
  cancelled: "读取已取消",
  unknown: "操作失败",
};
