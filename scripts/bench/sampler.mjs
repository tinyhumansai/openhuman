#!/usr/bin/env node
/**
 * Resource sampler for the agent-scale benchmark tier.
 *
 * Samples a running `openhuman-core` process from /proc and writes one JSON
 * object per line to stdout. The analyzer consumes that stream; keeping the two
 * apart means a run can be re-analyzed with different thresholds without
 * re-running the load.
 *
 * Why this exists alongside `src/openhuman/platform/proc_metrics/`: that module
 * samples the CURRENT process, for benchmarks that embed the core as a library.
 * This tier deliberately measures a separate, normally-built server process
 * over RPC, so the sampling has to come from outside it.
 *
 * Fields per sample:
 *   tMs           ms since sampling started
 *   epochMs       wall-clock ms, for aligning against the driver's turn log
 *   rssKib        resident set size
 *   vmHwmKib      peak RSS the kernel has ever seen for this process
 *   pssKib        proportional set size (smaps_rollup; null if unreadable)
 *   privateKib    private clean+dirty (smaps_rollup; null if unreadable)
 *   cpuUserMs     cumulative user CPU
 *   cpuSystemMs   cumulative system CPU
 *   threads       thread count
 *   openFds       open file descriptors (null if unreadable)
 *   treeRssKib    self + descendants RSS, only with --tree
 *   children      descendant count, only with --tree
 *
 * Usage:
 *   node scripts/bench/sampler.mjs --pid <pid> [--interval-ms 250] [--tree] > samples.jsonl
 */

import fs from 'node:fs';
import path from 'node:path';

const CLOCK_TICKS_PER_SEC = 100; // _SC_CLK_TCK is 100 on every mainstream Linux.

function parseArgs(argv) {
  const opts = { pid: null, intervalMs: 250, tree: false, durationMs: null };
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--tree') {
      opts.tree = true;
    } else if (arg === '--pid') {
      const raw = argv[++i];
      if (raw === undefined) throw new Error('--pid expects a value');
      opts.pid = Number(raw);
    } else if (arg === '--interval-ms') {
      const raw = argv[++i];
      if (raw === undefined) throw new Error('--interval-ms expects a value');
      opts.intervalMs = Number(raw);
    } else if (arg === '--duration-ms') {
      const raw = argv[++i];
      if (raw === undefined) throw new Error('--duration-ms expects a value');
      opts.durationMs = Number(raw);
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  if (!Number.isInteger(opts.pid) || opts.pid <= 0) {
    throw new Error('--pid <pid> is required');
  }
  if (!Number.isFinite(opts.intervalMs) || opts.intervalMs <= 0) {
    throw new Error('--interval-ms must be a positive number');
  }
  if (opts.durationMs !== null && (!Number.isFinite(opts.durationMs) || opts.durationMs <= 0)) {
    throw new Error('--duration-ms must be a positive number');
  }
  return opts;
}

function readOrNull(file) {
  try {
    return fs.readFileSync(file, 'utf8');
  } catch {
    return null;
  }
}

/** VmRSS / VmHWM / Threads from /proc/<pid>/status, in KiB. */
function parseStatus(text) {
  const out = { rssKib: null, vmHwmKib: null, threads: null };
  if (!text) return out;
  for (const line of text.split('\n')) {
    const colon = line.indexOf(':');
    if (colon < 0) continue;
    const key = line.slice(0, colon);
    const value = parseInt(line.slice(colon + 1).trim(), 10);
    if (Number.isNaN(value)) continue;
    if (key === 'VmRSS') out.rssKib = value;
    else if (key === 'VmHWM') out.vmHwmKib = value;
    else if (key === 'Threads') out.threads = value;
  }
  return out;
}

/** Pss and Private_{Clean,Dirty} from /proc/<pid>/smaps_rollup, in KiB. */
function parseSmapsRollup(text) {
  const out = { pssKib: null, privateKib: null };
  if (!text) return out;
  let privateClean = null;
  let privateDirty = null;
  for (const line of text.split('\n')) {
    const colon = line.indexOf(':');
    if (colon < 0) continue;
    const key = line.slice(0, colon);
    const value = parseInt(line.slice(colon + 1).trim(), 10);
    if (Number.isNaN(value)) continue;
    if (key === 'Pss') out.pssKib = value;
    else if (key === 'Private_Clean') privateClean = value;
    else if (key === 'Private_Dirty') privateDirty = value;
  }
  if (privateClean !== null || privateDirty !== null) {
    out.privateKib = (privateClean ?? 0) + (privateDirty ?? 0);
  }
  return out;
}

/**
 * utime/stime from /proc/<pid>/stat, converted to ms.
 *
 * The comm field (index 1) can itself contain spaces and parentheses, so the
 * fields are located relative to the LAST ')' rather than by splitting the
 * whole line — the standard way to parse this file safely.
 */
function parseStatCpu(text) {
  const out = { cpuUserMs: null, cpuSystemMs: null };
  if (!text) return out;
  const close = text.lastIndexOf(')');
  if (close < 0) return out;
  const rest = text.slice(close + 2).trim().split(/\s+/);
  // After comm and state, utime is field 14 and stime field 15 (1-indexed in
  // proc(5)); with the first two fields removed they land at rest[11]/rest[12].
  const utime = parseInt(rest[11], 10);
  const stime = parseInt(rest[12], 10);
  if (!Number.isNaN(utime)) out.cpuUserMs = (utime / CLOCK_TICKS_PER_SEC) * 1000;
  if (!Number.isNaN(stime)) out.cpuSystemMs = (stime / CLOCK_TICKS_PER_SEC) * 1000;
  return out;
}

/** ppid from /proc/<pid>/stat, using the same last-')' rule. */
function parseStatPpid(text) {
  if (!text) return null;
  const close = text.lastIndexOf(')');
  if (close < 0) return null;
  const rest = text.slice(close + 2).trim().split(/\s+/);
  const ppid = parseInt(rest[1], 10);
  return Number.isNaN(ppid) ? null : ppid;
}

function countOpenFds(pid) {
  try {
    return fs.readdirSync(`/proc/${pid}/fd`).length;
  } catch {
    return null;
  }
}

/**
 * Sum RSS across the process and every descendant.
 *
 * Scans all of /proc to build the parent map. That is the only way to find
 * grandchildren, and at a 250 ms cadence the cost is negligible next to the
 * workload — but it is why --tree is opt-in rather than always on.
 */
function sampleTree(rootPid) {
  let entries;
  try {
    entries = fs.readdirSync('/proc');
  } catch {
    return { treeRssKib: null, children: null };
  }

  const byParent = new Map();
  const rssByPid = new Map();
  for (const entry of entries) {
    const pid = Number(entry);
    if (!Number.isInteger(pid) || pid <= 0) continue;
    const stat = readOrNull(path.join('/proc', entry, 'stat'));
    if (!stat) continue;
    const ppid = parseStatPpid(stat);
    if (ppid === null) continue;
    if (!byParent.has(ppid)) byParent.set(ppid, []);
    byParent.get(ppid).push(pid);
    const status = parseStatus(readOrNull(path.join('/proc', entry, 'status')));
    if (status.rssKib !== null) rssByPid.set(pid, status.rssKib);
  }

  let total = rssByPid.get(rootPid) ?? 0;
  let children = 0;
  const queue = [rootPid];
  const seen = new Set([rootPid]);
  while (queue.length > 0) {
    const pid = queue.pop();
    for (const child of byParent.get(pid) ?? []) {
      if (seen.has(child)) continue; // guard against a cycle from pid reuse
      seen.add(child);
      children += 1;
      total += rssByPid.get(child) ?? 0;
      queue.push(child);
    }
  }
  return { treeRssKib: total, children };
}

function sampleOnce(pid, withTree) {
  const statusText = readOrNull(`/proc/${pid}/status`);
  if (statusText === null) return null; // process is gone
  const status = parseStatus(statusText);
  const smaps = parseSmapsRollup(readOrNull(`/proc/${pid}/smaps_rollup`));
  const cpu = parseStatCpu(readOrNull(`/proc/${pid}/stat`));
  const sample = {
    ...status,
    ...smaps,
    ...cpu,
    openFds: countOpenFds(pid),
  };
  if (withTree) Object.assign(sample, sampleTree(pid));
  return sample;
}

const opts = parseArgs(process.argv);
const startedAt = Date.now();

// Fail fast rather than emitting an empty series that the analyzer would have
// to interpret.
if (sampleOnce(opts.pid, false) === null) {
  process.stderr.write(`[sampler] pid ${opts.pid} is not readable in /proc\n`);
  process.exit(1);
}

let stopping = false;
const stop = () => {
  if (stopping) return;
  stopping = true;
  clearInterval(timer);
  process.exit(0);
};

const timer = setInterval(() => {
  const sample = sampleOnce(opts.pid, opts.tree);
  if (sample === null) {
    process.stderr.write(`[sampler] pid ${opts.pid} exited; stopping\n`);
    stop();
    return;
  }
  const now = Date.now();
  // epochMs as well as tMs: the analyzer aligns this series against the
  // driver's turn log, which has its own time origin.
  process.stdout.write(
    `${JSON.stringify({ tMs: now - startedAt, epochMs: now, ...sample })}\n`,
  );
  if (opts.durationMs !== null && Date.now() - startedAt >= opts.durationMs) stop();
}, opts.intervalMs);

for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, stop);
