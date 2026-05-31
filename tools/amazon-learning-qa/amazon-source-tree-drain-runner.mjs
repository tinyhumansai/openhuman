#!/usr/bin/env node

import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { appendFile, mkdir, open, readFile, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

import { resolveAmazonQaPaths } from "./amazon-qa-paths.mjs";

const QA_PATHS = resolveAmazonQaPaths(import.meta.dirname);
const WORKSPACE = QA_PATHS.workspace;
const OPENHUMAN_REPO = QA_PATHS.repoRoot;
const RUN_DIR = QA_PATHS.runRoot;
const STATE_PATH = path.join(RUN_DIR, "source-tree-drain.json");
const LOCK_PATH = path.join(RUN_DIR, "source-tree-drain.lock.json");
const STOP_PATH = path.join(RUN_DIR, "source-tree-drain.stop");
const LOG_DIR = path.join(RUN_DIR, "logs");
const MEMORY_TREE_DB_PATH = QA_PATHS.memoryTreeDbPath;
const DRAIN_BIN = QA_PATHS.drainBin;

const execFileAsync = promisify(execFile);

function usage() {
  console.log(`Usage:
  node tools/amazon-source-tree-drain-runner.mjs [--max-jobs 250] [--batch-size 1] [--sleep-ms 1000]

Runs OpenHuman memory_tree background jobs in resumable batches and writes progress to:
  ${STATE_PATH}`);
}

function argValue(args, name, fallback = undefined) {
  const index = args.indexOf(name);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`Missing value for ${name}`);
  return value;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    usage();
    return;
  }

  const maxJobs = clampNumberArg(argValue(args, "--max-jobs", "250"), 250, 1, 250);
  const batchSize = clampNumberArg(argValue(args, "--batch-size", "1"), 1, 1, 100);
  const sleepMs = clampNumberArg(argValue(args, "--sleep-ms", "1000"), 1000, 0, 60000);

  await mkdir(RUN_DIR, { recursive: true });
  await mkdir(LOG_DIR, { recursive: true });
  const runId = new Date().toISOString().replace(/[:.]/g, "-");
  const logPath = path.join(LOG_DIR, `source-tree-drain-${runId}.log`);
  await claimLock(runId);
  await rm(STOP_PATH, { force: true }).catch(() => {});

  let processedJobs = 0;
  try {
    if (!existsSync(DRAIN_BIN)) throw new Error(`Drain binary not found: ${DRAIN_BIN}`);
    await writeLog(logPath, `start run_id=${runId} max_jobs=${maxJobs} batch_size=${batchSize} sleep_ms=${sleepMs}`);
    await writeState({ state: "running", runId, logPath, processedJobs, maxJobs, batchSize, startedAt: new Date().toISOString() });

    while (processedJobs < maxJobs) {
      if (await stopRequested()) break;
      const before = await readJobCounts();
      if (before.queuedJobs <= 0) break;

      const remaining = Math.min(batchSize, maxJobs - processedJobs);
      const childLimit = Math.max(1, remaining);
      await writeLog(logPath, `batch start limit=${childLimit} configured_batch_size=${batchSize} before_queued=${before.queuedJobs}`);
      await writeState({
        state: "running",
        runId,
        logPath,
        stopRequested: false,
        processedJobs,
        maxJobs,
        batchSize,
        queuedJobs: before.queuedJobs,
        readyJobs: before.readyJobs,
        runningJobs: before.runningJobs,
        failedJobs: before.failedJobs,
        doneJobs: before.doneJobs,
        activeBatch: {
          limit: childLimit,
          configuredBatchSize: batchSize,
          beforeQueuedJobs: before.queuedJobs,
          beforeReadyJobs: before.readyJobs,
          beforeRunningJobs: before.runningJobs,
          beforeDoneJobs: before.doneJobs,
          startedAt: new Date().toISOString(),
        },
      });
      const batch = await runDrainBatch(childLimit, logPath);
      processedJobs += batch.processed;
      const after = await readJobCounts();
      const requestedStopAfterBatch = await stopRequested();
      await writeLog(logPath, `batch finish processed=${batch.processed} after_queued=${after.queuedJobs} failed=${after.failedJobs} stop_requested=${requestedStopAfterBatch}`);
      await writeState({
        state: requestedStopAfterBatch ? "stopping" : "running",
        runId,
        logPath,
        stopRequested: requestedStopAfterBatch,
        processedJobs,
        maxJobs,
        batchSize,
        queuedJobs: after.queuedJobs,
        readyJobs: after.readyJobs,
        runningJobs: after.runningJobs,
        failedJobs: after.failedJobs,
        doneJobs: after.doneJobs,
        lastBatch: {
          processed: batch.processed,
          limit: childLimit,
          configuredBatchSize: batchSize,
          beforeQueuedJobs: before.queuedJobs,
          afterQueuedJobs: after.queuedJobs,
          queuedDelta: before.queuedJobs - after.queuedJobs,
          beforeDoneJobs: before.doneJobs,
          afterDoneJobs: after.doneJobs,
          doneDelta: after.doneJobs - before.doneJobs,
        },
        activeBatch: null,
      });

      if (batch.processed <= 0 || after.queuedJobs <= 0 || requestedStopAfterBatch) break;
      if (sleepMs > 0) await sleep(sleepMs);
    }

    const finalCounts = await readJobCounts();
    const requestedStop = await stopRequested();
    await writeLog(logPath, `finish state=${finalCounts.queuedJobs > 0 ? "paused" : "complete"} processed=${processedJobs} queued=${finalCounts.queuedJobs} failed=${finalCounts.failedJobs} stop_reason=${requestedStop ? "user_requested" : ""}`);
    await writeState({
      state: finalCounts.queuedJobs > 0 ? "paused" : "complete",
      runId,
      logPath,
      stopRequested: false,
      processedJobs,
      maxJobs,
      batchSize,
      queuedJobs: finalCounts.queuedJobs,
      readyJobs: finalCounts.readyJobs,
      runningJobs: finalCounts.runningJobs,
      failedJobs: finalCounts.failedJobs,
      doneJobs: finalCounts.doneJobs,
      finishedAt: new Date().toISOString(),
      stopReason: requestedStop ? "user_requested" : "",
    });
  } catch (error) {
    const counts = await readJobCounts().catch(() => ({ queuedJobs: 0, failedJobs: 0 }));
    await writeLog(logPath, `failed error=${error instanceof Error ? error.stack || error.message : String(error)}`).catch(() => {});
    await writeState({
      state: "failed",
      runId,
      logPath,
      processedJobs,
      maxJobs,
      batchSize,
      queuedJobs: counts.queuedJobs,
      readyJobs: counts.readyJobs,
      runningJobs: counts.runningJobs,
      failedJobs: counts.failedJobs,
      doneJobs: counts.doneJobs,
      error: error instanceof Error ? error.message : String(error),
      finishedAt: new Date().toISOString(),
    });
    process.exitCode = 1;
  } finally {
    await rm(LOCK_PATH, { force: true }).catch(() => {});
  }
}

function clampNumberArg(value, fallback, min, max) {
  const number = Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.min(max, Math.max(min, Math.round(number)));
}

async function claimLock(runId) {
  const current = await readJson(LOCK_PATH);
  if (current?.pid && isProcessAlive(Number(current.pid))) {
    throw new Error(`Source-tree drain is already running with pid ${current.pid}`);
  }
  try {
    const handle = await open(LOCK_PATH, "wx");
    await handle.writeFile(JSON.stringify({ pid: process.pid, runId, startedAt: new Date().toISOString() }, null, 2), "utf8");
    await handle.close();
  } catch (error) {
    if (error?.code !== "EEXIST") throw error;
    const raced = await readJson(LOCK_PATH);
    if (raced?.pid && !isProcessAlive(Number(raced.pid))) {
      await rm(LOCK_PATH, { force: true });
      const handle = await open(LOCK_PATH, "wx");
      await handle.writeFile(JSON.stringify({ pid: process.pid, runId, startedAt: new Date().toISOString() }, null, 2), "utf8");
      await handle.close();
      return;
    }
    throw new Error(`Source-tree drain is already running${raced?.pid ? ` with pid ${raced.pid}` : ""}`);
  }
}

async function runDrainBatch(limit, logPath) {
  const { stdout, stderr } = await execFileAsync(
    DRAIN_BIN,
    ["--limit", String(limit), "--progress-every", String(Math.min(25, Math.max(1, limit)))],
    {
      cwd: OPENHUMAN_REPO,
      env: {
        ...process.env,
        PATH: `/Users/yangyingjia/.cargo/bin:/opt/homebrew/bin:${process.env.PATH || ""}`,
        OPENHUMAN_WORKSPACE: WORKSPACE,
        RUST_LOG: process.env.RUST_LOG || "warn",
      },
      timeout: 30 * 60 * 1000,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
  if (stderr) await writeLog(logPath, stderr.trim());
  if (stdout) await writeLog(logPath, stdout.trim());
  const parsed = JSON.parse(stdout || "{}");
  return { processed: Math.max(0, Number(parsed.processed || 0)) };
}

async function readJobCounts() {
  if (!existsSync(MEMORY_TREE_DB_PATH)) return { queuedJobs: 0, failedJobs: 0 };
  const { stdout } = await execFileAsync(
    "sqlite3",
    ["-cmd", ".timeout 15000", MEMORY_TREE_DB_PATH, "select status || '|' || count(*) from mem_tree_jobs group by status;"],
    { timeout: 18000 },
  );
  const counts = { readyJobs: 0, runningJobs: 0, failedJobs: 0, doneJobs: 0 };
  for (const line of stdout.split("\n").map((item) => item.trim()).filter(Boolean)) {
    const [status, countText] = line.split("|");
    const count = Math.max(0, Number(countText || 0));
    if (status === "ready") counts.readyJobs = count;
    if (status === "running") counts.runningJobs = count;
    if (status === "failed") counts.failedJobs = count;
    if (status === "done") counts.doneJobs = count;
  }
  return {
    readyJobs: counts.readyJobs,
    runningJobs: counts.runningJobs,
    queuedJobs: counts.readyJobs + counts.runningJobs,
    failedJobs: counts.failedJobs,
    doneJobs: counts.doneJobs,
  };
}

async function writeState(next) {
  const previous = await readJson(STATE_PATH);
  const merged = {
    ...(previous || {}),
    ...next,
    pid: process.pid,
    updatedAt: new Date().toISOString(),
  };
  if (["starting", "running", "stopping"].includes(String(merged.state || ""))) {
    delete merged.finishedAt;
    delete merged.error;
    if (!next.stopReason) delete merged.stopReason;
    if (!next.lastBatch && Number(next.processedJobs || 0) === 0) delete merged.lastBatch;
  }
  if (!merged.startedAt) merged.startedAt = merged.updatedAt;
  const tmpPath = `${STATE_PATH}.${process.pid}.tmp`;
  await writeFile(tmpPath, JSON.stringify(merged, null, 2), "utf8");
  await rename(tmpPath, STATE_PATH);
}

async function stopRequested() {
  return existsSync(STOP_PATH);
}

async function writeLog(logPath, line) {
  const text = String(line || "").trim();
  if (!text) return;
  await appendFile(logPath, `[${new Date().toISOString()}] ${text}\n`, "utf8");
}

async function readJson(file) {
  try {
    return JSON.parse(await readFile(file, "utf8"));
  } catch {
    return null;
  }
}

function isProcessAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
