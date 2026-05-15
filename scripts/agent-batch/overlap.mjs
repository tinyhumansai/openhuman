#!/usr/bin/env node
// Prove ownership disjointness in a batch spec.
// Usage: node scripts/agent-batch/overlap.mjs <spec.json>
// Exits 0 if all agents own disjoint path prefixes, 1 on any collision.

import {
  loadSpec,
  validateSpec,
  findOverlaps,
  SpecError,
  parseArgs,
} from "./lib.mjs";

function main() {
  const { positional } = parseArgs(process.argv.slice(2));
  const specPath = positional[0];
  if (!specPath) {
    process.stderr.write("usage: overlap.mjs <spec.json>\n");
    process.exit(2);
  }
  let spec;
  try {
    spec = validateSpec(loadSpec(specPath));
  } catch (e) {
    if (e instanceof SpecError) {
      process.stderr.write(`[agent-batch] spec error: ${e.message}\n`);
      process.exit(1);
    }
    throw e;
  }
  const collisions = findOverlaps(spec);
  if (collisions.length === 0) {
    process.stdout.write(
      `[agent-batch] ok: ${spec.agents.length} agent(s) own disjoint paths\n`,
    );
    process.exit(0);
  }
  process.stderr.write(
    `[agent-batch] ${collisions.length} ownership collision(s):\n`,
  );
  for (const c of collisions) {
    process.stderr.write(
      `  ${c.a} ↔ ${c.b}: "${c.pathA}" vs "${c.pathB}" (${c.reason})\n`,
    );
  }
  process.exit(1);
}

main();
