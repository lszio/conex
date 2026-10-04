// Real-browser verification of the landing scenes. Two independent tabs, two
// real sessions, real WebSockets: no mocks and no direct store access.
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
  await page.getByRole("button", { name: "hello 场景" }).click();
  await page.getByLabel("名称").fill(name);
  await page.getByLabel("组").fill(group);
  await page.getByRole("button", { name: "保存" }).click();
  await page.waitForTimeout(700);
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

async function main(): Promise<void> {
  let browser: Browser | undefined;
  try {
    browser = await chromium.launch();
    const contextA = await browser.newContext();
    const contextB = await browser.newContext();
    const a = await contextA.newPage();
    const b = await contextB.newPage();
    const errors: string[] = [];
    for (const [name, page] of [["A", a], ["B", b]] as const) {
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(`${name}: ${message.text()}`);
      });
      page.on("pageerror", (error) => errors.push(`${name}: ${error.message}`));
    }

    await a.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(a);
    check(await a.getByRole("heading", { name: "双向能力路由内核" }).isVisible(), "页面定位是 conex 本身");
    // The default tab is the hello scene, so the positioning is on its own tab.
    await a.getByRole("button", { name: "介绍" }).click();
    const about = await a.locator("body").innerText();
    check(
      about.includes("双向能力路由内核") && about.includes("反向连接") && about.includes("provider"),
      "介绍页说明 conex 的定位与连接方式",
    );

    await b.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(b);

    // Same group => peers are visible; the tab that never joined sees nobody.
    await setProfile(a, "alice", "team-a");
    await setProfile(b, "bob", "team-a");

    const c = await browser.newContext();
    const outsider = await c.newPage();
    await outsider.goto(`${origin}/`, { waitUntil: "domcontentloaded" });
    await ready(outsider);
    await setProfile(outsider, "mallory", "team-b");
    await outsider.getByRole("button", { name: "状态" }).click();
    await outsider.waitForTimeout(1500);
    const outsiderGroups = await outsider.locator("li").allInnerTexts();
    check(
      outsiderGroups.some((text) => text.includes("你的组")) &&
        !outsiderGroups.some((text) => text.includes("alice")),
      "另一个组看不到本组客户端",
    );

    // hello: A -> B produces a bubble on both sides with a real latency.
    await a.getByRole("button", { name: "hello 场景" }).click();
    await a.getByRole("button", { name: "发送 hello" }).first().click();
    await a.waitForTimeout(1200);
    const aBubbles = await a.locator("ul[aria-live] li").allInnerTexts();
    check(
      aBubbles.some((text) => text.includes("我 →") && /ms/.test(text)),
      `A 的消息泡泡带实测往返延迟: ${JSON.stringify(aBubbles)}`,
    );
    const bBubbles = await b.locator("ul[aria-live] li").allInnerTexts();
    check(
      bBubbles.some((text) => text.includes("alice →") && text.includes("hello")),
      `B 收到 A 的 hello 泡泡: ${JSON.stringify(bBubbles)}`,
    );

    // Status: counts, groups, and a latency that is measured rather than zero.
    await a.getByRole("button", { name: "状态" }).click();
    await a.waitForTimeout(1500);
    const statusText = await a.locator("main, body").first().innerText();
    check(/在线客户端/.test(statusText), "状态页显示客户端计数");
    check(/在线分组/.test(statusText), "状态页显示分组计数");
    const teamRow = await a
      .locator("li")
      .filter({ hasText: "team-a" })
      .first()
      .innerText();
    check(/2 客户端/.test(teamRow), `team-a 显示 2 个客户端: ${teamRow}`);
    check(/最近\s*(<1 ms|\d+ ms)/.test(teamRow), `team-a 显示实测延迟: ${teamRow}`);
    check(!/尚未测量/.test(teamRow), "本组完成过 hello 后不再显示「尚未测量」");
    const malloryRow = await a
      .locator("li")
      .filter({ hasText: "team-b" })
      .first()
      .innerText();
    check(/尚未测量/.test(malloryRow), `未发过 hello 的组显示「尚未测量」: ${malloryRow}`);

    // File scene: A shares, B sees a card, both can read the bytes.
    await a.getByRole("button", { name: "文件场景" }).click();
    await pickFile(a, SAMPLE_NAME, SAMPLE_BODY);
    await a.waitForTimeout(2000);
    check(
      (await a.locator("body").innerText()).includes("已共享 1 个文件"),
      "A 上传后收到成功提示",
    );

    await b.getByRole("button", { name: "文件场景" }).click();
    await b.waitForTimeout(1500);
    const bBody = await b.locator("body").innerText();
    check(bBody.includes(SAMPLE_NAME), "B 在文件场景看到 A 提供的文件");
    check(bBody.includes("alice（你）") === false, "文件卡片标出所有者而不是当前用户");
    check(bBody.includes("alice"), "文件按提供者分卡片显示所有者");
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
      typeof download === "string" && download.includes("/web/files/download"),
      "下载链接指向同源共享路由",
    );
    if (typeof download === "string") {
      const bytes = await b.evaluate(async (url) => {
        const response = await fetch(url, { credentials: "same-origin" });
        return response.ok ? new TextDecoder().decode(await response.arrayBuffer()) : "";
      }, download);
      check(bytes.includes("第一行"), "同组客户端下载到的字节与上传一致");
    }

    // Cross-group isolation on the file scene.
    await outsider.getByRole("button", { name: "文件场景" }).click();
    await outsider.waitForTimeout(1500);
    check(
      !(await outsider.locator("body").innerText()).includes(SAMPLE_NAME),
      "另一个组的文件场景看不到该文件",
    );

    // A closed tab reclaims its bytes.
    await a.close();
    await b.waitForTimeout(2500);
    const afterClose = await b.locator("body").innerText();
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

async function downloadHref(page: Page): Promise<string | null> {
  return page
    .locator('a[href*="/web/files/download"]')
    .first()
    .getAttribute("href")
    .catch(() => null);
}

await main();
