#!/usr/bin/env -S bun
// Dev helper: build the page, start a guest-only host on a free port, and print
// the URL. Keeps the loop for UI work to two commands.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dir, "..");

const build = Bun.spawnSync({ cmd: ["bun", "run", "build:web"], cwd: root });
if (build.exitCode !== 0) process.exit(build.exitCode);

const dir = mkdtempSync(join(tmpdir(), "conex-dev-"));
for (const name of ["content", "session", "operation"]) {
  Bun.spawnSync({ cmd: ["mkdir", "-p", join(dir, name)] });
}
const port = 8080;
writeFileSync(
  join(dir, "host.toml"),
  `listen = "127.0.0.1:${port}"
allow_loopback_http = true
audience = "conex.dev"
host_origin = "conex://dev"
web_origin = "http://127.0.0.1:${port}"
web_root = "${join(root, "web/dist")}"
content_root = "${join(dir, "content")}"
session_root = "${join(dir, "session")}"
operation_root = "${join(dir, "operation")}"

[web_guest]
principal_id = "guest"
tenant_id = "demo"
max_sessions = 256
max_issue_per_minute = 100000
`,
);

console.log(`dev host: http://127.0.0.1:${port}`);
const child = spawn(join(root, "target/debug/conex-host"), [join(dir, "host.toml")], {
  stdio: "inherit",
});
process.on("SIGINT", () => child.kill());
