//! Host-grouped browser view (plan M5): directory navigation per host,
//! online state, paginated results, and format-aware previews that stream
//! from the same-origin `/content` endpoint.

import type { EndpointSummary } from "@conex/sdk";

import { listArchive, previewDocx } from "./archives";
import {
  FAILURE_TEXT,
  PREVIEW_BYTE_LIMIT,
  classify,
  clampText,
  failureKind,
  formatBytes,
  rendersInline,
  type ResourceFacts,
} from "./preview";

const CONNECTION_LABEL: Record<number, string> = {
  0: "未标注",
  1: "无需连接",
  2: "离线",
  3: "在线",
};

export interface ListingRow {
  resourceId: string;
  title: string;
  mime: string;
  sizeBytes?: number;
  revision?: string;
  /** Synthesized from path prefixes for flat listings (see `visibleRows`). */
  isDirectory?: boolean;
}

export interface DirectoryPage {
  rows: ListingRow[];
  nextCursor?: string;
}

export interface BrowserHandlers {
  list(endpointId: string, input: { root: string; cursor?: string; limit: number }): Promise<DirectoryPage>;
  search(endpointId: string, input: { root: string; query: string; limit: number }): Promise<DirectoryPage>;
  contentUrl(endpointId: string, resourceId: string, revision?: string): string;
  fetchRange(url: string, start?: number, end?: number): Promise<Uint8Array>;
}

/** Wire errors carry a numeric code; narrow before reading it. */
function errorCode(error: unknown): number | undefined {
  if (error && typeof error === "object" && "code" in error) {
    const code = (error as { code: unknown }).code;
    if (typeof code === "number") return code;
  }
  return undefined;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface BrowsingState {
  endpointId: string;
  /** Directory path currently listed; "" is the endpoint root. */
  root: string;
  /** Preserved across refreshes: typing must not reset navigation. */
  query: string;
  cursor?: string;
  rows: ListingRow[];
  nextCursor?: string;
  loading: boolean;
  error?: string;
  /** Media elements must survive directory refreshes. */
  playingResource?: string;
  openResource?: string;
}

export class Browser {
  private state: BrowsingState = {
    endpointId: "",
    root: "",
    query: "",
    rows: [],
    loading: false,
  };
  private pages: HTMLElement;
  private listing: HTMLElement;
  private detail: HTMLElement;

  constructor(
    private readonly host: HTMLElement,
    private readonly handlers: BrowserHandlers,
  ) {
    this.pages = document.createElement("div");
    this.pages.className = "browser-pages";
    this.listing = document.createElement("div");
    this.listing.className = "browser-listing";
    this.detail = document.createElement("div");
    this.detail.className = "browser-detail";
    this.detail.setAttribute("aria-live", "polite");
    host.append(this.pages, this.listing, this.detail);
    this.render();
  }

  /** Point the browser at another host; open state is per endpoint. */
  async openEndpoint(endpoint: EndpointSummary | undefined): Promise<void> {
    const next = endpoint?.endpointId ?? "";
    if (next === this.state.endpointId) return;
    this.state = {
      endpointId: next,
      root: "",
      query: "",
      rows: [],
      loading: false,
    };
    if (next) await this.refresh();
    else this.render();
  }

  async refresh(): Promise<void> {
    if (!this.state.endpointId) return;
    this.state.loading = true;
    this.state.error = undefined;
    this.render();
    try {
      const page = this.state.query
        ? await this.handlers.search(this.state.endpointId, {
            root: this.state.root,
            query: this.state.query,
            limit: 50,
          })
        : await this.handlers.list(this.state.endpointId, {
            root: this.state.root,
            cursor: this.state.cursor,
            limit: 50,
          });
      this.state.rows = page.rows;
      this.state.nextCursor = page.nextCursor;
    } catch (error) {
      this.state.error = FAILURE_TEXT[failureKind(errorCode(error), errorMessage(error))];
      this.state.rows = [];
    } finally {
      this.state.loading = false;
      this.render();
    }
  }

  /** Next page of the current listing; appends without losing filters. */
  async nextPage(): Promise<void> {
    if (!this.state.nextCursor) return;
    this.state.cursor = this.state.nextCursor;
    const before = this.state.rows.length;
    await this.refresh();
    // Preserve scroll position so a page append does not jump the reader.
    this.listing.querySelector("ul")?.setAttribute("data-appended", String(this.state.rows.length > before));
  }

  async goUp(): Promise<void> {
    const parent = this.state.root.split("/").slice(0, -1).join("/");
    this.state.root = parent;
    this.state.cursor = undefined;
    await this.refresh();
  }

  setQuery(query: string): void {
    this.state.query = query;
  }

  /**
   * Return to the endpoint root. Directory depth is browser state, so the
   * caller can always get back to the top without re-selecting the endpoint.
   */
  async goToRoot(): Promise<void> {
    this.state.root = "";
    this.state.cursor = undefined;
    this.state.openResource = undefined;
    this.state.playingResource = undefined;
    await this.refresh();
  }

  async openResource(row: ListingRow): Promise<void> {
    this.state.openResource = row.resourceId;
    this.state.playingResource = row.resourceId;
    this.render();
    await this.renderDetail(row);
  }

  closeResource(): void {
    // Stop playback before detaching: a playing media element that is
    // removed mid-stream keeps downloading.
    this.detail.querySelectorAll("video,audio").forEach((element) => {
      const media = element as HTMLVideoElement;
      media.pause?.();
      media.removeAttribute("src");
    });
    this.state.openResource = undefined;
    this.state.playingResource = undefined;
    this.render();
  }

  private render(): void {
    this.renderPages();
    this.renderListing();
  }

  private renderPages(): void {
    this.pages.replaceChildren();
    if (!this.state.endpointId) return;
    const crumbs = this.state.root ? this.state.root.split("/") : [];
    const trail = document.createElement("nav");
    trail.className = "browser-trail";
    trail.setAttribute("aria-label", "目录路径");
    const root = document.createElement("button");
    root.type = "button";
    root.className = "crumb";
    root.textContent = "根目录";
    // The root crumb always jumps to the endpoint root; `goUp` is for one
    // level up from a subdirectory.
    root.addEventListener("click", () => { void this.goToRoot(); });
    trail.append(root);
    let path = "";
    for (const [index, segment] of crumbs.entries()) {
      path = path ? `${path}/${segment}` : segment;
      const target = path;
      const crumb = document.createElement("button");
      crumb.type = "button";
      crumb.className = "crumb";
      crumb.textContent = segment;
      const depth = index;
      crumb.addEventListener("click", () => {
        this.state.root = target;
        this.state.cursor = undefined;
        void this.refresh();
      });
      trail.append(document.createTextNode(" / "), crumb);
      if (depth === crumbs.length - 1) crumb.setAttribute("aria-current", "location");
    }
    this.pages.append(trail);
  }

  private renderListing(): void {
    this.listing.replaceChildren();
    if (!this.state.endpointId) return;
    const bar = document.createElement("div");
    bar.className = "browser-actions";
    const up = document.createElement("button");
    up.type = "button";
    up.className = "secondary";
    up.textContent = "返回上级";
    up.disabled = !this.state.root;
    up.addEventListener("click", () => { void this.goUp(); });
    bar.append(up);
    if (this.state.query) {
      // Search always runs from the endpoint root, so the trail reflects it.
      bar.append(this.notice(`搜索「${this.state.query}」：结果来自整个可读范围`));
    }
    this.listing.append(bar);

    if (this.state.loading) {
      const loading = document.createElement("p");
      loading.className = "muted";
      loading.textContent = "正在读取目录…";
      this.listing.append(loading);
    }
    if (this.state.error) {
      const error = document.createElement("p");
      error.className = "error";
      error.setAttribute("role", "alert");
      error.textContent = this.state.error;
      this.listing.append(error);
    }
    const list = document.createElement("ul");
    list.className = "resource-list";
    for (const row of this.visibleRows()) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.className = "resource-row";
      const title = document.createElement("span");
      title.className = "resource-title";
      title.textContent = row.title;
      const meta = document.createElement("span");
      meta.className = "resource-meta";
      meta.textContent = row.isDirectory
        ? "目录"
        : `${row.mime} · ${formatBytes(row.sizeBytes)}`;
      button.append(title, meta);
      button.addEventListener("click", () => {
        if (row.isDirectory) {
          this.state.root = row.resourceId;
          this.state.cursor = undefined;
          void this.refresh();
        } else {
          void this.openResource(row);
        }
      });
      item.append(button);
      list.append(item);
    }
    this.listing.append(list);
    if (this.state.nextCursor) {
      const more = document.createElement("button");
      more.type = "button";
      more.className = "secondary";
      more.textContent = "加载更多";
      more.addEventListener("click", () => { void this.nextPage(); });
      this.listing.append(more);
    }
  }

  /**
   * `source/list` is a flat scan: rows carry their full path. Directories are
   * synthesized from the path prefix, and only the rows directly inside the
   * current root are shown, so drilling in works on any provider.
   */
  private visibleRows(): ListingRow[] {
    const prefix = this.state.root ? `${this.state.root}/` : "";
    const directories = new Map<string, ListingRow>();
    const files: ListingRow[] = [];
    for (const row of this.state.rows) {
      if (this.state.query) {
        // Search results are shown flat: the query already spans the tree.
        files.push(row);
        continue;
      }
      if (!row.resourceId.startsWith(prefix)) continue;
      const rest = row.resourceId.slice(prefix.length);
      if (!rest) continue;
      const slash = rest.indexOf("/");
      if (slash < 0) {
        files.push(row);
      } else {
        const path = `${prefix}${rest.slice(0, slash)}`;
        directories.set(path, {
          resourceId: path,
          title: rest.slice(0, slash),
          mime: "inode/directory",
          isDirectory: true,
        });
      }
    }
    const dirs = [...directories.values()].sort((a, b) => a.title.localeCompare(b.title));
    return [...dirs, ...files];
  }

  private async renderDetail(row: ListingRow): Promise<void> {
    this.detail.replaceChildren();
    if (this.state.openResource !== row.resourceId) return;
    const facts: ResourceFacts = {
      resourceId: row.resourceId,
      title: row.title,
      mime: row.mime,
      sizeBytes: row.sizeBytes,
      revision: row.revision,
    };
    const kind = classify(facts);
    const head = document.createElement("div");
    head.className = "detail-head";
    const title = document.createElement("h3");
    title.textContent = row.title;
    const headMeta = document.createElement("p");
    headMeta.className = "muted";
    headMeta.textContent = `${row.mime} · ${formatBytes(row.sizeBytes)}${row.revision ? ` · revision ${row.revision}` : ""}`;
    const close = document.createElement("button");
    close.type = "button";
    close.className = "secondary";
    close.textContent = "关闭";
    close.addEventListener("click", () => { this.closeResource(); });
    head.append(title, headMeta, close);

    const url = this.handlers.contentUrl(row.resourceId ? this.state.endpointId : "", row.resourceId, row.revision);
    const download = document.createElement("a");
    download.className = "download-link";
    download.href = url;
    download.setAttribute("download", row.title);
    download.textContent = `下载原文件（${formatBytes(row.sizeBytes)}）`;

    const body = document.createElement("div");
    body.className = "detail-body";
    this.detail.append(head, body, download);

    if (row.sizeBytes !== undefined && row.sizeBytes > PREVIEW_BYTE_LIMIT) {
      body.append(this.notice("内容超出预览上限，仅提供下载。"));
      return;
    }
    if (kind === "image") {
      if (rendersInline(kind, row.mime)) {
        const image = document.createElement("img");
        image.className = "preview-image";
        image.src = url;
        image.alt = row.title;
        image.loading = "lazy";
        body.append(image);
      } else {
        body.append(this.notice("SVG 不在同源内联渲染（可能携带脚本与外链），请下载查看。"));
      }
      return;
    }
    if (kind === "video") {
      const video = document.createElement("video");
      video.className = "preview-video";
      video.controls = true;
      video.preload = "metadata";
      video.src = url;
      // Media is a live stream: a directory refresh must never swap the
      // element out from under playback.
      video.setAttribute("data-resource", row.resourceId);
      body.append(video);
      return;
    }
    if (kind === "text" || kind === "markdown") {
      try {
        const bytes = await this.handlers.fetchRange(url);
        const decoded = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
        const clamped = clampText(decoded);
        const pre = document.createElement("pre");
        pre.className = "preview-text";
        pre.tabIndex = 0;
        pre.textContent = clamped.text;
        body.append(pre);
        if (clamped.truncated) body.append(this.notice("文本已截断显示，请下载查看完整内容。"));
      } catch (error) {
        body.append(this.notice(this.failureText(error)));
      }
      return;
    }
    if (kind === "docx" || kind === "zip") {
      try {
        const bytes = await this.handlers.fetchRange(url);
        if (kind === "zip") {
          const listing = await listArchive(bytes);
          const table = document.createElement("ul");
          table.className = "archive-list";
          for (const entry of listing.entries) {
            const item = document.createElement("li");
            item.textContent = `${entry.name} · ${formatBytes(entry.size)}（压缩后 ${formatBytes(entry.compressedSize)}）`;
            if (entry.rejected) item.classList.add("rejected");
            table.append(item);
          }
          body.append(table);
          if (listing.rejected > 0) {
            body.append(this.notice(`${listing.rejected} 个条目因穿越、超限或压缩比过高被拒绝。`));
          }
          return;
        }
        const preview = await previewDocx(bytes);
        if (preview.html) {
          const frame = document.createElement("div");
          frame.className = "docx-preview";
          // Sanitized by `stripDangerousMarkup`: no scripts, no external
          // references survive insertion.
          frame.innerHTML = preview.html;
          body.append(frame);
        } else {
          body.append(this.notice("无法解析该文档，请下载原文件。"));
        }
      } catch (error) {
        body.append(this.notice(`${this.failureText(error)}（${errorMessage(error)}）`));
      }
      return;
    }
    body.append(this.notice("该类型不支持预览。"));
    const meta = document.createElement("dl");
    meta.className = "binary-meta";
    for (const [label, value] of [
      ["名称", row.title],
      ["类型", row.mime],
      ["大小", formatBytes(row.sizeBytes)],
      ["revision", row.revision ?? "未提供"],
    ]) {
      const term = document.createElement("dt");
      term.textContent = label;
      const definition = document.createElement("dd");
      definition.textContent = value;
      meta.append(term, definition);
    }
    body.append(meta);
  }

  private notice(message: string): HTMLElement {
    const node = document.createElement("p");
    node.className = "muted notice";
    node.textContent = message;
    return node;
  }

  private failureText(error: unknown): string {
    return FAILURE_TEXT[failureKind(errorCode(error), errorMessage(error))];
  }
}

export function connectionLabel(state: number | undefined): string {
  return CONNECTION_LABEL[state ?? 0] ?? "未知";
}
