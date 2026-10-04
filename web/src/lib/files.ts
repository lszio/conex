// Same-origin client for the group-scoped share routes.
//
// Every request carries the session cookie; writes add the CSRF nonce the
// host handed out with `/web/session`, which is the same dance the SDK's ticket
// request already performs. Names travel percent-encoded because a header
// cannot hold arbitrary UTF-8.

export interface SharedFile {
  id: string;
  name: string;
  mime: string;
  size: number;
  ownerLinkId: string;
  ownerName: string;
  createdAtMs: number;
  inline: boolean;
}

export interface FileListing {
  groupKey: string;
  label: string;
  selfLinkId: string;
  files: SharedFile[];
  limits: { maxFileBytes: number; maxFilesPerClient: number };
}

let csrfToken = "";

export function setCsrfToken(token: string): void {
  csrfToken = token;
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

export async function listFiles(): Promise<FileListing> {
  const response = await fetch("/web/files", { credentials: "same-origin" });
  if (!response.ok) await failure(response);
  const body = (await response.json()) as Omit<FileListing, "limits"> & {
    limits?: { maxFileBytes?: string; maxFilesPerClient?: string };
  };
  return {
    ...body,
    limits: {
      maxFileBytes: Number(body.limits?.maxFileBytes ?? "0"),
      maxFilesPerClient: Number(body.limits?.maxFilesPerClient ?? "0"),
    },
  };
}

export async function uploadFile(file: File): Promise<SharedFile> {
  const response = await fetch("/web/files", {
    method: "POST",
    credentials: "same-origin",
    headers: {
      "x-csrf-token": csrfToken,
      "x-conex-file-name": encodeURIComponent(file.name),
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
    headers: { "x-csrf-token": csrfToken, "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  if (!response.ok) await failure(response);
}

/** Preview and download both go through the same URL; the host decides
 *  `inline` vs `attachment` from the file's own MIME type. */
export function downloadFileUrl(file: SharedFile): string {
  return `/web/files/download?${query({ id: file.id })}`;
}
