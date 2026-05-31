#!/usr/bin/env node

import { createReadStream } from "node:fs";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { createInterface } from "node:readline/promises";
import { execFile } from "node:child_process";
import path from "node:path";
import { promisify } from "node:util";

import {
  amazonSourceId,
  buildMemoryTreeIngestRequest,
  sourceTreeImportPlan,
  summarizeSourceTreeStatus,
} from "./amazon-source-tree-lib.mjs";
import { resolveAmazonQaPaths } from "./amazon-qa-paths.mjs";

const QA_PATHS = resolveAmazonQaPaths(import.meta.dirname);
const ROOT = QA_PATHS.root;
const MANIFEST_PATH = QA_PATHS.manifestPath;
const MEMORY_TREE_DB_PATH = QA_PATHS.memoryTreeDbPath;
const DEFAULT_RPC = "http://127.0.0.1:7789/rpc";
const DEFAULT_NAMESPACE = "amazon-learning";
const DEFAULT_TOKEN = "openhuman-amazon-local-token";

const execFileAsync = promisify(execFile);

function usage() {
  console.log(`Usage:
  node tools/amazon-source-tree.mjs status
  node tools/amazon-source-tree.mjs import [--limit N] [--rpc ${DEFAULT_RPC}] [--namespace ${DEFAULT_NAMESPACE}] [--dry-run]

Imports only author original articles into OpenHuman memory_tree via openhuman.memory_tree_ingest.
Learning dossiers, product notes, and chat history are intentionally excluded.`);
}

function argValue(args, name, fallback = undefined) {
  const index = args.indexOf(name);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`Missing value for ${name}`);
  return value;
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (!command || command === "help" || command === "--help" || command === "-h") {
    usage();
    return;
  }
  if (command === "status") return status();
  if (command === "import") return importSourceTree(args);
  throw new Error(`Unknown command: ${command}`);
}

async function status() {
  const rows = await readManifestRows();
  const stats = await readMemoryTreeStats();
  console.log(JSON.stringify(summarizeSourceTreeStatus({ manifestRows: rows, stats }), null, 2));
}

async function importSourceTree(args) {
  const rpcUrl = argValue(args, "--rpc", DEFAULT_RPC);
  const namespace = argValue(args, "--namespace", DEFAULT_NAMESPACE);
  const dryRun = args.includes("--dry-run");
  const limit = Number(argValue(args, "--limit", "0"));
  const rows = await readManifestRows();
  const stats = await readMemoryTreeStats();
  const plan = sourceTreeImportPlan(rows, {
    alreadyIngestedSourceIds: stats.ingestedSourceIds,
    limit,
  });

  if (dryRun) {
    console.log(JSON.stringify({ namespace, dryRun: true, ...planSummary(plan) }, null, 2));
    return;
  }

  let imported = 0;
  let skippedAlreadyIngested = 0;
  for (const row of plan.toImport) {
    const fullPath = path.join(ROOT, row.markdown_path);
    const markdown = await readFile(fullPath, "utf8");
    const request = buildMemoryTreeIngestRequest(row, markdown, { namespace });
    const result = await rpcCall(rpcUrl, "openhuman.memory_tree_ingest", request);
    if (result?.result?.already_ingested || result?.already_ingested) {
      skippedAlreadyIngested += 1;
    } else {
      imported += 1;
    }
    const done = imported + skippedAlreadyIngested;
    if (done % 25 === 0 || done === plan.toImport.length) {
      console.log(`source-tree imported ${done}/${plan.toImport.length}`);
    }
  }

  const after = summarizeSourceTreeStatus({
    manifestRows: rows,
    stats: await readMemoryTreeStats(),
  });
  console.log(JSON.stringify({ namespace, imported, skippedAlreadyIngested, sourceTree: after }, null, 2));
}

function planSummary(plan) {
  return {
    total: plan.total,
    alreadyIngested: plan.alreadyIngested,
    toImport: plan.toImport.length,
    firstSourceIds: plan.toImport.slice(0, 5).map(amazonSourceId),
  };
}

async function rpcCall(rpcUrl, method, params) {
  const token = process.env.OPENHUMAN_CORE_TOKEN || DEFAULT_TOKEN;
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: Date.now(), method, params }),
  });
  const payload = await response.json();
  if (!response.ok || payload.error) {
    throw new Error(`${method} failed: ${JSON.stringify(payload.error ?? payload)}`);
  }
  return payload.result;
}

async function readManifestRows() {
  const rows = [];
  const rl = createInterface({ input: createReadStream(MANIFEST_PATH, { encoding: "utf8" }) });
  for await (const line of rl) {
    if (line.trim()) rows.push(JSON.parse(line));
  }
  return rows;
}

async function readMemoryTreeStats() {
  if (!existsSync(MEMORY_TREE_DB_PATH)) {
    return { chunks: 0, trees: 0, summaries: 0, chunkSourceIds: [], ingestedSourceIds: [] };
  }
  const [counts, chunkSourceIds, ingestedSourceIds, jobStatusRows] = await Promise.all([
    sqliteLines(
      "select " +
        "(select count(*) from mem_tree_chunks) || '|' || " +
        "(select count(*) from mem_tree_trees) || '|' || " +
        "(select count(*) from mem_tree_summaries);",
    ),
    sqliteLines("select distinct source_id from mem_tree_chunks where source_kind='document' order by source_id;"),
    sqliteLines("select source_id from mem_tree_ingested_sources where source_kind='document' order by source_id;"),
    sqliteLines("select status || '|' || count(*) from mem_tree_jobs group by status;"),
  ]);
  const [chunks, trees, summaries] = String(counts[0] || "")
    .split("|")
    .map((value) => Number(value || 0));
  const jobCounts = parseJobStatusRows(jobStatusRows);
  return { chunks, trees, summaries, chunkSourceIds, ingestedSourceIds, ...jobCounts };
}

function parseJobStatusRows(rows = []) {
  const counts = { readyJobs: 0, runningJobs: 0, failedJobs: 0, doneJobs: 0 };
  for (const row of rows) {
    const [status, countText] = String(row || "").split("|");
    const count = Math.max(0, Number(countText || 0));
    if (status === "ready") counts.readyJobs = count;
    if (status === "running") counts.runningJobs = count;
    if (status === "failed") counts.failedJobs = count;
    if (status === "done") counts.doneJobs = count;
  }
  return counts;
}

async function sqliteLines(sql) {
  const { stdout } = await execFileAsync("sqlite3", ["-cmd", ".timeout 15000", MEMORY_TREE_DB_PATH, sql], { timeout: 18000 });
  return stdout
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
