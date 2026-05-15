#!/usr/bin/env node
// Render a markdown progress table for an agent batch.
// Usage:
//   node scripts/agent-batch/status.mjs <spec.json>          # query gh, print to stdout
//   node scripts/agent-batch/status.mjs <spec.json> --post   # also rewrite the tracking comment
//   node scripts/agent-batch/status.mjs <spec.json> --fixture <file.json>
//       (for tests — read PR data from a JSON fixture instead of shelling out to gh)
//
// Fixture shape (array of records keyed by branch):
//   [{ "headRefName": "cursor/a01-…", "number": 1234, "url": "…",
//      "state": "OPEN" | "MERGED" | "CLOSED",
//      "statusCheckRollup": "SUCCESS" | "FAILURE" | "PENDING" | null }]

import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

import { loadSpec, validateSpec, SpecError, parseArgs } from "./lib.mjs";

const COMMENT_MARKER = (id) => `<!-- batch:${id} -->`;

function ciCell(rollup) {
  if (rollup === "SUCCESS") return "green";
  if (rollup === "FAILURE") return "failing";
  if (rollup === "PENDING") return "pending";
  return "—";
}

function stateCell(state) {
  if (!state) return "no PR";
  return state.toLowerCase();
}

function renderTable(spec, prsByBranch) {
  const lines = [];
  lines.push(COMMENT_MARKER(spec.batch_id));
  lines.push(`## Batch \`${spec.batch_id}\` — progress`);
  lines.push("");
  lines.push("| Agent | Issue | Branch | PR | CI | Status |");
  lines.push("| --- | --- | --- | --- | --- | --- |");
  for (const agent of spec.agents) {
    const pr = prsByBranch.get(agent.branch) ?? null;
    const prCell = pr ? `[#${pr.number}](${pr.url})` : "—";
    const ci = pr ? ciCell(pr.statusCheckRollup) : "—";
    const state = stateCell(pr?.state);
    lines.push(
      `| ${agent.id} | [#${agent.issue}](https://github.com/${spec.base_repo}/issues/${agent.issue}) | \`${agent.branch}\` | ${prCell} | ${ci} | ${state} |`,
    );
  }
  return lines.join("\n") + "\n";
}

function fetchPrsFromGh(spec) {
  // One `gh pr list` call per batch — cheap and avoids N+1.
  const args = [
    "pr",
    "list",
    "--repo",
    spec.base_repo,
    "--state",
    "all",
    "--search",
    `label:batch:${spec.batch_id}`,
    "--json",
    "headRefName,number,url,state,statusCheckRollup",
    "--limit",
    "100",
  ];
  const r = spawnSync("gh", args, { encoding: "utf8" });
  if (r.status !== 0) {
    throw new Error(
      `gh failed (${r.status}): ${r.stderr?.trim() || "unknown error"}`,
    );
  }
  return JSON.parse(r.stdout || "[]");
}

function indexByBranch(prs) {
  const m = new Map();
  for (const pr of prs) {
    // `statusCheckRollup` from gh is an array of contexts when populated; we
    // only care about the worst-status rollup for the cell.
    let rollup = null;
    if (
      Array.isArray(pr.statusCheckRollup) &&
      pr.statusCheckRollup.length > 0
    ) {
      const states = pr.statusCheckRollup
        .map((c) => c.conclusion || c.state)
        .filter(Boolean);
      if (
        states.some(
          (s) => s === "FAILURE" || s === "CANCELLED" || s === "TIMED_OUT",
        )
      ) {
        rollup = "FAILURE";
      } else if (states.every((s) => s === "SUCCESS")) {
        rollup = "SUCCESS";
      } else {
        rollup = "PENDING";
      }
    } else if (typeof pr.statusCheckRollup === "string") {
      // Fixture-friendly shape.
      rollup = pr.statusCheckRollup;
    }
    m.set(pr.headRefName, {
      number: pr.number,
      url: pr.url,
      state: pr.state,
      statusCheckRollup: rollup,
    });
  }
  return m;
}

function postOrUpdateTrackingComment(spec, body) {
  const issue = spec.tracking_issue;
  const list = spawnSync(
    "gh",
    [
      "api",
      `repos/${spec.base_repo}/issues/${issue}/comments`,
      "--paginate",
      "--jq",
      // Emit one object per line. `gh api --paginate` runs the jq filter
      // per-page; wrapping in `[...]` would produce concatenated array
      // fragments that aren't valid JSON. NDJSON sidesteps that.
      `.[] | select(.body | contains("${COMMENT_MARKER(spec.batch_id)}")) | {id, html_url}`,
    ],
    { encoding: "utf8" },
  );
  if (list.status !== 0) {
    throw new Error(
      `gh api failed (${list.status}): ${list.stderr?.trim() || "unknown error"}`,
    );
  }
  const existing = (list.stdout || "")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line));
  if (existing.length === 0) {
    const r = spawnSync(
      "gh",
      [
        "issue",
        "comment",
        String(issue),
        "--repo",
        spec.base_repo,
        "--body-file",
        "-",
      ],
      { encoding: "utf8", input: body },
    );
    if (r.status !== 0) {
      throw new Error(`gh issue comment failed: ${r.stderr?.trim() || ""}`);
    }
    process.stdout.write(
      `[agent-batch] posted new tracking comment on #${issue}\n`,
    );
  } else {
    const id = existing[0].id;
    // Pass the comment body via stdin (-F body=@-) rather than a command-line
    // arg. Long markdown tables can grow large and -f body=${body} risks
    // hitting OS argv length limits (ARG_MAX).
    const r = spawnSync(
      "gh",
      [
        "api",
        "--method",
        "PATCH",
        `repos/${spec.base_repo}/issues/comments/${id}`,
        "-F",
        "body=@-",
      ],
      { encoding: "utf8", input: body },
    );
    if (r.status !== 0) {
      throw new Error(`gh api PATCH failed: ${r.stderr?.trim() || ""}`);
    }
    process.stdout.write(
      `[agent-batch] updated tracking comment ${existing[0].html_url}\n`,
    );
  }
}

function main() {
  const { positional, flags } = parseArgs(process.argv.slice(2));
  const specPath = positional[0];
  if (!specPath) {
    process.stderr.write(
      "usage: status.mjs <spec.json> [--post] [--fixture <file>]\n",
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

  let prs;
  if (typeof flags.fixture === "string") {
    prs = JSON.parse(readFileSync(flags.fixture, "utf8"));
  } else {
    prs = fetchPrsFromGh(spec);
  }
  const body = renderTable(spec, indexByBranch(prs));
  process.stdout.write(body);

  if (flags.post) {
    postOrUpdateTrackingComment(spec, body);
  }
}

main();
