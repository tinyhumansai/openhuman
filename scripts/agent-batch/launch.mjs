#!/usr/bin/env node
// Print one launch comment per agent in the batch.
// The operator pastes each into Cursor as the agent prompt.
// Usage:
//   node scripts/agent-batch/launch.mjs <spec.json> [--agent <id>] [--print-only]
//
// `--print-only` is the default and the only mode currently supported; the
// flag exists so callers can be explicit and so a future `--post` mode (e.g.
// posting directly to a Cursor API) is unambiguous.

import {
  loadSpec,
  validateSpec,
  findOverlaps,
  SpecError,
  parseArgs,
} from "./lib.mjs";

function renderComment(spec, agent) {
  const owned = agent.owned_paths.map((p) => `  - \`${p}\``).join("\n");
  const shared = (agent.allowed_shared_paths ?? [])
    .map((p) => `  - \`${p}\``)
    .join("\n");
  return `@Cursor work issue #${agent.issue} for batch \`${spec.batch_id}\`.

Repo: ${spec.base_repo}
Base: latest \`${spec.base_branch}\` from upstream
Branch: \`${agent.branch}\`
PR target: ${spec.base_repo}:${spec.base_branch} (from your fork)

Owned paths (you MAY edit these and nothing else outside the shared-paths list):
${owned}

${shared ? `Allowed shared paths (touch only if strictly necessary):\n${shared}\n` : ""}
Required reading before editing:
  - docs/agent-workflows/cursor-cloud-agents.md
  - docs/agent-workflows/codex-pr-checklist.md
  - .github/PULL_REQUEST_TEMPLATE.md
  - AGENTS.md and CLAUDE.md

Pre-PR validation (run all, report any blocker exactly in the PR body):
  - pnpm --filter openhuman-app format:check
  - pnpm typecheck
  - pnpm lint
  - focused vitest for changed TS/React files
  - cargo fmt --manifest-path Cargo.toml --all --check (if Rust changed)
  - focused \`pnpm debug rust <filter>\` for changed Rust
  - pnpm test:coverage and pnpm test:rust — coverage on changed lines must be ≥ 80%

PR rules:
  - Title: \`<area>: ${agent.title} (#${agent.issue})\`
  - Body MUST follow .github/PULL_REQUEST_TEMPLATE.md verbatim, including the AI Authored PR Metadata section.
  - Add labels: ${(agent.labels ?? ["cursor-agent"]).map((l) => `\`${l}\``).join(", ")}
  - One PR per issue. Do not open duplicates. If retrying, update the existing PR.
  - Push to your fork; open the PR with \`--head <fork-owner>:${agent.branch}\` against ${spec.base_repo}:${spec.base_branch}.
  - Close the issue with \`Closes #${agent.issue}\` in the Related section.

Tracking: progress for this batch is reported on issue #${spec.tracking_issue}.
`;
}

function main() {
  const { positional, flags } = parseArgs(process.argv.slice(2));
  const specPath = positional[0];
  if (!specPath) {
    process.stderr.write(
      "usage: launch.mjs <spec.json> [--agent <id>] [--print-only]\n",
    );
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
  if (collisions.length > 0) {
    process.stderr.write(
      `[agent-batch] refusing to launch: ${collisions.length} ownership collision(s) — run overlap.mjs\n`,
    );
    process.exit(1);
  }

  const onlyId = typeof flags.agent === "string" ? flags.agent : null;
  const agents = onlyId
    ? spec.agents.filter((a) => a.id === onlyId)
    : spec.agents;
  if (onlyId && agents.length === 0) {
    process.stderr.write(
      `[agent-batch] no agent with id "${onlyId}" in spec\n`,
    );
    process.exit(1);
  }

  for (const agent of agents) {
    process.stdout.write(`\n===== agent ${agent.id} (#${agent.issue}) =====\n`);
    process.stdout.write(renderComment(spec, agent));
  }
  process.stdout.write("\n");
}

main();
