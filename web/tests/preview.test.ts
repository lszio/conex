//! M5 preview safety and classification. Hostile documents must never
//! escape the sandbox, and archives must never expand past their budget.

import { describe, expect, test } from "bun:test";
import { zipSync, strToU8 } from "fflate";

import { listArchive, previewDocx, ratioIsSafe } from "../src/archives";
import {
  ARCHIVE_ENTRY_LIMIT,
  classify,
  clampText,
  failureKind,
  formatBytes,
  rendersInline,
  reviewArchiveEntries,
  stripDangerousMarkup,
  type ResourceFacts,
} from "../src/preview";

function facts(overrides: Partial<ResourceFacts> = {}): ResourceFacts {
  return {
    resourceId: "notes/hello.md",
    title: "hello.md",
    mime: "text/markdown",
    ...overrides,
  };
}

describe("classification", () => {
  test("uses the Host MIME first, extension as fallback", () => {
    expect(classify(facts())).toBe("markdown");
    expect(classify(facts({ resourceId: "a/b.txt", mime: "text/plain" }))).toBe("text");
    expect(classify(facts({ resourceId: "a/clip.mp4", mime: "video/mp4" }))).toBe("video");
    expect(classify(facts({ resourceId: "a/pic.png", mime: "image/png" }))).toBe("image");
    expect(classify(facts({ resourceId: "a/doc.docx", mime: "application/octet-stream" }))).toBe("docx");
    expect(classify(facts({ resourceId: "a/x.zip", mime: "application/zip" }))).toBe("zip");
  });

  test("unknown types are binary, never guessed as previewable", () => {
    expect(classify(facts({ resourceId: "a/blob", mime: "application/octet-stream" }))).toBe("binary");
  });

  test("svg is not rendered inline on the Host origin", () => {
    const kind = classify(facts({ resourceId: "a/x.svg", mime: "image/svg+xml" }));
    expect(kind).toBe("image");
    expect(rendersInline(kind, "image/svg+xml")).toBe(false);
    expect(rendersInline("image", "image/png")).toBe(true);
  });
});

describe("markup scrubbing", () => {
  test("scripts and handlers never survive", () => {
    const hostile =
      '<p>ok</p><script>fetch("https://evil.example")</script>' +
      '<img src="x" onerror="alert(1)">' +
      '<a href="javascript:alert(1)">click</a>';
    const scrubbed = stripDangerousMarkup(hostile);
    expect(scrubbed).not.toContain("<script");
    expect(scrubbed).not.toContain("onerror");
    expect(scrubbed).not.toContain("javascript:");
    expect(scrubbed).toContain("ok");
  });

  test("external references are neutralized", () => {
    const scrubbed = stripDangerousMarkup(
      '<a href="https://evil.example/x">x</a><img src="https://tracker.example/p.gif">',
    );
    expect(scrubbed).not.toContain("evil.example");
    expect(scrubbed).not.toContain("tracker.example");
  });

  test("iframes, objects and stylesheets are removed", () => {
    const scrubbed = stripDangerousMarkup(
      '<iframe src="https://evil.example"></iframe><object data="x"></object><style>body{}</style>',
    );
    expect(scrubbed).not.toContain("iframe");
    expect(scrubbed).not.toContain("object");
    expect(scrubbed).not.toContain("<style");
  });
});

describe("archive safety", () => {
  test("traversal, absolute and control-character names are refused", () => {
    const review = reviewArchiveEntries([
      { name: "ok.txt", compressedSize: 10, size: 20 },
      { name: "../escape.txt", compressedSize: 1, size: 1 },
      { name: "/etc/passwd", compressedSize: 1, size: 1 },
      { name: "C:\\win.txt", compressedSize: 1, size: 1 },
      { name: "a\u0000b.txt", compressedSize: 1, size: 1 },
    ]);
    expect(review.visible.map((entry) => entry.name)).toEqual(["ok.txt"]);
    expect(review.rejected).toBe(4);
  });

  test("zip bombs are refused, not expanded", () => {
    // A 40 MiB member with a 4 KiB compressed size is a classic ratio bomb.
    const review = reviewArchiveEntries([
      { name: "big.bin", compressedSize: 4096, size: 40 * 1024 * 1024 },
    ]);
    expect(review.visible[0]?.rejected).toBe("ratio");
    expect(review.totalSize).toBe(0);
    expect(ratioIsSafe(4096, 40 * 1024 * 1024)).toBe(false);
  });

  test("entry count is bounded", () => {
    const many = Array.from({ length: ARCHIVE_ENTRY_LIMIT + 25 }, (_, index) => ({
      name: `f${index}.txt`,
      compressedSize: 4,
      size: 4,
    }));
    const review = reviewArchiveEntries(many);
    expect(review.visible.length).toBeLessThanOrEqual(ARCHIVE_ENTRY_LIMIT);
  });

  test("a real archive lists its entries with sizes", async () => {
    const archive = zipSync({
      "a/one.txt": strToU8("hello"),
      "a/two.md": strToU8("# two"),
    });
    const listing = await listArchive(archive);
    expect(listing.entries.length).toBe(2);
    expect(listing.entries.map((entry) => entry.name).sort()).toEqual(["a/one.txt", "a/two.md"]);
    expect(listing.rejected).toBe(0);
  });
});

describe("docx preview", () => {
  test("a real docx renders sanitized html", async () => {
    // Minimal OOXML package: [Content_Types].xml + document.xml.
    const docx = zipSync({
      "[Content_Types].xml": strToU8(
        '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
          '<Default Extension="xml" ContentType="application/xml"/>' +
          '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>' +
          "</Types>",
      ),
      "_rels/.rels": strToU8(
        '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
          '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>' +
          "</Relationships>",
      ),
      "word/document.xml": strToU8(
        '<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">' +
          "<w:body><w:p><w:r><w:t>Hello Notez</w:t></w:r></w:p></w:body></w:document>",
      ),
    });
    const preview = await previewDocx(docx);
    expect(preview.html).toContain("Hello Notez");
    expect(preview.html).not.toContain("<script");
  });
});

describe("presentation helpers", () => {
  test("text is clamped to the preview budget", () => {
    const long = "x".repeat(600 * 1024);
    const clamped = clampText(long);
    expect(clamped.truncated).toBe(true);
    expect(clamped.text.length).toBeLessThan(long.length);
  });

  test("byte sizes are human readable", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KiB");
    expect(formatBytes(undefined)).toBe("大小未知");
  });

  test("failures map to distinct user-facing classes", () => {
    expect(failureKind(-32001, "")).toBe("permission");
    expect(failureKind(403, "")).toBe("permission");
    expect(failureKind(400, "resource not found")).toBe("missing");
    expect(failureKind(-32005, "remote agent is offline")).toBe("offline");
    expect(failureKind(-32013, "")).toBe("stale");
    expect(failureKind(-32004, "")).toBe("unsupported");
    expect(failureKind(-32007, "")).toBe("too-large");
    expect(failureKind(0, "weird")).toBe("unknown");
  });
});
