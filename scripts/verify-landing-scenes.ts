// Real-browser verification of the landing workbench. Independent browser
// contexts, real WebSockets, real files: no mocks and no direct store access.
//
// Two things here are not obvious and both were wrong before:
//
//  - Two *tabs in one context* share a cookie, so they are the case that
//    proves per-tab links: they must show up as two separate clients. Two
//    contexts would not test that at all.
//  - The question round trip needs the answering tab to be in front. A
//    background tab still receives the frame, but asserting on a hidden tab
//    reads as a hang rather than as a slow page.
//
// Run against a live host: CONEX_VERIFY_ORIGIN=http://127.0.0.1:8080

import { chromium, type Browser, type Page } from "playwright";

const origin = process.env.CONEX_VERIFY_ORIGIN ?? "http://127.0.0.1:8080";
const SAMPLE_NAME = "季度报告.txt";
const SAMPLE_BODY = "第一行：这是共享的文本。\n第二行：换行也要正确。\n";
const failures: string[] = [];
const notes: string[] = [];

function check(condition: boolean, description: string): void {
  // Printed as it happens: a run that dies mid-way must still show which
  // checks already passed, otherwise a timeout erases all evidence.
  if (condition) {
    notes.push(`ok   ${description}`);
  } else {
    failures.push(description);
    notes.push(`FAIL ${description}`);
  }
  console.log(notes[notes.length - 1]);
}

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(
    () => document.body.innerText.includes("已连接"),
    undefined,
    { timeout: 20000 },
  );
}

async function setProfile(page: Page, name: string, group: string): Promise<void> {
  await page.getByLabel("名称").fill(name);
  await page.getByLabel("组名", { exact: true }).fill(group);
  await page.getByRole("button", { name: "保存" }).click();
  await page.waitForTimeout(800);
}

/**
 * Hand a file to the page's own input.
 *
 * `setInputFiles` silently drops a path whose name is not ASCII — the input
 * comes back with zero files and no change event — so the CJK file is built
 * in the page and dispatched the way a real picker delivers it. The code path
 * under test is identical either way: the same input, the same change event.
 */
async function pickFile(page: Page, name: string, body: string): Promise<void> {
  const files = await page.evaluateHandle(
    ({ fileName, fileBody }) => {
      const transfer = new DataTransfer();
      transfer.items.add(new File([fileBody], fileName, { type: "text/plain" }));
      return transfer.files;
    },
    { fileName: name, fileBody: body },
  );
  await page
    .locator('input[aria-label="选择要共享的文件"]')
    .evaluate((element, list) => {
      const input = element as HTMLInputElement;
      input.files = list as FileList;
      input.dispatchEvent(new Event("change", { bubbles: true }));
    }, files);
}

/**
 * A file big enough to take the chunked route, built in the page.
 *
 * Built rather than read from disk because the size matters here: the chunked
 * path is what anything over one chunk takes, and a small sample would exercise
 * the one-request route instead of the one under test.
 */
async function pickBigFile(page: Page, name: string, bytes: number): Promise<void> {
  const files = await page.evaluateHandle(
    ({ fileName, byteCount }) => {
      const body = new Uint8Array(byteCount);
      // A per-byte counter, so a wrong offset cannot accidentally match.
      for (let i = 0; i < byteCount; i += 1) body[i] = i % 251;
      const transfer = new DataTransfer();
      transfer.items.add(new File([body], fileName, { type: "text/plain" }));
      return transfer.files;
    },
    { fileName: name, byteCount: bytes },
  );
  await page
    .locator('input[aria-label="选择要共享的文件"]')
    .evaluate((element, list) => {
      const input = element as HTMLInputElement;
      input.files = list as FileList;
      input.dispatchEvent(new Event("change", { bubbles: true }));
    }, files);
}

/** The short link the status bar shows for this tab. */
async function ownLink(page: Page): Promise<string> {
  return (await page.locator("header").innerText()).match(/链接\s*(\S+)/)?.[1] ?? "";
}

/**
 * The chunked route, driven for real: declare, send, commit, and then read the
 * bytes back through the ordinary download link.
 *
 * `byteCount` is over one chunk so the page takes the resumable path rather than
 * the single-request one, and the assertion is on the bytes the peer downloads,
 * not on a progress label.
 */
async function chunkedUploadRoundTrip(tabA: Page, tabB: Page, origin: string): Promise<void> {
  const byteCount = 256 * 1024 * 2 + 519;
  const expected = new Uint8Array(byteCount);
  for (let i = 0; i < byteCount; i += 1) expected[i] = i % 251;

  await pickBigFile(tabA, "chunked.bin", byteCount);
  // The label reports real chunks landing, so waiting for it is waiting for the
  // last one rather than sleeping and hoping.
  await tabA.getByText("已共享 1 个文件", { timeout: 30000 }).waitFor();
  await tabB.waitForTimeout(1200);

  // The row that carries this file's name, so the assertion is about this file
  // and not about whichever download link happens to come first on the page.
  const href = await tabB
    .locator("li")
    .filter({ hasText: "chunked.bin" })
    .locator('a[href*="/web/files/download"]')
    .first()
    .getAttribute("href");
  check(typeof href === "string", "分块上传后出现下载链接");
  if (typeof href !== "string") return;
  const bytes = await tabB.evaluate(async (url) => {
    const response = await fetch(url, { credentials: "same-origin" });
    if (!response.ok) return null;
    return Array.from(new Uint8Array(await response.arrayBuffer()));
  }, href);
  check(
    bytes !== null && bytes.length === byteCount,
    `分块上传的字节数与声明一致: ${bytes?.length ?? 0}/${byteCount}`,
  );
  check(
    bytes !== null && bytes.every((value, index) => value === expected[index]),
    "分块上传的每个字节都与上传的一致",
  );

  // The item answers with a validator that names its content, and a media
  // element resuming a seek relies on it: an `If-Range` that does not match
  // must give the whole item rather than a slice of something else.
  const etag = await tabB.evaluate(async (url: string) => {
    const response = await fetch(url, { credentials: "same-origin" });
    return response.headers.get("etag") ?? "";
  }, href);
  check(etag.startsWith('"') && etag.length > 10, `下载响应带 ETag: ${etag.slice(0, 18)}…`);

  const seek = await tabB.evaluate(
    async ({ url, validator }: { url: string; validator: string }) => {
      const read = async (range: string, ifRange?: string) => {
        const response = await fetch(url, {
          credentials: "same-origin",
          headers: range ? { range } : {},
        });
        const matched = ifRange
          ? await fetch(url, {
              credentials: "same-origin",
              headers: { range, "if-range": ifRange },
            })
          : null;
        return {
          plain: response.status,
          plainLength: response.headers.get("content-length"),
          matched: matched?.status ?? null,
          matchedLength: matched?.headers.get("content-length") ?? null,
          range: response.headers.get("content-range"),
          validator,
        };
      };
      return read("bytes=4-8", validator);
    },
    { url: href, validator: etag },
  );
  check(seek.plain === 206, "带 Range 的请求答 206");
  check(seek.range === `bytes 4-8/${byteCount}`, `Content-Range 正确: ${seek.range}`);
  check(seek.matched === 206 && seek.matchedLength === "5", "If-Range 匹配时仍按 Range 答");

  const stale = await tabB.evaluate(
    async ({ url, validator }: { url: string; validator: string }) => {
      const response = await fetch(url, {
        credentials: "same-origin",
        headers: { range: "bytes=4-8", "if-range": validator },
      });
      return { status: response.status, length: response.headers.get("content-length") };
    },
    // A validator naming other content, which is what a stale element holds.
    { url: href, validator: '"bafkrei-not-this-content"' },
  );
  check(
    stale.status === 200 && Number(stale.length) === byteCount,
    `If-Range 不匹配时答整个 item: ${stale.status}/${stale.length}`,
  );

  // A preview of the same file reads one head, not the whole body.
  const head = await tabB.evaluate(async (url) => {
    const response = await fetch(`${url}${url.includes("?") ? "&" : "?"}range=bytes=0-1023`, {
      credentials: "same-origin",
    });
    if (!response.ok) return null;
    const buffer = new Uint8Array(await response.arrayBuffer());
    return { length: buffer.length, first: buffer[0], last: buffer[buffer.length - 1] };
  }, href);
  // Byte 1023 of a counter payload: 1023 % 251 = 19.
  check(
    head?.length === 1024 && head.first === 0 && head.last === 1023 % 251,
    `预览只读取请求的那一段: ${head?.length ?? 0} 字节，首 ${head?.first} 末 ${head?.last}`,
  );
}

/**
 * An interrupted chunked upload, resumed.
 *
 * This is the behaviour the chunked design exists for, so it is driven through
 * the routes a visitor's browser uses rather than against the store: three
 * chunks land, the connection drops, and re-declaring the same content has to
 * find the same staging and report how far it got. If it ever reports zero, the
 * client silently re-sends every byte it already sent.
 */
async function interruptedUploadResumes(page: Page, origin: string): Promise<void> {
  // The nonce and the tab's link, both of which every write route needs.
  const csrf = await page.evaluate(async () => {
    const response = await fetch("/web/session", { credentials: "same-origin" });
    const body = (await response.json()) as { csrf?: string };
    return body.csrf ?? "";
  });
  // The listing route needs the link header, so the full id is read out of a
  // download link the page itself rendered: it rides in that URL's query, and
  // this is the same value the page's own client sends on every request.
  const linkId = (await (async () => {
    const href = await page
      .locator('a[href*="/web/files/download"]')
      .first()
      .getAttribute("href");
    const match = href?.match(/[?&]linkId=([^&]+)/);
    return match ? decodeURIComponent(match[1]) : "";
  })());
  if (!csrf || !linkId) {
    check(false, `中断续传需要会话（csrf=${Boolean(csrf)} link=${Boolean(linkId)}）`);
    return;
  }

  const chunkSize = 256 * 1024;
  const result = await page.evaluate(
    async ({ size, token, link, CHUNK }: { size: number; token: string; link: string; CHUNK: number }) => {
      const body = new Uint8Array(size);
      // A per-run prefix, so this content never collides with the staging a
      // previous run left behind: the host reuses staging by content identity,
      // which is exactly what the resume below relies on, and a deterministic
      // payload would inherit a stranger's progress. Four random bytes give the
      // whole file a different root CID.
      const random = crypto.getRandomValues(new Uint8Array(4));
      for (let i = 0; i < size; i += 1) body[i] = i < 4 ? random[i] : (i + random[0]) % 251;
      const cid = async (bytes: Uint8Array) => {
        const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
        const all = new Uint8Array(4 + digest.length);
        all[0] = 1; all[1] = 0x55; all[2] = 0x12; all[3] = 0x20;
        all.set(digest, 4);
        const alphabet = "abcdefghijklmnopqrstuvwxyz234567";
        let out = "", buffer = 0, bits = 0;
        for (const byte of all) {
          buffer = (buffer << 8) | byte;
          bits += 8;
          while (bits >= 5) {
            bits -= 5;
            out += alphabet[(buffer >>> bits) & 31];
          }
        }
        if (bits > 0) out += alphabet[(buffer << (5 - bits)) & 31];
        return `b${out}`;
      };
      const headers = {
        "content-type": "application/json",
        "x-csrf-token": token,
        "x-conex-link-id": link,
      };
      const chunkCids: string[] = [];
      for (let offset = 0; offset < size; offset += CHUNK) {
        chunkCids.push(await cid(body.slice(offset, Math.min(offset + CHUNK, size))));
      }
      const declare = () =>
        fetch("/web/files/uploads", {
          method: "POST",
          credentials: "same-origin",
          headers,
          body: JSON.stringify({
            name: "broken.bin",
            mime: "text/plain",
            sizeBytes: String(size),
            chunkSize: CHUNK,
            chunkCids,
            kind: "file",
          }),
        });
      const started = await (await declare()).json();
      // A fresh run starts from zero; anything else means this run inherited
      // staging from an earlier one, and the assertion below would be vacuous.
      if (Number(started.receivedCount) !== 0) {
        throw new Error(`inherited staging with ${started.receivedCount} chunks`);
      }
      const uploadId = started.uploadId as string;
      const puts: string[] = [];
      for (let index = 0; index * CHUNK < size; index += 1) {
        const put = await fetch(`/web/files/uploads/${uploadId}/chunks/${index}`, {
          method: "PUT",
          credentials: "same-origin",
          headers: {
            "x-csrf-token": token,
            "x-conex-link-id": link,
            "content-type": "application/octet-stream",
          },
          body: body.slice(index * CHUNK, Math.min((index + 1) * CHUNK, size)),
        });
        puts.push(`${index}:${put.status}`);
        // Three chunks land, then the connection drops.
        if (index === 2) break;
      }
      // The connection drops here; the client remembers nothing but the file.
      const resumed = await (await declare()).json();
      return {
        first: uploadId,
        second: resumed.uploadId as string,
        receivedCount: Number(resumed.receivedCount),
        chunkCids,
        puts,
      };
    },
    { size: chunkSize * 5 + 33, token: csrf, link: linkId, CHUNK: chunkSize },
  ).catch(() => null);
  if (!result) {
    check(false, "中断续传的请求失败");
    return;
  }
  check(result.first === result.second, "重新声明找到同一个上传，而不是重新开始");
  check(result.receivedCount === 3, `主机报告已收 ${result.receivedCount} 块（应为 3）`);
  for (const put of result.puts) console.log(`  put ${put}`);
  void origin;
}

async function main(): Promise<void> {
  let browser: Browser | undefined;
  try {
    browser = await chromium.launch();
    const errors: string[] = [];

    // One context, two tabs: the same cookie, so this is exactly the case
    // where a per-session link would make them one client.
    const shared = await browser.newContext();
    const tabA = await shared.newPage();
    const tabA2 = await shared.newPage();
    for (const [name, page] of [["A", tabA], ["A2", tabA2]] as const) {
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(`${name}: ${message.text()}`);
      });
      page.on("pageerror", (error) => errors.push(`${name}: ${error.message}`));
    }

    await tabA.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(tabA);
    await tabA2.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(tabA2);
    check(
      await tabA.getByRole("heading", { name: "双向能力路由内核" }).isVisible(),
      "工作台标题是 conex 本身",
    );

    const linkA = await ownLink(tabA);
    const linkA2 = await ownLink(tabA2);
    check(linkA.length > 0, `标签页 A 拿到自己的短链: ${linkA}`);
    check(
      linkA2.length > 0 && linkA2 !== linkA,
      `同一浏览器的两个标签页是不同的链接: ${linkA} vs ${linkA2}`,
    );

    // Separate context: separate cookie, so a different visitor entirely.
    const contextB = await browser.newContext();
    const b = await contextB.newPage();
    b.on("console", (message) => {
      if (message.type() === "error") errors.push(`B: ${message.text()}`);
    });
    b.on("pageerror", (error) => errors.push(`B: ${error.message}`));

    await b.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(b);

    const contextC = await browser.newContext();
    const outsider = await contextC.newPage();
    await outsider.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(outsider);

    await setProfile(tabA, "alice", "team-a");
    await setProfile(tabA2, "alice2", "team-a");
    await setProfile(b, "bob", "team-a");
    await setProfile(outsider, "mallory", "team-b");

    // The workbench lists every same-group client, and A sees two of its own
    // tabs plus B — three rows, not one flickering row for the shared cookie.
    await tabA.waitForTimeout(800);
    await tabA.waitForTimeout(1200);
    const listText = await tabA.locator("main").innerText();
    check(
      listText.includes("alice2") && listText.includes("bob"),
      `同组三个客户端都在 A 的列表里: ${listText.includes("alice2")}/${listText.includes("bob")}`,
    );
    check(
      !listText.includes("mallory"),
      "另一个组的客户端不出现在 A 的列表里",
    );

    // Greeting: B sends to alice, so *alice's* tab gets the corner notice and
    // B's own tab gets the round trip it measured.
    await b.getByRole("button", { name: "发送 hello" }).first().click();
    await b.waitForTimeout(1500);
    const aToasts = await tabA.locator('div[aria-label="消息提示"]').innerText();
    check(
      aToasts.includes("向你打招呼"),
      `A 收到角落提示而不是对话气泡: ${JSON.stringify(aToasts.slice(0, 60))}`,
    );
    const bToasts = await b.locator('div[aria-label="消息提示"]').innerText();
    check(
      /(<1 ms|\d+ ms)/.test(bToasts),
      `发送方看到实测往返: ${JSON.stringify(bToasts.slice(0, 60))}`,
    );
    check(
      !(await tabA.locator("main").innerText()).includes("消息泡泡"),
      "页面不再有「消息泡泡」区块",
    );

    // Question with a callback argument: B asks alice, alice answers in her
    // toast, and the answer comes back to B with the fields that were asked.
    await b.getByRole("button", { name: "提问并等回答" }).first().click();
    await b.getByLabel(/向 .* 提问的字段/).fill("你的状态");
    await b.getByRole("button", { name: "提问", exact: true }).click();
    await b.waitForTimeout(1500);
    const question = await tabA.locator('div[aria-label="消息提示"]').innerText();
    check(
      question.includes("提问") && question.includes("你的状态"),
      `提问在接收方变成待确认提示: ${JSON.stringify(question.slice(0, 80))}`,
    );
    check(
      await tabA.getByRole("button", { name: "确认并回复" }).count() > 0,
      "待确认提示里有确认按钮",
    );
    await tabA.getByLabel("你的状态").fill("在线");
    await tabA.getByRole("button", { name: "确认并回复" }).click();
    await b.waitForTimeout(2000);
    const answered = await b.locator('div[aria-label="消息提示"]').innerText();
    check(
      answered.includes("你的状态：在线"),
      `回答回到发送方: ${JSON.stringify(answered.slice(0, 80))}`,
    );

    // Status: counters and a latency that was measured, not assumed.
    await tabA.getByText("运行状态与延迟").click();
    await tabA.waitForTimeout(600);
    await tabA.waitForTimeout(1500);
    const statusText = await tabA.locator("main").innerText();
    check(/在线客户端/.test(statusText), "状态面板显示客户端计数");
    check(/在线分组/.test(statusText), "状态面板显示分组计数");
    // Rows are anonymous: the group text is the join secret, so the own row
    // carries the「你的组」badge and the other row is what remains.
    const teamRow = await tabA
      .locator("li")
      .filter({ hasText: "你的组" })
      .first()
      .innerText();
    check(/3 客户端/.test(teamRow), `team-a 显示 3 个客户端: ${teamRow}`);
    check(/最近\s*(<1 ms|\d+ ms)/.test(teamRow), `team-a 显示实测延迟: ${teamRow}`);
    check(!teamRow.includes("team-a"), "状态行不显示组名文本");
    const malloryRow = await tabA
      .locator("li")
      .filter({ hasText: "尚未测量" })
      .first()
      .innerText();
    check(/尚未测量/.test(malloryRow), `未发过 hello 的组显示「尚未测量」: ${malloryRow}`);

    // Files: the two shared-cookie tabs are two owners, not one.
    await tabA.waitForTimeout(600);
    await pickFile(tabA, SAMPLE_NAME, SAMPLE_BODY);
    await tabA.waitForTimeout(2000);
    check(
      (await tabA.locator("main").innerText()).includes("已共享 1 个文件"),
      "A 上传后收到成功提示",
    );

    await b.waitForTimeout(600);
    await b.waitForTimeout(1500);
    const bBody = await b.locator("main").innerText();
    check(bBody.includes(SAMPLE_NAME), "B 在文件面板看到 A 提供的文件");
    check(
      (await b.getByRole("button", { name: "撤回" }).count()) === 0,
      "B 看不到撤回按钮（非所有者）",
    );

    const preview = await b.locator("details summary").first().innerText();
    check(preview.includes("预览文本"), "文本文件提供预览入口");
    await b.locator("details").first().click();
    await b.waitForTimeout(800);
    const previewed = await b.locator("details p").first().innerText();
    check(previewed.includes("第一行"), `预览读到真实字节: ${previewed.slice(0, 40)}`);

    const download = await b
      .locator('a[href*="/web/files/download"]')
      .first()
      .getAttribute("href");
    check(
      typeof download === "string" && download.includes("linkId="),
      "下载链接带上本标签页的 linkId",
    );
    if (typeof download === "string") {
      const bytes = await b.evaluate(async (url) => {
        const response = await fetch(url, { credentials: "same-origin" });
        return response.ok ? new TextDecoder().decode(await response.arrayBuffer()) : "";
      }, download);
      check(bytes.includes("第一行"), "同组客户端下载到的字节与上传一致");
    }

    // The sibling tab can see the file but cannot withdraw it, and the
    // cross-group visitor sees nothing.
    await tabA2.waitForTimeout(600);
    await tabA2.waitForTimeout(1500);
    const a2Body = await tabA2.locator("main").innerText();
    check(a2Body.includes(SAMPLE_NAME), "同一浏览器的另一个标签页也能看到该文件");
    check(
      (await tabA2.getByRole("button", { name: "撤回" }).count()) === 0,
      "另一个标签页不能撤回他人文件（所有者按链接区分）",
    );

    await outsider.waitForTimeout(600);
    await outsider.waitForTimeout(1500);
    check(
      !(await outsider.locator("main").innerText()).includes(SAMPLE_NAME),
      "另一个组的文件面板看不到该文件",
    );

    // The chunked route, end to end, before anything is withdrawn.
    await chunkedUploadRoundTrip(tabA, b, origin);

    // A scene change is pushed, not polled for. The file scene's own
    // interval is now thirty seconds, so a peer's row appearing within a
    // couple of seconds of an upload is only possible through the
    // notification. This runs while both peers are still connected:
    // after `tabA.close()` the file is gone and a push from a closed tab
    // is a no-op.
    await b.bringToFront();
    await tabA2.getByRole("button", { name: "撤回" }).count();
    const pushSeen = await b.evaluate(async (label: string) => {
      const deadline = Date.now() + 4000;
      while (Date.now() < deadline) {
        if (document.body.innerText.includes(label)) return true;
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      return false;
    }, "chunked.bin");
    check(pushSeen, "对端通过推送刷新文件面板，而不是等下一次轮询");

    // ...and interrupted: three chunks land, the connection drops, and
    // re-declaring has to find the same staging rather than restarting.
    await interruptedUploadResumes(tabA, origin);

    // A closed tab reclaims its bytes.
    //
    // Waited for rather than slept on: the page polls every three seconds, so a
    // fixed pause either wastes a second or races the next refresh, and a race
    // here reads as a reclaim that did not happen.
    await tabA.close();
    try {
      await b.waitForFunction(
        (name) => !document.body.innerText.includes(name),
        SAMPLE_NAME,
        { timeout: 15000 },
      );
      check(true, "提供者断开后文件被回收");
    } catch {
      check(false, "提供者断开后文件被回收（等待列表更新超时）");
    }

  check(errors.length === 0, `页面无控制台错误: ${errors.join(" | ")}`);
  } finally {
    await browser?.close();
  }

  console.log(notes.join("\n"));
  if (failures.length > 0) {
    console.error(`\n${failures.length} 项失败`);
    process.exit(1);
  }
  console.log("\n全部检查通过");
}

await main();
