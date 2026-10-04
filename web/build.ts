export {};
import { unlink } from "node:fs/promises";
const root = import.meta.dir;
const result = await Bun.build({
  entrypoints: [`${root}/src/main.ts`],
  outdir: `${root}/dist`,
  target: "browser",
  minify: false,
  sourcemap: "none",
  naming: { entry: "app.js" },
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
await unlink(`${root}/dist/main.js`).catch(() => undefined);
await Bun.write(`${root}/dist/index.html`, Bun.file(`${root}/index.html`));
await Bun.write(`${root}/dist/style.css`, Bun.file(`${root}/src/style.css`));
// Served by the Host at /llms.txt (and /llm.txt); the file is part of the
// page's contract, so a missing one must fail the build rather than
// silently serving a 404 to crawlers.
const llms = Bun.file(`${root}/llms.txt`);
if (!(await llms.exists())) {
  console.error("web/llms.txt is missing; run from the web directory");
  process.exit(1);
}
await Bun.write(`${root}/dist/llms.txt`, llms);
