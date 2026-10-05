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

async function openPanel(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name, exact: true }).click();
  await page.waitForTimeout(400);
}

async function setProfile(page: Page, name: string, group: string): Promise<void> {
  await openPanel(page, "消息");
  await page.getByLabel("名称").fill(name);
  await page.getByLabel("组").fill(group);
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

/** The short link the status bar shows for this tab. */
async function ownLink(page: Page): Promise<string> {
  return (await page.locator("header").innerText()).match(/链接\s*(\S+)/)?.[1] ?? "";
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
    await openPanel(tabA, "消息");
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
    await openPanel(tabA, "状态");
    await tabA.waitForTimeout(1500);
    const statusText = await tabA.locator("main").innerText();
    check(/在线客户端/.test(statusText), "状态面板显示客户端计数");
    check(/在线分组/.test(statusText), "状态面板显示分组计数");
    const teamRow = await tabA
      .locator("li")
      .filter({ hasText: "team-a" })
      .first()
      .innerText();
    check(/3 客户端/.test(teamRow), `team-a 显示 3 个客户端: ${teamRow}`);
    check(/最近\s*(<1 ms|\d+ ms)/.test(teamRow), `team-a 显示实测延迟: ${teamRow}`);
    const malloryRow = await tabA
      .locator("li")
      .filter({ hasText: "team-b" })
      .first()
      .innerText();
    check(/尚未测量/.test(malloryRow), `未发过 hello 的组显示「尚未测量」: ${malloryRow}`);

    // Files: the two shared-cookie tabs are two owners, not one.
    await openPanel(tabA, "文件");
    await pickFile(tabA, SAMPLE_NAME, SAMPLE_BODY);
    await tabA.waitForTimeout(2000);
    check(
      (await tabA.locator("main").innerText()).includes("已共享 1 个文件"),
      "A 上传后收到成功提示",
    );

    await openPanel(b, "文件");
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
    await openPanel(tabA2, "文件");
    await tabA2.waitForTimeout(1500);
    const a2Body = await tabA2.locator("main").innerText();
    check(a2Body.includes(SAMPLE_NAME), "同一浏览器的另一个标签页也能看到该文件");
    check(
      (await tabA2.getByRole("button", { name: "撤回" }).count()) === 0,
      "另一个标签页不能撤回他人文件（所有者按链接区分）",
    );

    await openPanel(outsider, "文件");
    await outsider.waitForTimeout(1500);
    check(
      !(await outsider.locator("main").innerText()).includes(SAMPLE_NAME),
      "另一个组的文件面板看不到该文件",
    );

    // A closed tab reclaims its bytes.
    await tabA.close();
    await b.waitForTimeout(2500);
    const afterClose = await b.locator("main").innerText();
    check(
      afterClose.includes("还没有人共享文件") || !afterClose.includes(SAMPLE_NAME),
      "提供者断开后文件被回收",
    );

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
