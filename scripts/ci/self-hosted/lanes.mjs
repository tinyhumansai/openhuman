#!/usr/bin/env node
// Run the CI lane plan (lanes-plan.mjs): private lanes sequentially, hosted
// lanes with bounded concurrency, and each lane's checks in order.
//
// Usage:
//   node scripts/ci/self-hosted/lanes.mjs --profile ex63|hosted
//        [--lanes a,b] [--max-parallel N] [--out ci-out] [--dry-run]
//        [--print-matrix] [--detach]
//   node scripts/ci/self-hosted/lanes.mjs --wait <lane> [--out ci-out]
//   node scripts/ci/self-hosted/lanes.mjs --wait-all [--out ci-out]
//
// Step-per-lane mode, so a workflow shows every lane as its own step while
// the profile's scheduling remains in effect:
//   1. `--detach` starts the runner in the background (under
//      scripts/ci-cancel-aware.sh) and writes the active lanes as JSON to
//      $GITHUB_OUTPUT `lanes`.
//   2. one `--wait <lane>` step per lane streams that lane's log live, one
//      folded group per check, and exits with that lane's own result.
//   3. `--wait-all` waits for the whole run, writes the step summary and exits
//      with the overall result.
//
// Area flags come from CI_AREA_* (see AREA_ENV). Outputs, under --out:
//   logs/<lane>.log     full output of each lane
//   lcov/*.info         coverage written by the coverage checks
//   ci-timings.json     per-check outcome, timings, target-dir sizes
// and a per-check outcome table in $GITHUB_STEP_SUMMARY.
//
// Exit status is non-zero when any gating check failed or was blocked by a
// failed dependency. A failed check never stops the checks after it (#6451).
import { spawn, spawnSync } from "node:child_process";
import {
  appendFileSync,
  closeSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  areasFromEnv,
  buildPlan,
  hostedMatrix,
  selectLanes,
  validatePlan,
} from "./lanes-plan.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const HEARTBEAT_MS = 5 * 60 * 1000;
const KILL_GRACE_MS = 10 * 1000;

function log(msg) {
  console.log(`[ci][lanes] ${msg}`);
}

export function parseArgs(argv) {
  const args = {
    profile: null,
    lanes: [],
    maxParallel: 0,
    out: "ci-out",
    dryRun: false,
    printMatrix: false,
    detach: false,
    wait: null,
    waitAll: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      return argv[++i];
    };
    if (a === "--profile") args.profile = next();
    else if (a === "--lanes")
      args.lanes = next()
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    else if (a === "--max-parallel")
      args.maxParallel = Number.parseInt(next(), 10);
    else if (a === "--out") args.out = next();
    else if (a === "--dry-run") args.dryRun = true;
    else if (a === "--print-matrix") args.printMatrix = true;
    else if (a === "--detach") args.detach = true;
    else if (a === "--wait") args.wait = next();
    else if (a === "--wait-all") args.waitAll = true;
    else throw new Error(`unknown argument ${a}`);
  }
  if (!args.profile && !args.wait && !args.waitAll)
    throw new Error("--profile is required");
  return args;
}

/**
 * Lanes that consume another lane's check must start after it, or a lane
 * waiting on a provider that has no free slot would deadlock.
 */
export function orderProblems(plan) {
  const index = new Map(plan.lanes.map((l, i) => [l.name, i]));
  const problems = [];
  plan.lanes.forEach((lane, i) => {
    for (const c of lane.checks) {
      for (const need of c.needs) {
        if (!need.includes(":")) continue;
        const provider = need.split(":")[0];
        if (!index.has(provider))
          problems.push(
            `${lane.name}:${c.name} needs ${need}, whose lane is not selected`,
          );
        else if (index.get(provider) > i)
          problems.push(
            `${lane.name}:${c.name} needs ${need}, whose lane is ordered after it`,
          );
      }
    }
  });
  return problems;
}

/** Render the per-check outcome table for $GITHUB_STEP_SUMMARY. */
export function renderSummary(results) {
  const rows = [
    "### CI lanes — per-check outcome",
    "",
    "| lane | check | outcome | time | peak RSS |",
    "| --- | --- | --- | --- | --- |",
  ];
  for (const lane of results.lanes) {
    for (const c of lane.checks) {
      const time =
        c.durationS == null
          ? "—"
          : `${Math.floor(c.durationS / 60)}m${String(c.durationS % 60).padStart(2, "0")}s`;
      const outcome =
        c.reportOnly && c.status === "failure"
          ? "failure (report-only)"
          : c.status;
      const rss = c.peakRssMiB
        ? `${(c.peakRssMiB / 1024).toFixed(1)} GiB`
        : "—";
      rows.push(`| ${lane.name} | ${c.name} | ${outcome} | ${time} | ${rss} |`);
    }
  }
  const mem = results.memory;
  if (mem?.totalMiB)
    rows.push(
      "",
      `VM memory: ${(mem.totalMiB / 1024).toFixed(1)} GiB, lowest available ${mem.minAvailableMiB == null ? "?" : `${(mem.minAvailableMiB / 1024).toFixed(1)} GiB`}; heavy-compile slots: ${results.heavySlots ?? "uncapped"}.`,
    );
  rows.push(
    "",
    "`skipped` = area untouched; `blocked` = a check it needs did not succeed. Neither is a pass for the check itself.",
    "",
  );
  return rows.join("\n");
}

/** A gating failure: a non-report-only check that failed, was blocked or cancelled. */
export function gatingFailures(results) {
  const failed = [];
  for (const lane of results.lanes) {
    for (const c of lane.checks) {
      if (c.reportOnly) continue;
      if (
        c.status === "failure" ||
        c.status === "blocked" ||
        c.status === "cancelled"
      )
        failed.push(`${lane.name}:${c.name}`);
    }
  }
  return failed;
}

function du(path) {
  if (!path || !existsSync(path)) return null;
  // `du` exits 1 when a subdirectory is unreadable (the cache disk's
  // root-owned lost+found) but still prints the total of what it could read.
  const r = spawnSync("du", ["-sb", path], { encoding: "utf8" });
  const bytes = Number.parseInt((r.stdout ?? "").split(/\s+/)[0], 10);
  return Number.isFinite(bytes) ? bytes : null;
}

/**
 * Where the scratch disk's space goes, for the ex63 profile only: `df -h` of
 * the scratch mount, then `du -sh` of every top-level entry under `target/` and
 * of the scratch dir's own top level (the checkout). A lane
 * that fills the disk (EX63 run 39292268284 ran it to 16 MB free) then shows
 * which directory grew, lane by lane. Returns null off ex63 or without a
 * scratch dir. `run` is injectable for tests.
 */
export function diskUsageReport({
  profile,
  scratch = process.env.CI_SCRATCH_DIR,
  root = ROOT,
  run = (cmd) =>
    spawnSync("sh", ["-c", cmd], { encoding: "utf8", cwd: root }).stdout ?? "",
} = {}) {
  if (profile !== "ex63" || !scratch) return null;
  const q = (v) => `'${String(v).replace(/'/g, `'\\''`)}'`;
  const df = run(`df -h ${q(scratch)} 2>&1`).trim();
  // `du` exits non-zero on an unreadable entry but still prints the rest.
  const target = run(`du -sh target/* 2>/dev/null | sort -h -r; true`).trim();
  const top = run(
    `du -xsh ${q(scratch)}/* 2>/dev/null | sort -h -r | head -15; true`,
  ).trim();
  return [
    `[ci][lanes][disk] df -h ${scratch}`,
    df,
    "[ci][lanes][disk] du -sh target/*",
    target || "(no target dir)",
    `[ci][lanes][disk] du -sh ${scratch}/* (top 15)`,
    top || "(nothing readable)",
  ].join("\n");
}

/**
 * Counting semaphore with priorities (lower first, then FIFO). Heavy lanes
 * each compile the whole core crate, several GB of rustc apiece, so the VM
 * runs only as many at once as its memory holds.
 */
export class PrioritySemaphore {
  constructor(slots) {
    this.free = slots;
    this.waiters = [];
    this.seq = 0;
  }

  acquire(priority) {
    if (this.free > 0 && this.waiters.length === 0) {
      this.free--;
      return Promise.resolve();
    }
    return new Promise((res) => {
      this.waiters.push({ priority, seq: this.seq++, res });
      this.waiters.sort((a, b) => a.priority - b.priority || a.seq - b.seq);
    });
  }

  release() {
    const next = this.waiters.shift();
    if (next) next.res();
    else this.free++;
  }
}

/** Heavy-compile slots for a VM with `memTotalMiB` of RAM: ~1 per 9 GiB. */
export function defaultHeavySlots(memTotalMiB) {
  return Math.max(1, Math.floor(memTotalMiB / (9 * 1024)));
}

function memInfoMiB() {
  try {
    const text = readFileSync("/proc/meminfo", "utf8");
    const get = (k) => {
      const m = text.match(new RegExp(`^${k}:\\s+(\\d+) kB`, "m"));
      return m ? Math.round(Number(m[1]) / 1024) : null;
    };
    return { total: get("MemTotal"), available: get("MemAvailable") };
  } catch {
    return { total: null, available: null };
  }
}

/**
 * The process table from /proc (Linux only; empty elsewhere): each pid's
 * parent and resident MiB.
 */
export function processTable(procDir = "/proc") {
  const table = new Map();
  let pids;
  try {
    pids = readdirSync(procDir).filter((d) => /^\d+$/.test(d));
  } catch {
    return table;
  }
  for (const pid of pids) {
    try {
      const stat = readFileSync(`${procDir}/${pid}/stat`, "utf8");
      const f = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      // After the comm field: [0]=state [1]=ppid ... [21]=rss (pages)
      table.set(Number(pid), {
        ppid: Number(f[1]),
        rssMiB: (Number(f[21]) * 4096) / 1048576,
      });
    } catch {}
  }
  return table;
}

/**
 * Resident MiB of `root` and all its descendants. Follows parent links, not
 * process groups: scripts/ci-cancel-aware.sh starts cargo under `setsid`, so a
 * check's compilers and test binaries leave the group the runner spawned.
 */
export function treeRssMiB(table, root) {
  const children = new Map();
  for (const [pid, { ppid }] of table) {
    if (!children.has(ppid)) children.set(ppid, []);
    children.get(ppid).push(pid);
  }
  let total = 0;
  const stack = [root];
  const seen = new Set();
  while (stack.length) {
    const pid = stack.pop();
    if (seen.has(pid)) continue;
    seen.add(pid);
    total += table.get(pid)?.rssMiB ?? 0;
    for (const c of children.get(pid) ?? []) stack.push(c);
  }
  return total;
}

export class Runner {
  constructor(plan, { out, maxParallel, maxHeavy = Infinity }) {
    this.plan = plan;
    this.out = out;
    // ex63 runs lanes one at a time. Overlapping the frontend lanes with the
    // cargo chain was measured in run 39434038570 on 4 vCPU / 24 GB: vitest
    // took 2.6x as long and cache restores 5.6x, rust-cov peaked at 19.4 GB,
    // and nothing finished sooner, since compiles and tests already use every
    // core.
    this.maxParallel =
      plan.profile === "ex63"
        ? 1
        : maxParallel > 0
          ? maxParallel
          : plan.lanes.length;
    this.heavy = new PrioritySemaphore(maxHeavy);
    this.maxHeavy = maxHeavy;
    // check id -> { pgid, peakRssMiB } while running; sampled every 5 s.
    this.running = new Map();
    this.memory = { totalMiB: memInfoMiB().total, minAvailableMiB: null };
    this.children = new Set();
    this.cancelled = false;
    this.done = new Map(); // "lane:check" -> Promise<status>
    this.resolvers = new Map();
    for (const lane of plan.lanes) {
      for (const c of lane.checks) {
        const id = `${lane.name}:${c.name}`;
        this.done.set(id, new Promise((res) => this.resolvers.set(id, res)));
      }
    }
  }

  finish(id, status) {
    this.resolvers.get(id)?.(status);
  }

  cancel(signal) {
    if (this.cancelled) return;
    this.cancelled = true;
    log(`received ${signal}; stopping ${this.children.size} running check(s)`);
    for (const child of this.children) {
      try {
        process.kill(-child.pid, "SIGTERM");
      } catch {}
    }
    setTimeout(() => {
      for (const child of this.children) {
        try {
          process.kill(-child.pid, "SIGKILL");
        } catch {}
      }
    }, KILL_GRACE_MS).unref();
  }

  /** Sample VM memory and each running check's process-group RSS. */
  sampleMemory() {
    const { available } = memInfoMiB();
    if (
      available != null &&
      (this.memory.minAvailableMiB == null ||
        available < this.memory.minAvailableMiB)
    )
      this.memory.minAvailableMiB = available;
    if (this.running.size === 0) return;
    const table = processTable();
    for (const entry of this.running.values()) {
      const now = Math.round(treeRssMiB(table, entry.pgid));
      if (now > entry.record.peakRssMiB) entry.record.peakRssMiB = now;
    }
  }

  runCommand(lane, check, logStream, record) {
    const env = { ...process.env, ...(lane.env ?? {}), ...(check.env ?? {}) };
    if (lane.targetDir) {
      env.CARGO_BUILD_TARGET_DIR =
        check.env?.CARGO_BUILD_TARGET_DIR ?? lane.targetDir;
      delete env.CARGO_TARGET_DIR;
      mkdirSync(lane.targetDir, { recursive: true });
    }
    const argv = ["bash", "-o", "pipefail", "-c", check.run];
    const cmd = lane.nice ? ["nice", "-n", String(lane.nice), ...argv] : argv;
    return new Promise((res) => {
      const child = spawn(cmd[0], cmd.slice(1), {
        cwd: ROOT,
        env,
        detached: true,
        stdio: ["ignore", "pipe", "pipe"],
      });
      this.children.add(child);
      const key = `${lane.name}:${check.name}`;
      if (record) this.running.set(key, { pgid: child.pid, record });
      child.stdout.pipe(logStream, { end: false });
      child.stderr.pipe(logStream, { end: false });
      child.on("error", (err) => {
        logStream.write(`[ci][lanes] spawn failed: ${err.message}\n`);
      });
      child.on("close", (code, signal) => {
        this.children.delete(child);
        this.running.delete(key);
        res({ code: code ?? (signal ? 128 : 1), signal });
      });
    });
  }

  /** Publish a lane's outcome for its `--wait` step (atomic rename). */
  writeStatus(name, value) {
    const dir = join(this.out, "status");
    mkdirSync(dir, { recursive: true });
    const tmp = join(dir, `.${name}.json.tmp`);
    writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
    renameSync(tmp, join(dir, `${name}.json`));
  }

  async runLane(lane) {
    let heavyWaitS = null;
    if (lane.heavy != null && Number.isFinite(this.maxHeavy)) {
      const w0 = Date.now();
      log(
        `lane ${lane.name}: waiting for a heavy-compile slot (priority ${lane.heavy})`,
      );
      await this.heavy.acquire(lane.heavy);
      heavyWaitS = Math.round((Date.now() - w0) / 1000);
      log(`lane ${lane.name}: got a heavy-compile slot after ${heavyWaitS}s`);
    }
    try {
      return await this.runLaneChecks(lane, heavyWaitS);
    } finally {
      if (heavyWaitS != null) this.heavy.release();
    }
  }

  async runLaneChecks(lane, heavyWaitS) {
    const logPath = join(this.out, "logs", `${lane.name}.log`);
    const logStream = createWriteStream(logPath);
    const results = [];
    log(
      `lane ${lane.name}: start${lane.targetDir ? ` (target ${lane.targetDir})` : ""}`,
    );
    for (const check of lane.checks) {
      const id = `${lane.name}:${check.name}`;
      const record = {
        name: check.name,
        status: "skipped",
        reportOnly: Boolean(check.reportOnly),
        exitCode: null,
        start: null,
        end: null,
        durationS: null,
        peakRssMiB: 0,
      };
      results.push(record);
      if (!check.when) {
        this.finish(id, "skipped");
        continue;
      }
      if (this.cancelled) {
        record.status = "cancelled";
        this.finish(id, "cancelled");
        continue;
      }
      const needs = check.needs.map((n) =>
        n.includes(":") ? n : `${lane.name}:${n}`,
      );
      const needStatuses = await Promise.all(
        needs.map((n) => this.done.get(n) ?? Promise.resolve("missing")),
      );
      const unmet = needs.filter((_, i) => needStatuses[i] !== "success");
      if (unmet.length > 0) {
        record.status = "blocked";
        logStream.write(
          `\n[ci][lanes] ${check.name}: blocked — needs ${unmet.join(", ")}\n`,
        );
        log(`lane ${lane.name}: ${check.name} blocked by ${unmet.join(", ")}`);
        this.finish(id, "blocked");
        continue;
      }
      record.start = new Date().toISOString();
      const t0 = Date.now();
      logStream.write(
        `\n[ci][lanes] ===== ${check.name} =====\n$ ${check.run}\n`,
      );
      log(`lane ${lane.name}: ${check.name} started`);
      const beat = setInterval(() => {
        log(
          `lane ${lane.name}: ${check.name} still running (${Math.round((Date.now() - t0) / 60000)} min)`,
        );
      }, HEARTBEAT_MS);
      const { code } = await this.runCommand(lane, check, logStream, record);
      clearInterval(beat);
      record.end = new Date().toISOString();
      record.durationS = Math.round((Date.now() - t0) / 1000);
      record.exitCode = code;
      record.status = this.cancelled
        ? "cancelled"
        : code === 0
          ? "success"
          : "failure";
      logStream.write(
        `[ci][lanes] ${check.name}: ${record.status} (exit ${code}, ${record.durationS}s)\n`,
      );
      log(
        `lane ${lane.name}: ${check.name} ${record.status} in ${record.durationS}s${record.reportOnly ? " (report-only)" : ""}`,
      );
      this.finish(id, record.status);
    }
    const disk = diskUsageReport({ profile: this.plan.profile });
    if (disk) {
      logStream.write(`\n[ci][lanes] lane ${lane.name} end\n${disk}\n`);
    }
    await new Promise((res) => logStream.end(res));
    // Dump the whole lane log once, folded, so parallel lanes never interleave.
    const failed = results.some((r) => r.status === "failure" && !r.reportOnly);
    console.log(`::group::lane ${lane.name} log${failed ? " (FAILED)" : ""}`);
    process.stdout.write(readFileSync(logPath, "utf8"));
    console.log("::endgroup::");
    const laneResult = {
      name: lane.name,
      heavyWaitS,
      targetDir: lane.targetDir ?? null,
      targetBytes: du(lane.targetDir),
      checks: results,
    };
    this.writeStatus(lane.name, laneResult);
    return laneResult;
  }

  async run() {
    const sampler = setInterval(() => this.sampleMemory(), 5000);
    try {
      return await this.runQueue();
    } finally {
      clearInterval(sampler);
    }
  }

  async runQueue() {
    const queue = [...this.plan.lanes];
    const running = new Set();
    const finished = [];
    await new Promise((resolveAll) => {
      const pump = () => {
        while (running.size < this.maxParallel && queue.length > 0) {
          const lane = queue.shift();
          const p = this.runLane(lane).then((r) => {
            running.delete(p);
            finished.push(r);
            pump();
          });
          running.add(p);
        }
        if (running.size === 0 && queue.length === 0) resolveAll();
      };
      pump();
    });
    const order = new Map(this.plan.lanes.map((l, i) => [l.name, i]));
    finished.sort((a, b) => order.get(a.name) - order.get(b.name));
    return finished;
  }
}

const sleep = (ms) => new Promise((res) => setTimeout(res, ms));

/**
 * Turn one lane-log line into workflow-command output: every check becomes a
 * folded `::group::`, closed by the runner's own outcome line for it.
 */
export function foldLine(line, state) {
  const start = line.match(/^\[ci\]\[lanes\] ===== (.+) =====$/);
  if (start) {
    const close = state.open ? "::endgroup::\n" : "";
    state.open = true;
    return `${close}::group::${start[1]}`;
  }
  const end = line.match(
    /^\[ci\]\[lanes\] (\S+): (success|failure|cancelled) \(exit/,
  );
  if (end && state.open) {
    state.open = false;
    return `${line}\n::endgroup::`;
  }
  return line;
}

function runnerAlive(out) {
  try {
    process.kill(Number(readFileSync(join(out, "runner.pid"), "utf8")), 0);
    return true;
  } catch {
    return false;
  }
}

function printRunnerTail(out) {
  const p = join(out, "runner.log");
  if (!existsSync(p)) return;
  const lines = readFileSync(p, "utf8").split("\n");
  console.log("::group::runner.log (last 80 lines)");
  console.log(lines.slice(-80).join("\n"));
  console.log("::endgroup::");
}

/** Per-check table for one lane, as plain step output. */
export function renderLaneTable(lane) {
  const wait = lane.heavyWaitS
    ? ` (waited ${lane.heavyWaitS}s for a heavy-compile slot)`
    : "";
  const rows = [`lane ${lane.name}:${wait}`];
  for (const c of lane.checks) {
    if (c.status === "skipped") continue;
    const time = c.durationS == null ? "" : ` ${c.durationS}s`;
    const mem = c.peakRssMiB
      ? ` peak ${(c.peakRssMiB / 1024).toFixed(1)} GiB`
      : "";
    const note = c.reportOnly && c.status === "failure" ? " (report-only)" : "";
    rows.push(`  ${c.status.padEnd(9)} ${c.name}${time}${mem}${note}`);
  }
  return rows.join("\n");
}

/** `--wait <lane>`: stream one lane's log live and exit with its result. */
async function waitLane(out, name) {
  const logPath = join(out, "logs", `${name}.log`);
  const statusPath = join(out, "status", `${name}.json`);
  const donePath = join(out, "status", "_done.json");
  const state = { open: false };
  let pos = 0;
  let pending = "";
  const pump = () => {
    if (!existsSync(logPath)) return;
    const size = statSync(logPath).size;
    if (size <= pos) return;
    const fd = openSync(logPath, "r");
    const chunk = Buffer.alloc(size - pos);
    readSync(fd, chunk, 0, chunk.length, pos);
    closeSync(fd);
    pos = size;
    const lines = (pending + chunk.toString("utf8")).split("\n");
    pending = lines.pop();
    for (const l of lines) console.log(foldLine(l, state));
  };
  for (;;) {
    pump();
    if (existsSync(statusPath)) break;
    if (existsSync(donePath)) {
      console.log(`[ci][lanes] lane ${name} did not run in this job`);
      return 0;
    }
    if (!runnerAlive(out)) {
      console.error(
        `::error::[ci][lanes] the lane runner exited before lane ${name} finished`,
      );
      printRunnerTail(out);
      return 2;
    }
    await sleep(2000);
  }
  pump();
  if (pending) console.log(foldLine(pending, state));
  if (state.open) console.log("::endgroup::");
  const lane = JSON.parse(readFileSync(statusPath, "utf8"));
  console.log(renderLaneTable(lane));
  const failures = gatingFailures({ lanes: [lane] });
  for (const f of failures)
    console.log(`::error::[ci][lanes] ${f} did not pass`);
  return failures.length > 0 ? 1 : 0;
}

/** `--wait-all`: wait for the detached run, summarise, exit with its code. */
async function waitAll(out) {
  const donePath = join(out, "status", "_done.json");
  while (!existsSync(donePath)) {
    if (!runnerAlive(out)) {
      console.error(
        "::error::[ci][lanes] the lane runner exited without finishing",
      );
      printRunnerTail(out);
      return 2;
    }
    await sleep(2000);
  }
  const { code } = JSON.parse(readFileSync(donePath, "utf8"));
  const timingsPath = join(out, "ci-timings.json");
  if (existsSync(timingsPath)) {
    const results = JSON.parse(readFileSync(timingsPath, "utf8"));
    const summary = renderSummary(results);
    console.log(summary);
    if (process.env.GITHUB_STEP_SUMMARY)
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
    for (const f of gatingFailures(results))
      console.log(`::error::[ci][lanes] ${f} did not pass`);
  }
  return code;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.wait) return waitLane(resolve(ROOT, args.out), args.wait);
  if (args.waitAll) return waitAll(resolve(ROOT, args.out));
  const areas = areasFromEnv(process.env);
  const isPullRequest = (
    process.env.GITHUB_EVENT_NAME ?? "pull_request"
  ).startsWith("pull_request");
  const fullPlan = buildPlan({
    profile: args.profile,
    areas,
    env: process.env,
    isPullRequest,
  });

  if (args.printMatrix) {
    console.log(JSON.stringify({ include: hostedMatrix(fullPlan) }));
    return 0;
  }

  const plan = selectLanes(fullPlan, args.lanes);
  plan.lanes = plan.lanes.filter((l) => l.active);
  const problems = [...validatePlan(plan), ...orderProblems(plan)];
  if (problems.length > 0) {
    for (const p of problems) console.error(`::error::[ci][lanes] ${p}`);
    return 2;
  }

  log(
    `profile=${args.profile} areas=${
      Object.entries(areas)
        .filter(([, v]) => v)
        .map(([k]) => k)
        .join(",") || "(none)"
    }`,
  );
  for (const lane of plan.lanes) {
    const on = lane.checks.filter((c) => c.when).map((c) => c.name);
    log(`plan ${lane.name}: ${on.join(", ")}`);
  }
  if (args.dryRun) {
    for (const lane of plan.lanes) {
      for (const c of lane.checks.filter((x) => x.when))
        console.log(`${lane.name}:${c.name}\t${c.run}`);
    }
    return 0;
  }

  const out = resolve(ROOT, args.out);
  mkdirSync(join(out, "logs"), { recursive: true });
  mkdirSync(join(out, "lcov"), { recursive: true });
  mkdirSync(join(out, "status"), { recursive: true });

  if (args.detach) {
    // Re-run this same command in the background, under the cancellation
    // watchdog, and return at once so the workflow can show one step per lane.
    const childArgs = process.argv.slice(2).filter((a) => a !== "--detach");
    const logFd = openSync(join(out, "runner.log"), "a");
    const child = spawn(
      "bash",
      [
        "scripts/ci-cancel-aware.sh",
        process.execPath,
        fileURLToPath(import.meta.url),
        ...childArgs,
      ],
      {
        cwd: ROOT,
        detached: true,
        stdio: ["ignore", logFd, logFd],
        env: { ...process.env, OH_LANES_DETACHED: "1" },
      },
    );
    writeFileSync(join(out, "runner.pid"), String(child.pid));
    child.unref();
    closeSync(logFd);
    const lanesJson = JSON.stringify(plan.lanes.map((l) => l.name));
    if (process.env.GITHUB_OUTPUT)
      appendFileSync(process.env.GITHUB_OUTPUT, `lanes=${lanesJson}\n`);
    log(`detached runner pid=${child.pid} lanes=${lanesJson}`);
    return 0;
  }

  const envHeavy = Number.parseInt(process.env.OH_LANES_MAX_HEAVY ?? "", 10);
  const maxHeavy =
    args.profile !== "ex63"
      ? Infinity
      : Number.isInteger(envHeavy) && envHeavy > 0
        ? envHeavy
        : defaultHeavySlots(memInfoMiB().total ?? 8192);
  log(`heavy-compile slots: ${maxHeavy}`);
  const runner = new Runner(plan, {
    out,
    maxParallel: args.maxParallel,
    maxHeavy,
  });
  process.on("SIGTERM", () => runner.cancel("SIGTERM"));
  process.on("SIGINT", () => runner.cancel("SIGINT"));

  const started = new Date().toISOString();
  const lanes = await runner.run();
  const results = {
    profile: args.profile,
    started,
    finished: new Date().toISOString(),
    runner: process.env.RUNNER_NAME ?? null,
    areas,
    lanes,

    heavySlots: Number.isFinite(maxHeavy) ? maxHeavy : null,
    memory: runner.memory,
    cacheBytes: du(process.env.CI_CACHE_DIR),
  };
  writeFileSync(
    join(out, "ci-timings.json"),
    `${JSON.stringify(results, null, 2)}\n`,
  );
  // Detached, the step that owns $GITHUB_STEP_SUMMARY has already ended;
  // `--wait-all` writes the summary instead.
  if (process.env.GITHUB_STEP_SUMMARY && process.env.OH_LANES_DETACHED !== "1")
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, renderSummary(results));

  const failures = gatingFailures(results);
  let code = 0;
  if (runner.cancelled) code = 143;
  else if (failures.length > 0) {
    for (const f of failures)
      console.error(`::error::[ci][lanes] ${f} did not pass`);
    code = 1;
  } else log("all gating checks passed");
  writeFileSync(
    join(out, "status", "_done.json"),
    `${JSON.stringify({ code })}\n`,
  );
  return code;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().then(
    (code) => process.exit(code),
    (err) => {
      console.error(`::error::[ci][lanes] ${err.stack ?? err}`);
      process.exit(2);
    },
  );
}
