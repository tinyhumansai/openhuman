#!/usr/bin/env node
// Thin dispatcher so `pnpm agent-batch <verb> <spec> [...]` works.
// Verbs map to sibling .mjs scripts.

import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERBS = new Set(["validate", "overlap", "launch", "status"]);

function usage(code = 2) {
  process.stderr.write(
    "usage: pnpm agent-batch <validate|overlap|launch|status> <spec.json> [...]\n",
  );
  process.exit(code);
}

const [verb, ...rest] = process.argv.slice(2);
if (!verb || verb === "--help" || verb === "-h") usage(verb ? 0 : 2);
if (!VERBS.has(verb)) {
  process.stderr.write(`[agent-batch] unknown verb "${verb}"\n`);
  usage();
}

const here = dirname(fileURLToPath(import.meta.url));
const child = spawn(process.execPath, [join(here, `${verb}.mjs`), ...rest], {
  stdio: "inherit",
});
child.on("exit", (code) => process.exit(code ?? 1));
