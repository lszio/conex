import { declareChunks } from "./cid";

// Same-origin client for the group-scoped share routes.
//
// Every request carries the session cookie; writes add the CSRF nonce the
// host handed out with `/web/session`, which is the same dance the SDK's ticket
// request already performs. Names travel percent-encoded because a header
// cannot hold arbitrary UTF-8.
//
// Two upload shapes live here on purpose. `uploadFile` is the whole-body
// convenience route for a small file; `uploadInChunks` is the one worth
// resuming, and it is what a big file takes. Neither is a special case in the
// store: both end as chunks in one staged upload, and the split exists only
// because one round trip is cheaper than five when the file is small.

/** `file` is a picked file, `clip` is clipboard content the owner offered.
 *  Both travel the same route and the same download path; a clip is smaller,
 *  expires, and is counted against its own budget. */
export type ShareKind = "file" | "clip";

export interface SharedFile {
  id: string;
  kind: ShareKind;
  name: string;
  mime: string;
  size: number;
  ownerLinkId: string;
  ownerName: string;
  createdAtMs: number;
  /** Epoch milliseconds after which a clip stops being served; 0 for a file. */
  expiresAtMs: number;
  inline: boolean;
  /** Leading bytes the host offers for a preview read. 0 when the item may
   *  not render, which is what keeps a preview from pulling the whole file. */
  previewBytes: number;
}

export interface FileListing {
  groupKey: string;
  label: string;
  selfLinkId: string;
  files: SharedFile[];
  limits: {
    maxFileBytes: number;
    maxFilesPerClient: number;
    maxClipsPerClient: number;
    clipTtlMs: number;
  };
}

let csrfToken = "";
let linkId = "";

export function setCsrfToken(token: string): void {
  csrfToken = token;
}

/**
 * The tab's own link, sent on every share request.
 *
 * Two tabs share one session cookie, so the cookie says who is browsing and
 * this says which tab is asking. The host checks the link belongs to the
 * session, which is what keeps a tab from acting as its sibling.
 */
export function setLinkId(value: string): void {
  linkId = value;
}

function authHeaders(): Record<string, string> {
  return linkId ? { "x-conex-link-id": linkId } : {};
}

function query(params: Record<string, string>): string {
  return new URLSearchParams(params).toString();
}

async function failure(response: Response): Promise<never> {
  let message = `HTTP ${response.status}`;
  try {
    const body = (await response.json()) as { message?: string };
    if (body?.message) message = body.message;
  } catch {
    // A non-JSON error body keeps the status; the message is a nicety.
  }
  throw new Error(message);
}

/** The last listing's validator, so an unchanged listing costs one `304`.
 *  Kept per tab rather than per page: two tabs of one browser are two clients,
 *  and sharing this would make one tab's listing answer another's validator. */
let listingEtag = "";
/// The host's limits, remembered from the last full listing.
///
/// Remembered rather than re-read because they are host configuration, not
/// per-request state: a conditional listing answers `304` with no body, and a
/// caller that then needed the limits would have to force a full read it does
/// not otherwise need.
let lastLimits: FileListing["limits"] | null = null;

/** The host's per-item ceiling, from the last full listing it sent. */
export function listingLimits(): FileListing["limits"] | null {
  return lastLimits ? { ...lastLimits } : null;
}

export async function listFiles(): Promise<FileListing | null> {
  const response = await fetch("/web/files", {
    credentials: "same-origin",
    headers: { ...authHeaders(), ...(listingEtag ? { "if-none-match": listingEtag } : {}) },
  });
  // An unchanged listing is not a failure, and it carries no body to re-read:
  // the scene is exactly what this tab last showed.
  if (response.status === 304) return null;
  if (!response.ok) await failure(response);
  listingEtag = response.headers.get("etag") ?? "";
  const body = (await response.json()) as Omit<FileListing, "limits"> & {
    limits?: {
      maxFileBytes?: string;
      maxFilesPerClient?: string;
      maxClipsPerClient?: string;
      clipTtlMs?: string;
    };
  };
  const limits = {
    maxFileBytes: Number(body.limits?.maxFileBytes ?? "0"),
    maxFilesPerClient: Number(body.limits?.maxFilesPerClient ?? "0"),
    maxClipsPerClient: Number(body.limits?.maxClipsPerClient ?? "0"),
    clipTtlMs: Number(body.limits?.clipTtlMs ?? "0"),
  };
  lastLimits = limits;
  return { ...body, limits };
}

/** Chunk size the scene fixed. The host refuses any other value, which is why
 *  this is a constant and not a negotiation. */
const CHUNK_SIZE = 262_144;

/** Progress of one chunked upload, for the page to render. */
export interface UploadProgress {
  sent: number;
  total: number;
  label: string;
}

export interface ChunkUploadOptions {
  /** Called as chunks land, so a visitor can watch a real number move. */
  onProgress?: (progress: UploadProgress) => void;
}

/**
 * Upload one file in chunks, resuming an interrupted attempt for free.
 *
 * The client declares the CIDs of the chunks it is about to send, then sends
 * them one at a time. Because the host binds staging to that *content*, asking
 * again for the same file returns how far it got — so a dropped connection and
 * a reloaded tab are the same case, and the client keeps no upload state of its
 * own: it re-declares what it was always going to send.
 */
export async function uploadInChunks(
  file: File,
  kind: ShareKind = "file",
  options: ChunkUploadOptions = {},
): Promise<SharedFile> {
  const declared = await declareChunks(file);
  const started = await beginUpload(file, declared.size, declared.chunkCids, kind);
  let receivedCount = started.receivedCount;
  options.onProgress?.({ sent: receivedCount, total: declared.chunkCids.length, label: file.name });

  for (let index = receivedCount; index < declared.chunkCids.length; index += 1) {
    const slice = file.slice(index * CHUNK_SIZE, Math.min((index + 1) * CHUNK_SIZE, declared.size));
    const bytes = await slice.arrayBuffer();
    receivedCount = await putChunk(started.uploadId, index, bytes);
    options.onProgress?.({
      sent: receivedCount,
      total: declared.chunkCids.length,
      label: file.name,
    });
  }
  return commitUpload(started.uploadId);
}

/** Begin, or rejoin, the staged upload for this content. */
async function beginUpload(
  file: File,
  size: number,
  chunkCids: string[],
  kind: ShareKind,
): Promise<{ uploadId: string; receivedCount: number; chunkSize: number }> {
  const response = await fetch("/web/files/uploads", {
    method: "POST",
    credentials: "same-origin",
    headers: {
      ...authHeaders(),
      "x-csrf-token": csrfToken,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      name: file.webkitRelativePath || file.name,
      mime: file.type || "application/octet-stream",
      sizeBytes: String(size),
      chunkSize: CHUNK_SIZE,
      chunkCids,
      kind,
    }),
  });
  if (!response.ok) await failure(response);
  const body = (await response.json()) as {
    uploadId: string;
    receivedCount: string;
    chunkSize: string;
  };
  return {
    uploadId: body.uploadId,
    receivedCount: Number(body.receivedCount ?? "0"),
    chunkSize: Number(body.chunkSize ?? "0"),
  };
}

async function putChunk(uploadId: string, index: number, bytes: ArrayBuffer): Promise<number> {
  const response = await fetch(
    `/web/files/uploads/${encodeURIComponent(uploadId)}/chunks/${index}`,
    {
      method: "PUT",
      credentials: "same-origin",
      headers: {
        ...authHeaders(),
        "x-csrf-token": csrfToken,
        "content-type": "application/octet-stream",
      },
      body: bytes,
    },
  );
  if (!response.ok) await failure(response);
  const count = response.headers.get("x-conex-received-count");
  return Number(count ?? "0");
}

async function commitUpload(uploadId: string): Promise<SharedFile> {
  const response = await fetch(
    `/web/files/uploads/${encodeURIComponent(uploadId)}/commit`,
    {
      method: "POST",
      credentials: "same-origin",
      headers: { ...authHeaders(), "x-csrf-token": csrfToken },
    },
  );
  if (!response.ok) await failure(response);
  const body = (await response.json()) as { file: SharedFile };
  return body.file;
}

export async function uploadFile(file: File): Promise<SharedFile> {
  const response = await fetch("/web/files", {
    method: "POST",
    credentials: "same-origin",
    headers: {
      ...authHeaders(),
      "x-csrf-token": csrfToken,
      "x-conex-file-name": encodeURIComponent(file.webkitRelativePath || file.name),
      "x-conex-file-type": encodeURIComponent(file.type || "application/octet-stream"),
      "content-type": "application/octet-stream",
    },
    body: file,
  });
  if (!response.ok) await failure(response);
  const body = (await response.json()) as { file: SharedFile };
  return body.file;
}

export async function removeFile(id: string): Promise<void> {
  const response = await fetch("/web/files/remove", {
    method: "POST",
    credentials: "same-origin",
    headers: { ...authHeaders(), "x-csrf-token": csrfToken, "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  if (!response.ok) await failure(response);
}

/** Preview and download both go through the same URL; the host decides
 *  `inline` vs `attachment` from the file's own MIME type. The link travels in
 *  the query because a plain `<a href>` cannot set a header. */
export function downloadFileUrl(file: SharedFile): string {
  return `/web/files/download?${query(linkId ? { id: file.id, linkId } : { id: file.id })}`;
}

/** URL that reads only the head of an item, for a preview.
 *
 *  The range is derived from the host's own `previewBytes`, so the page never
 *  decides how much is safe to fetch. A host that answers `previewBytes: 0`
 *  means "not previewable", and the page renders the download link instead. */
export function previewFileUrl(file: SharedFile): string {
  const length = Math.min(file.previewBytes, Math.max(file.size, 1));
  return `${downloadFileUrl(file)}&range=bytes=0-${length - 1}`;
}
