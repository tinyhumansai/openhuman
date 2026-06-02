#!/usr/bin/env node
// Validate a Cursor Cloud Agents batch spec.
// Usage: node scripts/agent-batch/validate.mjs <spec.json>
// Exits 0 on success, 1 on any policy violation. Prints a one-line summary
// on success and a structured error otherwise.

import { loadSpec, validateSpec, SpecError, parseArgs } from "./lib.mjs";

function usage() {
  return "usage: validate.mjs <spec.json>";
}

function main() {
  const { positional, flags } = parseArgs(process.argv.slice(2));
  if (flags.help || flags.h || flags["?"]) {
    process.stdout.write(`${usage()}\n`);
    process.exit(0);
  }
  const specPath = positional[0];
  if (!specPath) {
    process.stderr.write(`${usage()}\n`);
    process.exit(2);
  }
  try {
    const spec = validateSpec(loadSpec(specPath));
    process.stdout.write(
      `[agent-batch] ok: batch ${spec.batch_id} with ${spec.agents.length} agent(s)\n`,
    );
    process.exit(0);
  } catch (e) {
    if (e instanceof SpecError) {
      process.stderr.write(`[agent-batch] spec error: ${e.message}\n`);
      process.exit(1);
    }
    process.stderr.write(
      `[agent-batch] unexpected error: ${e.stack || e.message}\n`,
    );
    process.exit(2);
  }
}

main();
