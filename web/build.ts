export {};
import { rm } from "node:fs/promises";
import { join } from "node:path";

// Bun's bundler handles JSX and JS natively. Tailwind runs as its own CLI
// because Bun's CSS pipeline rejects Tailwind v4's at-rules (@theme,
// @import "tailwindcss"); the CLI is the supported entry point for those.
const root = import.meta.dir;
const outdir = `${root}/dist`;
// Debug escape hatch: `bun run build:web -- --no-minify` keeps readable names
// in the shipped bundle when a production stack trace has to be read.
const minify = !process.argv.includes("--no-minify");

await rm(outdir, { recursive: true, force: true });

const result = await Bun.build({
  entrypoints: [`${root}/src/main.tsx`],
  outdir,
  target: "browser",
  format: "esm",
  // Minified: the page is one blocking bundle on the critical path, and
  // measured from the public host it was 827 KB taking 5-7 s to transfer
  // (the session + ticket round trip is 22 ms by comparison). Minified it is
  // 548 KB, and gzipped on the wire ~168 KB. `bun run build:web -- --no-minify`
  // keeps the readable bundle for production incident debugging.
  minify,
  sourcemap: "none",
  naming: { entry: "app.js" },
  define: { "process.env.NODE_ENV": '"production"' },
  plugins: [
    {
      // shadcn components import via `@/…`; resolve it against src/ without
      // adding a bundler dependency. The extension is appended because the
      // specifiers are written without one, and only the bundler (not the
      // editor) needs to know which file it is.
      name: "alias",
      setup(build) {
        build.onResolve({ filter: /^@\// }, async (args) => {
          const base = join(root, "src", args.path.slice(2));
          for (const candidate of [base, `${base}.tsx`, `${base}.ts`, join(base, "index.tsx"), join(base, "index.ts")]) {
            if (await Bun.file(candidate).exists()) return { path: candidate };
          }
          return { path: base, external: true };
        });
      },
    },
  ],
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

const tailwind = Bun.spawnSync({
  cmd: [
    join(root, "..", "node_modules", ".bin", "tailwindcss"),
    "-i",
    `${root}/src/index.css`,
    "-o",
    `${outdir}/style.css`,
    "--minify",
  ],
  cwd: root,
});
if (tailwind.exitCode !== 0) {
  console.error(tailwind.stderr.toString());
  process.exit(1);
}

// The page shell. Written by hand: the header and section order are part of
// the content contract, not a build artifact.
await Bun.write(`${outdir}/index.html`, Bun.file(`${root}/index.html`));

// llms.txt is served by the Host at /llms.txt and /llm.txt. It is part of the
// page's contract, so a missing file must fail the build rather than silently
// serving a 404 to crawlers.
const llms = Bun.file(`${root}/llms.txt`);
if (!(await llms.exists())) {
  console.error("web/llms.txt is missing");
  process.exit(1);
}
await Bun.write(`${outdir}/llms.txt`, llms);

// The bench report the performance page renders is imported into the bundle
// from src/perf-data.json, so it ships inside app.js rather than as a separate
// file the Host would have to be taught to serve. Its presence is still
// checked: a missing report means the performance page would render nothing.
if (!(await Bun.file(`${root}/src/perf-data.json`).exists())) {
  console.error("web/src/perf-data.json is missing; run `cargo xtask bench --suite hello`");
  process.exit(1);
}
