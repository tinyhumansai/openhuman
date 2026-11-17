#!/usr/bin/env node
// Compare CI Lite against the lane flow (CI Fast on EX63, CI Fast (hosted)) on
// the same commits, to measure what the self-hosted runners buy.
//
// Usage:
//   node scripts/ci/self-hosted/compare-runs.mjs [--repo owner/name] [--limit N] [--lanes]
//
// Pairs runs by head SHA. Wall clock is run creation to last update, so queue
// time counts: that is what a contributor waits for. With --lanes it also
// downloads each fast run's ci-timings.json and prints per-lane durations.
// Needs an authenticated `gh`.
import { execFileSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const WORKFLOWS = {
  lite: "ci-lite.yml",
  ex63: "ci-fast.yml",
  hosted: "ci-fast-hosted.yml",
};

function gh(args) {
  // stderr is dropped: a workflow not yet on the default branch answers 404,
  // which simply means "no runs yet".
  return execFileSync("gh", args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "ignore"],
  });
}

/** Minutes between two ISO timestamps, one decimal. */
export function minutesBetween(a, b) {
  if (!a || !b) return null;
  return Math.round((Date.parse(b) - Date.parse(a)) / 6000) / 10;
}

/**
 * Pair runs by head SHA. Keeps the newest completed run per workflow per SHA
 * and only SHAs that ran CI Lite and at least one lane-flow workflow.
 */
export function pairRuns(runsByKind) {
  const bySha = new Map();
  for (const [kind, runs] of Object.entries(runsByKind)) {
    for (const r of runs) {
      if (r.status !== "completed") continue;
      const row = bySha.get(r.headSha) ?? { sha: r.headSha };
      if (
        !row[kind] ||
        Date.parse(r.createdAt) > Date.parse(row[kind].createdAt)
      )
        row[kind] = r;
      bySha.set(r.headSha, row);
    }
  }
  return [...bySha.values()]
    .filter((row) => row.lite && (row.ex63 || row.hosted))
    .sort(
      (a, b) => Date.parse(b.lite.createdAt) - Date.parse(a.lite.createdAt),
    );
}

/** Per-lane wall-clock minutes from a ci-timings.json object. */
export function laneMinutes(timings) {
  const out = {};
  for (const lane of timings.lanes ?? []) {
    const starts = lane.checks
      .map((c) => c.start)
      .filter(Boolean)
      .sort();
    const ends = lane.checks
      .map((c) => c.end)
      .filter(Boolean)
      .sort();
    if (starts.length)
      out[lane.name] = minutesBetween(starts[0], ends[ends.length - 1]);
  }
  return out;
}

function listRuns(repo, workflow, limit) {
  try {
    return JSON.parse(
      gh([
        "run",
        "list",
        "-R",
        repo,
        "--workflow",
        workflow,
        "--limit",
        String(limit),
        "--json",
        "databaseId,headSha,createdAt,updatedAt,status,conclusion,event",
      ]),
    );
  } catch {
    return [];
  }
}

function findTimings(dir) {
  const found = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) found.push(...findTimings(p));
    else if (entry === "ci-timings.json")
      found.push(JSON.parse(readFileSync(p, "utf8")));
  }
  return found;
}

function downloadTimings(repo, runId) {
  const dir = mkdtempSync(join(tmpdir(), "ci-timings-"));
  try {
    gh([
      "run",
      "download",
      String(runId),
      "-R",
      repo,
      "-p",
      "ci-out-*",
      "-D",
      dir,
    ]);
    return findTimings(dir);
  } catch {
    return [];
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function main(argv) {
  let repo = "tinyhumansai/openhuman";
  let limit = 50;
  let lanes = false;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--repo") repo = argv[++i];
    else if (argv[i] === "--limit") limit = Number.parseInt(argv[++i], 10);
    else if (argv[i] === "--lanes") lanes = true;
    else throw new Error(`unknown argument ${argv[i]}`);
  }
  const runs = Object.fromEntries(
    Object.entries(WORKFLOWS).map(([kind, wf]) => [
      kind,
      listRuns(repo, wf, limit),
    ]),
  );
  const rows = pairRuns(runs);
  if (rows.length === 0) {
    console.log("No commit has both a CI Lite run and a lane-flow run yet.");
    return;
  }
  const fmt = (r) =>
    r ? `${minutesBetween(r.createdAt, r.updatedAt)}m ${r.conclusion}` : "—";
  console.log(
    "| sha | CI Lite | CI Fast (EX63) | CI Fast (hosted) | EX63 speed-up |",
  );
  console.log("| --- | --- | --- | --- | --- |");
  for (const row of rows) {
    const lite = minutesBetween(row.lite.createdAt, row.lite.updatedAt);
    const ex = row.ex63
      ? minutesBetween(row.ex63.createdAt, row.ex63.updatedAt)
      : null;
    const speed = ex ? `${Math.round((lite / ex) * 10) / 10}x` : "—";
    console.log(
      `| ${row.sha.slice(0, 10)} | ${fmt(row.lite)} | ${fmt(row.ex63)} | ${fmt(row.hosted)} | ${speed} |`,
    );
    if (lanes) {
      for (const kind of ["ex63", "hosted"]) {
        if (!row[kind]) continue;
        for (const t of downloadTimings(repo, row[kind].databaseId)) {
          const per = Object.entries(laneMinutes(t))
            .map(([k, v]) => `${k}=${v}m`)
            .join(" ");
          console.log(`|   ↳ ${kind} lanes | ${per} | | | |`);
        }
      }
    }
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
