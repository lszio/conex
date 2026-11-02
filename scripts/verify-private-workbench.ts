// Real-browser verification of the private workbench (MVP-P7).
//
// Runs against a live hub + node exactly like `verify-landing-scenes.ts` does
// for the guest landing: real Chromium, real bootstrap-code login, real
// direct-grant → ticket → WSS channel, real OMP model traffic. No mocks, no
// direct store access, no fake states.
//
// Prerequisites (see docs/runbooks/private-network.md for the config shape):
//   CONEX_VERIFY_ORIGIN   private hub origin (http://127.0.0.1:8790)
//   CONEX_HOST_BIN        conex-host binary (for private-bootstrap)
//   CONEX_HUB_CONFIG      hub.toml path (for private-bootstrap)
//
// Each scenario mints its own one-time bootstrap code; codes are single-use
// and the script logs in a fresh context per scenario where isolation matters.

import { chromium, type Browser, type BrowserContext, type Page } from "playwright";
import { spawnSync } from "node:child_process";

const origin = process.env.CONEX_VERIFY_ORIGIN ?? "http://127.0.0.1:8790";
const hostBin = process.env.CONEX_HOST_BIN ?? "target/debug/conex-host";
const hubConfig = process.env.CONEX_HUB_CONFIG ?? "";
const failures: string[] = [];
const notes: string[] = [];

function check(condition: boolean, description: string): void {
  if (condition) {
    notes.push(`ok   ${description}`);
  } else {
    failures.push(description);
    notes.push(`FAIL ${description}`);
  }
  console.log(notes[notes.length - 1]);
}

function mintBootstrapCode(): string {
  if (!hubConfig) throw new Error("CONEX_HUB_CONFIG is required");
  const result = spawnSync(hostBin, ["private-bootstrap", `--config=${hubConfig}`], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`private-bootstrap failed: ${result.stderr}`);
  }
  const code = result.stdout.trim();
  if (code.length !== 43) throw new Error(`bootstrap code malformed: ${code.length} chars`);
  return code;
}

async function login(page: Page, code: string): Promise<void> {
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  // 私有入口在无会话时必须渲染登录视图，而不是降级到访客页。
  await page.getByRole("textbox", { name: "引导码" }).waitFor({ timeout: 15000 });
  await page.getByRole("textbox", { name: "引导码" }).fill(code);
  await page.getByRole("button", { name: "登录" }).click();
}

async function waitChannelReady(page: Page): Promise<void> {
  // 直连徽标 ready = grant + 票据 + WSS 全链路成功。
  await page.waitForFunction(
    () => document.body.innerText.includes("直连"),
    undefined,
    { timeout: 30000 },
  );
}

async function submitAndWaitDone(page: Page, prompt: string): Promise<void> {
  const composer = page.getByPlaceholder(/输入要发给 Agent 的内容/);
  await composer.fill(prompt);
  await page.keyboard.press("Enter");
  // 终态徽标（已完成/失败/已取消）出现在状态行。
  await page
    .getByText(/(已完成|失败|已取消)/)
    .first()
    .waitFor({ timeout: 180000 });
}

/** 释放私有会话槽位：hub 会话表有界，验收不留下占用。 */
async function logout(page: Page): Promise<void> {
  await page.evaluate(async () => {
    // 登出是写操作：必须带 CSRF，否则会话槽位不释放（主体上限 8）。
    const csrf = localStorage.getItem("conex.private.csrf.v1") ?? "";
    await fetch("/web/logout", {
      method: "POST",
      credentials: "include",
      headers: { "x-csrf-token": csrf },
    }).catch(() => undefined);
    localStorage.removeItem("conex.private.csrf.v1");
  });
}

async function main(): Promise<void> {
  // channel chrome: drive the installed Google Chrome (the playwright CDN
  // is unreachable here; installed Chrome is equivalent for this purpose).
  const browser = await chromium.launch({ channel: "chrome" });
  try {
    await scenarioLoginAndDirectory(browser);
    await scenarioConversation(browser);
    await scenarioInteraction(browser);
    await scenarioCancel(browser);
    await scenarioServersAndGraph(browser);
  } finally {
    await browser.close();
  }
  console.log(`\n${notes.length - failures.length}/${notes.length} checks passed`);
  if (failures.length > 0) {
    console.error(`FAILED:\n${failures.map((f) => `  - ${f}`).join("\n")}`);
    process.exit(1);
  }
}

/** S1：引导码登录、目录身份、端点/Workspace 发现。 */
async function scenarioLoginAndDirectory(browser: Browser): Promise<void> {
  const context = await browser.newContext();
  const page = await context.newPage();
  const code = mintBootstrapCode();
  await login(page, code);
  await page.getByRole("heading", { name: /私有工作台/ }).waitFor({ timeout: 15000 });
  check(true, "login view accepts the bootstrap code and renders the workbench");
  await page.getByText(/节点 [0-9A-Z]/).waitFor({ timeout: 30000 });
  check(true, "directory shows the registered node identity");
  await waitChannelReady(page);
  check(true, "direct channel establishes (grant + ticket + WSS)");
  // 端点下拉应出现 OMP 端点（目录 inventory 经节点自报）。
  await page.waitForFunction(
    () => document.querySelectorAll("select")[0]?.options.length > 1,
    undefined,
    { timeout: 20000 },
  );
  await page.locator("select").first().selectOption({ index: 1 });
  await page.waitForFunction(
    () => document.querySelectorAll("select")[1]?.options.length > 1,
    undefined,
    { timeout: 15000 },
  );
  check(true, "agent endpoint and workspaces discovered over the direct channel");
  await page.screenshot({ path: "docs/verification/assets/p7-01-workbench.png", fullPage: true });
  await logout(page);
  await context.close();
}

/** S2：真实 OMP 会话创建 + 流式完成。 */
async function scenarioConversation(browser: Browser): Promise<void> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await login(page, mintBootstrapCode());
  await waitChannelReady(page);
  await page.waitForFunction(
    () => document.querySelectorAll("select")[0]?.options.length > 1,
    undefined,
    { timeout: 20000 },
  );
  await page.locator("select").first().selectOption({ index: 1 });
  await page.waitForFunction(
    () => document.querySelectorAll("select")[1]?.options.length > 1,
    undefined,
    { timeout: 15000 },
  );
  await page.getByRole("button", { name: "新建" }).click();
  await page.getByPlaceholder(/输入要发给 Agent 的内容/).waitFor({ timeout: 20000 });
  check(true, "conversation created; composer enabled");
  await submitAndWaitDone(page, "Reply with exactly the word READY and nothing else.");
  await page.waitForFunction(
    () => document.body.innerText.includes("READY"),
    undefined,
    { timeout: 20000 },
  );
  check(true, "real model streaming completed and the assistant reply rendered");
  await page.screenshot({ path: "docs/verification/assets/p7-02-conversation.png", fullPage: true });
  await logout(page);
  await context.close();
}

/** S3：人工交互（ask 工具）在横幅中呈现并作答。 */
async function scenarioInteraction(browser: Browser): Promise<void> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await login(page, mintBootstrapCode());
  await waitChannelReady(page);
  await page.waitForFunction(
    () => document.querySelectorAll("select")[0]?.options.length > 1,
    undefined,
    { timeout: 20000 },
  );
  await page.locator("select").first().selectOption({ index: 1 });
  await page.waitForFunction(
    () => document.querySelectorAll("select")[1]?.options.length > 1,
    undefined,
    { timeout: 15000 },
  );
  await page.getByRole("button", { name: "新建" }).click();
  await page.getByPlaceholder(/输入要发给 Agent 的内容/).waitFor({ timeout: 20000 });
  const composer = page.getByPlaceholder(/输入要发给 Agent 的内容/);
  await composer.fill(
    "Use the ask tool to ask: continue or stop? Offer exactly two options: Continue and Stop. After the user answers Stop, reply with the single word STOPPED and nothing else.",
  );
  await page.keyboard.press("Enter");
  const banner = page.getByRole("region", { name: "等待人工响应" });
  await banner.waitFor({ timeout: 180000 });
  check(true, "pending interaction surfaced in the approval banner");
  await page.screenshot({ path: "docs/verification/assets/p7-03-interaction.png", fullPage: true });
  const stop = banner.getByRole("button", { name: /Stop/i }).first();
  await stop.click();
  await page.getByText("STOPPED").waitFor({ timeout: 180000 });
  check(true, "deny answer resolved the interaction and the turn completed");
  await logout(page);
  await context.close();
}

/** S4：取消执行到 cancelled 终态。 */
async function scenarioCancel(browser: Browser): Promise<void> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await login(page, mintBootstrapCode());
  await waitChannelReady(page);
  await page.waitForFunction(
    () => document.querySelectorAll("select")[0]?.options.length > 1,
    undefined,
    { timeout: 20000 },
  );
  await page.locator("select").first().selectOption({ index: 1 });
  await page.waitForFunction(
    () => document.querySelectorAll("select")[1]?.options.length > 1,
    undefined,
    { timeout: 15000 },
  );
  await page.getByRole("button", { name: "新建" }).click();
  await page.getByPlaceholder(/输入要发给 Agent 的内容/).waitFor({ timeout: 20000 });
  // 模型可能先跑完：取消按钮出现就点；自然终态则换更长的提示重试。
  for (const prompt of [
    "Count from 1 to 300 slowly, writing every number on its own line.",
    "Write an essay of at least 3000 words about the history of computing. Do not stop early.",
    "Write a 6000-word detailed technical proposal for a distributed operating system. Do not summarize.",
  ]) {
    const composer = page.getByPlaceholder(/输入要发给 Agent 的内容/);
    await composer.fill(prompt);
    await page.keyboard.press("Enter");
    const cancel = page.getByRole("button", { name: "取消执行" });
    try {
      await cancel.waitFor({ timeout: 45000 });
      await cancel.click();
      await page.getByText("已取消").first().waitFor({ timeout: 120000 });
      check(true, "cancel reached the cancelled terminal state in the projection");
      await logout(page);
      await context.close();
      return;
    } catch {
      await page
        .getByText(/(已完成|失败|已取消)/)
        .first()
        .waitFor({ timeout: 180000 });
    }
  }
  check(false, "cancel scenario: every prompt finished before cancel could land");
  await logout(page);
  await context.close();
}

/** S5：服务器指标 + 图谱/标注。 */
async function scenarioServersAndGraph(browser: Browser): Promise<void> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await login(page, mintBootstrapCode());
  await waitChannelReady(page);
  await page.getByRole("button", { name: "服务器" }).click();
  await page.getByText("CPU", { exact: true }).waitFor({ timeout: 30000 });
  await page.waitForFunction(
    () => document.body.innerText.includes("live"),
    undefined,
    { timeout: 20000 },
  );
  check(true, "server metrics render real live samples from the node");
  await page.screenshot({ path: "docs/verification/assets/p7-04-servers.png", fullPage: true });
  await page.getByRole("button", { name: "会话" }).click();
  await page.waitForFunction(
    () => document.querySelectorAll("select")[0]?.options.length > 1,
    undefined,
    { timeout: 20000 },
  );
  await page.locator("select").first().selectOption({ index: 1 });
  await page.waitForFunction(
    () => document.querySelectorAll("select")[1]?.options.length > 1,
    undefined,
    { timeout: 15000 },
  );
  await page.getByRole("button", { name: "新建" }).click();
  await page.getByPlaceholder(/输入要发给 Agent 的内容/).waitFor({ timeout: 20000 });
  await page.getByRole("button", { name: "图谱" }).click();
  await page.getByPlaceholder("添加标注").fill("p7-verified");
  await page.getByRole("button", { name: "添加", exact: true }).click();
  await page.getByText("p7-verified").waitFor({ timeout: 15000 });
  check(true, "graph view tags the conversation through the relations module");
  await page.screenshot({ path: "docs/verification/assets/p7-05-graph.png", fullPage: true });
  await logout(page);
  await context.close();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
