// The file scene: offer local files to your group, browse what peers offered.
//
// Each provider gets a card so it is obvious who a file came from — the
// isolation boundary is the group, and a flat list would hide who is actually
// sharing. Preview is the host's decision, not the page's: it answers
// `inline` only for types that cannot carry script, so an HTML or SVG upload
// downloads instead of rendering in someone else's session.

import { useEffect, useRef, useState } from "react";
import { Download, FileIcon, FolderOpen, Trash2, Upload } from "lucide-react";

import { Badge, Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import type { SharedFile } from "@/lib/files";

function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

/** Text previews are truncated: a multi-megabyte file must not be pulled into
 *  the DOM just because it happens to be readable. Download gets the rest. */
const PREVIEW_LIMIT = 200 * 1024;

function isText(file: SharedFile): boolean {
  return (
    file.inline &&
    (file.mime.startsWith("text/") ||
      file.mime === "application/json" ||
      file.mime === "application/pdf")
  );
}

function isImage(file: SharedFile): boolean {
  return file.inline && file.mime.startsWith("image/");
}

function isPdf(file: SharedFile): boolean {
  return file.inline && file.mime === "application/pdf";
}

export function FileScene({
  files,
  selfLinkId,
  groupLabel,
  groupKey,
  notice,
  uploading,
  onShare,
  onWithdraw,
  fileUrl,
}: {
  files: SharedFile[];
  selfLinkId: string;
  groupLabel: string;
  groupKey: string;
  notice: string;
  uploading: boolean;
  onShare: (picked: FileList) => void;
  onWithdraw: (file: SharedFile) => void;
  fileUrl: (file: SharedFile) => string;
}) {
  const picker = useRef<HTMLInputElement>(null);
  // One card per provider, own files first so a visitor sees their own
  // contribution without hunting for it.
  const providers = new Map<string, { name: string; files: SharedFile[] }>();
  for (const file of files) {
    const entry = providers.get(file.ownerLinkId) ?? { name: file.ownerName, files: [] };
    entry.files.push(file);
    providers.set(file.ownerLinkId, entry);
  }
  const cards = [...providers.entries()].sort((a, b) => {
    if (a[0] === selfLinkId) return -1;
    if (b[0] === selfLinkId) return 1;
    return a[1].name.localeCompare(b[1].name);
  });

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0">
          <CardTitle>共享文件给同组</CardTitle>
          <Badge variant="secondary">
            {groupLabel || "未命名分组"} · {groupKey.slice(0, 8)}
          </Badge>
        </CardHeader>
        <CardContent className="grid gap-3">
          <p className="text-sm text-[var(--color-muted-foreground)]">
            选一个本地目录里的文件或若干文件，它们会出现在同组其他客户端的卡片里。
            其他组的客户端看不到，也猜不到文件 id。
          </p>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              onClick={() => picker.current?.click()}
              disabled={uploading}
            >
              <Upload className="size-4" aria-hidden />
              {uploading ? "上传中…" : "选择文件"}
            </Button>
            <input
              ref={picker}
              type="file"
              multiple
              className="sr-only"
              aria-label="选择要共享的文件"
              onChange={(event) => {
                if (event.target.files?.length) onShare(event.target.files);
                // Reset so picking the same file again still fires a change.
                event.target.value = "";
              }}
            />
            <label className="inline-flex cursor-pointer items-center gap-2 text-sm text-[var(--color-muted-foreground)] underline underline-offset-4">
              <FolderOpen className="size-4" aria-hidden />
              选择整个目录
              {/* The two real inputs are the mechanism; the button and this
                  label are the controls, so the browser's own file widget
                  stays out of the layout. */}
              <input
                type="file"
                multiple
                {...{ webkitdirectory: "", directory: "" }}
                className="sr-only"
                aria-label="选择整个目录"
                onChange={(event) => {
                  const picked = event.target.files;
                  if (picked?.length) onShare(picked);
                  event.target.value = "";
                }}
              />
            </label>
          </div>
          {notice ? (
            <p className="text-xs text-[var(--color-muted-foreground)]" role="status">
              {notice}
            </p>
          ) : null}
        </CardContent>
      </Card>

      {cards.length === 0 ? (
        <Card>
          <CardContent>
            <p className="rounded-md border border-dashed border-[var(--color-border)] p-6 text-center text-sm text-[var(--color-muted-foreground)]">
              这个组还没有人共享文件。选一个文件，或让同组的人先发一个。
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-3">
          {cards.map(([ownerId, provider]) => (
            <Card key={ownerId}>
              <CardHeader className="flex-row items-center justify-between space-y-0">
                <CardTitle className="text-sm">
                  {provider.name}
                  {ownerId === selfLinkId ? "（你）" : ""}
                </CardTitle>
                <Badge variant="outline">{provider.files.length} 个文件</Badge>
              </CardHeader>
              <CardContent className="grid gap-2">
                {provider.files.map((file) => (
                  <FileRow
                    key={file.id}
                    file={file}
                    own={file.ownerLinkId === selfLinkId}
                    onWithdraw={() => onWithdraw(file)}
                    fileUrl={fileUrl}
                  />
                ))}
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}

function FileRow({
  file,
  own,
  onWithdraw,
  fileUrl,
}: {
  file: SharedFile;
  own: boolean;
  onWithdraw: () => void;
  fileUrl: (file: SharedFile) => string;
}) {
  return (
    <div className="grid gap-2 rounded-md border border-[var(--color-border)] px-3 py-2.5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-2">
          <FileIcon className="size-4 shrink-0 text-[var(--color-muted-foreground)]" aria-hidden />
          <span className="truncate text-sm font-medium">{file.name}</span>
        </span>
        <span className="flex items-center gap-2">
          <span className="text-xs tabular-nums text-[var(--color-muted-foreground)]">
            {file.mime} · {bytes(file.size)}
          </span>
          <a
            className="inline-flex items-center gap-1 text-xs font-medium text-[var(--color-primary)] underline underline-offset-4"
            href={fileUrl(file)}
            download={file.name}
          >
            <Download className="size-3" aria-hidden />
            下载
          </a>
          {own ? (
            <Button variant="ghost" size="sm" onClick={onWithdraw}>
              <Trash2 className="size-3.5" aria-hidden />
              撤回
            </Button>
          ) : null}
        </span>
      </div>
      {isImage(file) ? (
        <img
          src={fileUrl(file)}
          alt={`${file.name} 的预览`}
          className="max-h-64 w-fit rounded-md border border-[var(--color-border)]"
          loading="lazy"
        />
      ) : null}
      {isPdf(file) ? (
        <object
          data={fileUrl(file)}
          type="application/pdf"
          className="h-64 w-full rounded-md border border-[var(--color-border)]"
          aria-label={`${file.name} 的预览`}
        >
          {/* The object element silently renders nothing when a browser
              refuses the type, so the download link is the fallback. */}
          <a className="text-sm underline" href={fileUrl(file)} download={file.name}>
            下载 {file.name}
          </a>
        </object>
      ) : null}
      {isText(file) ? <TextPreview file={file} fileUrl={fileUrl} /> : null}
      {!file.inline ? (
        <p className="text-xs text-[var(--color-muted-foreground)]">
          {file.mime} 不在可预览范围内（可能携带活动内容），只提供下载。
        </p>
      ) : null}
    </div>
  );
}

function TextPreview({ file, fileUrl }: { file: SharedFile; fileUrl: (file: SharedFile) => string }) {
  const url = fileUrl(file);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // An oversized file is refused with a reason rather than pulled into the
    // DOM; text is fetched, never inlined, so a hostile payload cannot be
    // handed to React as markup.
    if (file.size > PREVIEW_LIMIT) {
      setText(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setText(null);
    setError(null);
    void fetch(url, { credentials: "same-origin" })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.arrayBuffer();
      })
      .then((body) => {
        // Decoded explicitly rather than through `response.text()`: the host
        // declares utf-8, and letting fetch re-decode is how a CJK file turns
        // into replacement characters in the preview.
        if (!cancelled) setText(new TextDecoder("utf-8").decode(body));
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [file.size, url]);

  return (
    <details>
      <summary className="cursor-pointer text-xs text-[var(--color-muted-foreground)]">
        预览文本
      </summary>
      <p className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-[var(--color-muted)] p-2 text-xs">
        {file.size > PREVIEW_LIMIT
          ? `文件 ${bytes(file.size)}，超过 ${bytes(PREVIEW_LIMIT)} 预览上限，请下载查看。`
          : error
            ? `预览失败：${error}`
            : (text ?? "读取中…")}
      </p>
    </details>
  );
}
