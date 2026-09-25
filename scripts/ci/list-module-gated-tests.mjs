#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const sourceRoot = path.join(root, "crates", "openhuman-core", "src");
const run = [];
const skipped = [];

function visit(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) visit(file);
    else if (entry.name.endsWith(".rs")) scan(file);
  }
}

function scan(file) {
  const lines = fs.readFileSync(file, "utf8").split("\n");
  let pending = null;
  for (const line of lines) {
    if (line.includes('#[ignore = "needs a built tinymemory module')) pending = "run";
    else if (line.includes('#[ignore = "needs a built tinydocs')) pending = "tinydocs";

    const match = line.match(/^\s*(?:async\s+)?fn\s+([A-Za-z0-9_]+)/);
    if (!pending || !match) continue;
    (pending === "run" ? run : skipped).push(match[1]);
    pending = null;
  }
}

visit(sourceRoot);
const result = { run: run.sort(), skipped: skipped.sort() };

if (process.argv.includes("--json")) console.log(JSON.stringify(result));
else if (process.argv.includes("--skipped")) console.log(result.skipped.join("\n"));
else console.log(result.run.join("\n"));

if (result.run.length === 0) {
  console.error("module-gated test source inventory is empty");
  process.exit(1);
}
