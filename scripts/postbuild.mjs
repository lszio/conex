// postbuild: add shebang to CLI entry so it works as a node bin
import { readFileSync, writeFileSync, chmodSync, existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");

const cliEntry = join(root, "dist", "cli", "index.js");

if (existsSync(cliEntry)) {
  const content = readFileSync(cliEntry, "utf-8");
  if (!content.startsWith("#!/usr/bin/env node")) {
    writeFileSync(cliEntry, "#!/usr/bin/env node\n" + content);
    chmodSync(cliEntry, 0o755);
    console.log("[postbuild] shebang added to " + cliEntry);
  } else {
    console.log("[postbuild] " + cliEntry + " already has shebang");
  }
} else {
  console.warn("[postbuild] " + cliEntry + " not found -- skipping");
}