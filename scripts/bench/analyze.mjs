#!/usr/bin/env node
/**
 * Leak / drift analysis for the agent-scale benchmark tier.
 *
 * Consumes the sampler's JSONL series plus the driver's summary and turn log,
 * and produces a verdict per resource. Kept separate from the load run so a
 * recorded run can be re-analyzed with different thresholds without paying for
 * the load again.
 *
 * WHAT A VERDICT HERE DOES AND DOES NOT MEAN
 *
 * A rising RSS line is not proof of a leak, and a flat one is not proof of its
 * absence. Allocators return memory to the OS lazily, arenas grow to a working
 * set and stop, and caches are supposed to fill. So the RSS check does not rest
 * on slope alone — it requires BOTH a positive slope AND a final window that
 * sits meaningfully above the earlier one, i.e. growth that never plateaued.
 * A run that grows and then levels off reports `plateau`, which is the healthy
 * shape for a process with caches.
 *
 * Thread and file-descriptor counts are held to a much stricter standard,
 * because unlike memory they have no legitimate reason to climb without bound
 * under steady load. A monotonic rise in either is a leak with far less room
 * for interpretation, which is why they are checked independently rather than
 * folded into a single score.
 *
 * In `per-worker` and `shared` thread modes conversation history genuinely
 * accumulates, so RSS growth is EXPECTED. The analyzer will not call a leak in
 * those modes; it reports the growth rate and says the mode cannot distinguish
 * a leak from intended retention. Use `fresh` for a leak verdict.
 *
 * `fresh` is necessary but NOT sufficient. It removes accumulating conversation
 * history; it does not stop the agent persisting memory chunks and embeddings
 * every turn, which a real run does to the tune of gigabytes. An index over data
 * that genuinely grew is not a leak. So when RSS fails alongside large workspace
 * growth, the verdict is marked `confounded` and says so, rather than asserting
 * a leak it cannot distinguish from correct behaviour.
 *
 * Usage:
 *   node scripts/bench/analyze.mjs --samples samples.jsonl --driver summary.json \
 *     [--out report.json] [--rss-kib-per-turn 8] [--warmup-frac 0.25]
 */

import fs from 'node:fs';

function parseArgs(argv) {
  const opts = {
    samples: null,
    driver: null,
    turns: null,
    workspaceMibBefore: null,
    workspaceMibAfter: null,
    out: null,
    // Growth budget per turn before RSS growth is called a leak. Default is
    // deliberately loose; tighten it per scenario once a baseline exists.
    rssKibPerTurn: 8,
    // Fraction of the series treated as warm-up and excluded. First-touch
    // initialization dominates early samples and would fake a steep slope.
    warmupFrac: 0.25,
    // Threads/FDs may drift a little with pool churn; require real growth.
    maxThreadGrowth: 8,
    maxFdGrowth: 32,
  };
  const spec = {
    '--samples': ['samples', String],
    '--driver': ['driver', String],
    '--turns': ['turns', String],
    '--workspace-mib-before': ['workspaceMibBefore', Number],
    '--workspace-mib-after': ['workspaceMibAfter', Number],
    '--out': ['out', String],
    '--rss-kib-per-turn': ['rssKibPerTurn', Number],
    '--warmup-frac': ['warmupFrac', Number],
    '--max-thread-growth': ['maxThreadGrowth', Number],
    '--max-fd-growth': ['maxFdGrowth', Number],
  };
  for (let i = 2; i < argv.length; i += 1) {
    const entry = spec[argv[i]];
    if (!entry) throw new Error(`unknown argument: ${argv[i]}`);
    const [key, cast] = entry;
    const raw = argv[++i];
    const value = cast(raw);
    if (cast === Number && !Number.isFinite(value)) {
      throw new Error(`${argv[i - 1]} expects a number, got: ${raw}`);
    }
    opts[key] = value;
  }
  if (!opts.samples) throw new Error('--samples <file.jsonl> is required');
  if (opts.warmupFrac < 0 || opts.warmupFrac >= 1) {
    throw new Error('--warmup-frac must be in [0, 1)');
  }
  return opts;
}

function readJsonl(file) {
  let dropped = 0;
  return fs
    .readFileSync(file, 'utf8')
    .split('\n')
    .filter(line => line.trim().length > 0)
    .flatMap(line => {
      try {
        return [JSON.parse(line)];
      } catch {
        dropped += 1;
        if (dropped === 1) {
          process.stderr.write(`[analyze] skipping unparsable line in ${file}\n`);
        }
        return [];
      }
    });
}

/** Ordinary least squares. Returns slope in units of y per unit of x. */
function linearFit(xs, ys) {
  const n = xs.length;
  if (n < 2) return { slope: null, intercept: null, r2: null };
  const meanX = xs.reduce((a, b) => a + b, 0) / n;
  const meanY = ys.reduce((a, b) => a + b, 0) / n;
  let sxy = 0;
  let sxx = 0;
  for (let i = 0; i < n; i += 1) {
    sxy += (xs[i] - meanX) * (ys[i] - meanY);
    sxx += (xs[i] - meanX) ** 2;
  }
  if (sxx === 0) return { slope: null, intercept: null, r2: null };
  const slope = sxy / sxx;
  const intercept = meanY - slope * meanX;

  let ssRes = 0;
  let ssTot = 0;
  for (let i = 0; i < n; i += 1) {
    ssRes += (ys[i] - (slope * xs[i] + intercept)) ** 2;
    ssTot += (ys[i] - meanY) ** 2;
  }
  return { slope, intercept, r2: ssTot === 0 ? null : 1 - ssRes / ssTot };
}

const mean = arr => (arr.length ? arr.reduce((a, b) => a + b, 0) / arr.length : null);

function windowMeans(values) {
  if (values.length < 4) return { early: null, late: null };
  const q = Math.max(1, Math.floor(values.length / 4));
  return { early: mean(values.slice(0, q)), late: mean(values.slice(-q)) };
}

const opts = parseArgs(process.argv);
const allSamples = readJsonl(opts.samples);
if (allSamples.length === 0) {
  process.stderr.write('[analyze] sample series is empty\n');
  process.exit(1);
}
const driver = opts.driver ? JSON.parse(fs.readFileSync(opts.driver, 'utf8')) : null;

// Clip to the window in which load was actually being applied.
//
// This is load-bearing, not tidying. The sampler deliberately runs before the
// driver starts and for a few seconds after it stops, so the series has an idle
// head and an idle tail. Analyzing those is not merely noisy — it inverts the
// result: an idle tail is flat and consumes no CPU, so "growth stopped by the
// end" and "CPU rate fell" both become trivially true and every check passes
// regardless of what happened under load. A short run, where the tail is a large
// fraction of the series, would always report a clean bill of health.
//
// The driver records the wall-clock bounds of its measured window; samples carry
// epochMs for exactly this alignment.
let loadSamples = allSamples;
let clippedTail = [];
if (driver?.measureStartedAtMs && driver?.wallMs) {
  const loadStart = driver.measureStartedAtMs;
  const loadEnd = loadStart + driver.wallMs;
  const withEpoch = allSamples.filter(s => typeof s.epochMs === 'number');
  if (withEpoch.length > 0) {
    const inWindow = withEpoch.filter(s => s.epochMs >= loadStart && s.epochMs <= loadEnd);
    // Only trust the clip if it left enough to analyze; otherwise fall back to
    // the full series and say so, rather than failing on a technicality.
    if (inWindow.length >= 8) {
      loadSamples = inWindow;
      clippedTail = withEpoch.filter(s => s.epochMs > loadEnd);
    }
  }
}
const clippedToLoadWindow = loadSamples !== allSamples;

// Drop the warm-up head of what remains.
const skip = Math.floor(loadSamples.length * opts.warmupFrac);
const samples = loadSamples.slice(skip);
if (samples.length < 4) {
  process.stderr.write(
    `[analyze] only ${samples.length} samples after clipping to the load window ` +
      `and dropping ${skip} warm-up samples — run longer, sample more often, or ` +
      `lower --warmup-frac\n`
  );
  process.exit(1);
}

// A leak verdict drawn from a handful of samples over a couple of seconds is
// not worth the confidence the word "pass" conveys. Warn rather than fail: the
// throughput and latency figures are still useful at this size.
const UNDERPOWERED_SAMPLES = 40;
const UNDERPOWERED_MS = 30_000;
const underpowered =
  samples.length < UNDERPOWERED_SAMPLES ||
  samples[samples.length - 1].tMs - samples[0].tMs < UNDERPOWERED_MS;

const durationMs = samples[samples.length - 1].tMs - samples[0].tMs;
const durationMin = durationMs / 60_000;

const threadMode = driver?.config?.threadMode ?? 'unknown';
// Turn count inside the analyzed window, used to express growth per turn. The
// driver's total covers the whole measured run; scaling it by the analyzed
// fraction is an approximation, and is labeled as such in the report.
const turnsTotal = driver?.turnsOk ?? null;
// Scale by the analyzed window only: allSamples includes the idle head before
// the driver starts and the idle tail after it stops, so dividing by it would
// deflate per-turn figures whenever the series was clipped.
const analyzedFrac = loadSamples.length > 0 ? samples.length / loadSamples.length : 1;
const turnsInWindow = turnsTotal !== null ? turnsTotal * analyzedFrac : null;

function seriesFor(field) {
  const xs = [];
  const ys = [];
  for (const s of samples) {
    if (typeof s[field] === 'number' && Number.isFinite(s[field])) {
      xs.push(s.tMs);
      ys.push(s[field]);
    }
  }
  return { xs, ys };
}

function analyzeMemory(field, label) {
  const { xs, ys } = seriesFor(field);
  if (ys.length < 4) return { field, label, available: false };

  const fit = linearFit(xs, ys);

  /** Convert a KiB-per-ms slope into KiB per turn, or null if not derivable. */
  const perTurn = slope =>
    slope !== null && turnsInWindow && turnsInWindow > 0 && durationMs > 0
      ? (slope * durationMs) / turnsInWindow
      : null;

  const kibPerMin = fit.slope === null ? null : fit.slope * 60_000;
  const kibPerTurn = perTurn(fit.slope);

  // Whether growth has STOPPED is a question about the end of the run, not
  // about early-vs-late averages: a series that never grew and one that grew
  // and then flattened have the same early-vs-late delta, but only the second
  // is a plateau. So fit the final third separately and ask whether the line
  // is still climbing there.
  const tailStart = Math.floor(ys.length * (2 / 3));
  const tailFit = linearFit(xs.slice(tailStart), ys.slice(tailStart));
  const tailKibPerTurn = perTurn(tailFit.slope);

  const { early, late } = windowMeans(ys);
  const relativeGrowth = early && early > 0 ? (late - early) / early : null;

  const overBudget = kibPerTurn !== null && kibPerTurn > opts.rssKibPerTurn;
  const tailWithinBudget =
    tailFit.slope !== null &&
    (tailFit.slope <= 0 || (tailKibPerTurn !== null && tailKibPerTurn <= opts.rssKibPerTurn));

  let verdict;
  let reason;
  if (threadMode === 'per-worker' || threadMode === 'shared') {
    verdict = 'not-assessed';
    reason =
      `thread-mode=${threadMode} accumulates conversation history by design, so ` +
      `growth here cannot be distinguished from a leak. Re-run with ` +
      `--thread-mode fresh for a verdict.`;
  } else if (fit.slope === null || fit.slope <= 0) {
    verdict = 'pass';
    reason = 'no positive growth trend in the steady-state window.';
  } else if (!overBudget) {
    verdict = 'pass';
    reason =
      kibPerTurn !== null
        ? `growth ${kibPerTurn.toFixed(2)} KiB/turn is within the ` +
          `${opts.rssKibPerTurn} KiB/turn budget.`
        : 'positive slope but no turn count available to normalize against.';
  } else if (tailWithinBudget) {
    // It grew past the budget overall, but stopped climbing by the end — the
    // shape of a cache filling to its working set rather than an unbounded leak.
    verdict = 'plateau';
    reason =
      `grew ${kibPerTurn.toFixed(2)} KiB/turn overall but leveled off: the final ` +
      `third of the window grew ${tailKibPerTurn === null ? 'not at all' : `${tailKibPerTurn.toFixed(2)} KiB/turn`}` +
      `, within the ${opts.rssKibPerTurn} KiB/turn budget. Consistent with a ` +
      `cache reaching its working set rather than an unbounded leak. Run longer ` +
      `to confirm the plateau holds.`;
  } else {
    verdict = 'fail';
    reason =
      `grew ${kibPerTurn.toFixed(2)} KiB/turn and was still growing ` +
      `${tailKibPerTurn === null ? '' : `${tailKibPerTurn.toFixed(2)} KiB/turn `}` +
      `in the final third, over the ${opts.rssKibPerTurn} KiB/turn budget.`;
  }

  return {
    field,
    label,
    available: true,
    verdict,
    reason,
    firstKib: ys[0],
    lastKib: ys[ys.length - 1],
    minKib: Math.min(...ys),
    maxKib: Math.max(...ys),
    earlyWindowMeanKib: early,
    lateWindowMeanKib: late,
    relativeGrowth,
    kibPerMin,
    kibPerTurn,
    tailKibPerTurn,
    r2: fit.r2,
  };
}

function analyzeCounter(field, label, maxGrowth) {
  const { ys } = seriesFor(field);
  if (ys.length < 4) return { field, label, available: false };
  const first = ys[0];
  const last = ys[ys.length - 1];
  const max = Math.max(...ys);
  const { early, late } = windowMeans(ys);
  const growth = late - early;
  // Threads and FDs have no legitimate reason to climb without bound under
  // steady load, so this is a straight threshold rather than a trend test.
  const verdict = growth > maxGrowth ? 'fail' : 'pass';
  return {
    field,
    label,
    available: true,
    verdict,
    reason:
      verdict === 'fail'
        ? `grew by ${growth.toFixed(1)} between the early and final windows, ` +
          `over the budget of ${maxGrowth}.`
        : `stable (growth ${growth.toFixed(1)}, budget ${maxGrowth}).`,
    first,
    last,
    max,
    earlyWindowMean: early,
    lateWindowMean: late,
    growth,
  };
}

/**
 * CPU drift: does a turn cost more CPU at the end of the run than at the start?
 *
 * CPU counters are cumulative, so the meaningful quantity is the DERIVATIVE —
 * CPU ms consumed per wall ms — compared between the early and final windows.
 * A rising figure under constant offered load means per-turn work is growing,
 * which is the CPU analogue of a memory leak (an unbounded list being rescanned
 * every turn, say).
 */
function analyzeCpu() {
  const usable = samples.filter(
    s => typeof s.cpuUserMs === 'number' && typeof s.cpuSystemMs === 'number'
  );
  if (usable.length < 8) return { available: false };

  const totals = usable.map(s => s.cpuUserMs + s.cpuSystemMs);
  const times = usable.map(s => s.tMs);

  const rates = [];
  for (let i = 1; i < usable.length; i += 1) {
    const dt = times[i] - times[i - 1];
    if (dt <= 0) continue;
    rates.push({ tMs: times[i], rate: (totals[i] - totals[i - 1]) / dt });
  }
  if (rates.length < 4) return { available: false };

  const rateValues = rates.map(r => r.rate);
  const { early, late } = windowMeans(rateValues);
  const drift = early && early > 0 ? (late - early) / early : null;

  const totalCpuMs = totals[totals.length - 1] - totals[0];
  const cpuMsPerTurn = turnsInWindow && turnsInWindow > 0 ? totalCpuMs / turnsInWindow : null;

  // 25% more CPU per unit time for the same offered load is a real drift, not
  // scheduling noise.
  const verdict = drift !== null && drift > 0.25 ? 'fail' : 'pass';
  return {
    available: true,
    verdict,
    reason:
      verdict === 'fail'
        ? `CPU rate rose ${(drift * 100).toFixed(1)}% from the early to the final ` +
          `window under constant offered load — per-turn work is growing.`
        : drift === null
          ? 'no early-window baseline to compare against.'
          : `CPU rate change ${(drift * 100).toFixed(1)}% is within noise.`,
    totalCpuMs,
    cpuMsPerTurn,
    earlyRateCpuMsPerMs: early,
    lateRateCpuMsPerMs: late,
    drift,
    meanUtilizationCores: durationMs > 0 ? totalCpuMs / durationMs : null,
  };
}

const memory = [
  analyzeMemory('rssKib', 'RSS'),
  analyzeMemory('pssKib', 'PSS'),
  analyzeMemory('privateKib', 'Private (clean+dirty)'),
];
if (samples.some(s => typeof s.treeRssKib === 'number')) {
  memory.push(analyzeMemory('treeRssKib', 'RSS incl. descendants'));
}

const threads = analyzeCounter('threads', 'Threads', opts.maxThreadGrowth);
const fds = analyzeCounter('openFds', 'Open FDs', opts.maxFdGrowth);
const cpu = analyzeCpu();

const throughput = opts.turns ? analyzeThroughput(opts.turns) : { available: false };

/**
 * On-disk state the run produced.
 *
 * This is the caveat to the memory verdict, not decoration. `fresh` thread mode
 * removes accumulating CONVERSATION history, but it does not stop the agent
 * persisting memory chunks and embeddings on every turn — a run can leave
 * gigabytes behind. An index over data that genuinely grew is not a leak, so a
 * failing RSS curve that arrives alongside large workspace growth is ambiguous
 * and has to be reported that way.
 */
const workspace =
  opts.workspaceMibBefore !== null && opts.workspaceMibAfter !== null
    ? {
        available: true,
        beforeMib: opts.workspaceMibBefore,
        afterMib: opts.workspaceMibAfter,
        growthMib: opts.workspaceMibAfter - opts.workspaceMibBefore,
      }
    : { available: false };

// Enough on-disk growth that an index over it could plausibly account for a
// rising RSS curve on its own.
const WORKSPACE_CONFOUND_MIB = 100;
const memoryConfounded = workspace.available && workspace.growthMib >= WORKSPACE_CONFOUND_MIB;

const checks = [...memory.filter(m => m.available), threads, fds, cpu, throughput].filter(
  c => c.available !== false
);
const failed = checks.filter(c => c.verdict === 'fail');

// If the core STOPPED, every resource check is describing an idle process and
// none of their verdicts mean what they appear to. Degradation is different:
// the core was still working, just less, so the resource numbers remain real
// even though the run is unhealthy. Conflating the two would put "the core
// stopped serving" on a report where it plainly kept serving.
const livenessBroken = throughput.available && throughput.stopped === true;
// PSS and Private track RSS closely; the headline verdict keys off RSS, with
// the others reported for corroboration.
const overall = failed.length > 0 ? 'fail' : 'pass';

/**
 * What happened after load stopped.
 *
 * Not a pass/fail check — it is an observation that helps interpret the memory
 * verdict. Memory released once work stops was working-set, not retained; memory
 * still held is the part a leak would live in. CPU that keeps burning after the
 * last turn is its own finding.
 */
function analyzeSettle() {
  if (clippedTail.length < 2) return { available: false };
  const rss = clippedTail.map(s => s.rssKib).filter(v => typeof v === 'number');
  if (rss.length < 2) return { available: false };

  const lastUnderLoad = samples[samples.length - 1];
  const releasedKib = (lastUnderLoad.rssKib ?? rss[0]) - rss[rss.length - 1];

  const cpuFirst = clippedTail[0];
  const cpuLast = clippedTail[clippedTail.length - 1];
  const idleMs = cpuLast.tMs - cpuFirst.tMs;
  const idleCpuMs =
    typeof cpuFirst.cpuUserMs === 'number' && typeof cpuLast.cpuUserMs === 'number'
      ? cpuLast.cpuUserMs + cpuLast.cpuSystemMs - (cpuFirst.cpuUserMs + cpuFirst.cpuSystemMs)
      : null;

  return {
    available: true,
    tailSamples: clippedTail.length,
    idleWindowMs: idleMs,
    rssAtLoadEndKib: lastUnderLoad.rssKib ?? null,
    rssAfterSettleKib: rss[rss.length - 1],
    releasedKib,
    idleCpuMs,
    idleCpuFraction: idleMs > 0 && idleCpuMs !== null ? idleCpuMs / idleMs : null,
  };
}

const settle = analyzeSettle();

/**
 * Did the core keep serving at a steady rate for the whole run?
 *
 * This check exists because the analyzer once reported a confident PASS on a
 * run where the core had stopped answering entirely two thirds of the way in.
 * Every other check agreed: memory was flat, CPU had fallen, threads were
 * stable — all true, and all because nothing was happening. A dead process is
 * indistinguishable from a healthy idle one on resource metrics alone, so
 * liveness has to be judged on whether work was actually completing.
 *
 * Reads the driver's per-turn log rather than the resource series, since that
 * is the only record of when turns completed.
 */
function analyzeThroughput(turnsPath) {
  let turns;
  try {
    turns = readJsonl(turnsPath);
  } catch {
    return { available: false };
  }
  if (driver?.measureStartedAtMs && driver?.wallMs) {
    const loadStart = driver.measureStartedAtMs;
    const loadEnd = loadStart + driver.wallMs;
    const withEpoch = turns.filter(t => typeof t.epochMs === 'number');
    if (withEpoch.length > 0) {
      turns = withEpoch
        .filter(t => t.epochMs >= loadStart && t.epochMs <= loadEnd)
        .map(t => ({ ...t, tMs: t.epochMs - loadStart }));
    }
  }
  if (turns.length < 20) return { available: false };

  const okTurns = turns.filter(t => t.ok);
  if (okTurns.length < 20) return { available: false };

  // The window must be the run's actual duration, NOT the timestamp of the last
  // logged turn. If the core stops answering and the driver's in-flight requests
  // hang or stop being recorded, the log simply ends early — and measuring
  // against its own last entry would rescale the window to fit the period when
  // things still worked, hiding exactly the outage this check exists to catch.
  const runMs = driver?.wallMs ?? turns[turns.length - 1].tMs;
  const quarter = runMs / 4;
  const firstQuarter = okTurns.filter(t => t.tMs < quarter).length;
  const lastQuarter = okTurns.filter(t => t.tMs >= 3 * quarter).length;

  const firstRate = firstQuarter / (quarter / 1000);
  const lastRate = lastQuarter / (quarter / 1000);
  const retained = firstRate > 0 ? lastRate / firstRate : null;

  let verdict;
  let reason;
  let stopped = false;
  if (lastQuarter === 0) {
    verdict = 'fail';
    stopped = true;
    reason =
      `the core completed NO successful turns in the final quarter of the run ` +
      `(${firstQuarter} in the first quarter). It stopped serving — every other ` +
      `check in this report describes an idle process, not a healthy one.`;
  } else if (retained !== null && retained < 0.5) {
    verdict = 'fail';
    reason =
      `throughput fell to ${(retained * 100).toFixed(0)}% of its starting rate ` +
      `(${firstRate.toFixed(1)} → ${lastRate.toFixed(1)} turns/s) under constant ` +
      `offered load — the core is degrading as the run proceeds.`;
  } else {
    verdict = 'pass';
    reason =
      retained === null
        ? 'no starting rate to compare against.'
        : `held ${(retained * 100).toFixed(0)}% of its starting rate ` +
          `(${firstRate.toFixed(1)} → ${lastRate.toFixed(1)} turns/s).`;
  }
  return {
    available: true,
    verdict,
    reason,
    stopped,
    firstQuarterTurnsPerSec: firstRate,
    lastQuarterTurnsPerSec: lastRate,
    retainedFraction: retained,
  };
}

const report = {
  overall,
  threadMode,
  underpowered,
  underpoweredNote: underpowered
    ? `analyzed ${samples.length} samples over ${(durationMs / 1000).toFixed(1)}s. ` +
      `A leak verdict from a window this short is weak — growth and plateau are ` +
      `hard to separate over seconds. Prefer --duration-ms 900000 or more for a ` +
      `run you intend to act on.`
    : null,
  window: {
    totalSamples: allSamples.length,
    clippedToLoadWindow,
    loadWindowSamples: loadSamples.length,
    warmupSamplesDropped: skip,
    analyzedSamples: samples.length,
    durationMs,
    turnsTotal,
    turnsInWindowApprox: turnsInWindow,
  },
  settle,
  driverSummary: driver
    ? {
        turnsOk: driver.turnsOk,
        turnsFailed: driver.turnsFailed,
        throughputTurnsPerSec: driver.throughputTurnsPerSec,
        latencyMs: driver.latencyMs,
        errors: driver.errors,
      }
    : null,
  memory: memory.map(m =>
    m.available && m.verdict === 'fail' && memoryConfounded
      ? {
          ...m,
          confounded: true,
          confoundNote:
            `the workspace grew ${workspace.growthMib} MiB during this run, so some ` +
            `of this RSS growth is likely an index over data that genuinely ` +
            `accumulated rather than a leak. To separate the two, re-run with ` +
            `memory capture disabled, or run long enough that on-disk growth ` +
            `levels off while RSS does not.`,
        }
      : m
  ),
  workspace,
  threads,
  fds,
  cpu,
  throughput,
  livenessBroken,
  livenessNote: livenessBroken
    ? 'The core stopped serving during this run, so the memory, CPU, thread and ' +
      'FD verdicts above describe an idle process and must not be read as a ' +
      'clean bill of health. Fix the liveness failure and re-run.'
    : null,
  failedChecks: failed.map(c => `${c.label ?? 'check'}: ${c.reason}`),
};

const rendered = JSON.stringify(report, null, 2);
if (opts.out) fs.writeFileSync(opts.out, rendered);

// Human summary to stderr so stdout stays machine-readable.
const lines = [];
lines.push('');
lines.push(`  agent-scale benchmark — ${overall.toUpperCase()}`);
lines.push(
  `  thread-mode=${threadMode}  samples=${samples.length}/${allSamples.length}` +
    `  window=${(durationMs / 1000).toFixed(1)}s` +
    `${clippedToLoadWindow ? ' (clipped to load)' : ''}`
);
if (underpowered) lines.push(`  WEAK EVIDENCE: ${report.underpoweredNote}`);
if (driver) {
  lines.push(
    `  turns: ${driver.turnsOk} ok / ${driver.turnsFailed} failed` +
      `  throughput=${driver.throughputTurnsPerSec?.toFixed(2)}/s` +
      `  p50=${driver.latencyMs?.p50?.toFixed(0)}ms p99=${driver.latencyMs?.p99?.toFixed(0)}ms`
  );
}
lines.push('');
for (const m of memory) {
  if (!m.available) continue;
  lines.push(
    `  ${m.verdict.padEnd(12)} ${m.label.padEnd(24)} ` +
      `${(m.firstKib / 1024).toFixed(1)} → ${(m.lastKib / 1024).toFixed(1)} MiB` +
      (m.kibPerTurn !== null ? `  (${m.kibPerTurn.toFixed(2)} KiB/turn)` : '')
  );
  lines.push(`               ${m.reason}`);
  if (m.verdict === 'fail' && memoryConfounded) {
    lines.push(
      `               CAVEAT: the workspace grew ${workspace.growthMib} MiB — some of ` +
        `this is likely index over real data, not a leak.`
    );
  }
}
for (const c of [threads, fds]) {
  if (!c.available) continue;
  lines.push(
    `  ${c.verdict.padEnd(12)} ${c.label.padEnd(24)} ${c.first} → ${c.last} (max ${c.max})`
  );
  lines.push(`               ${c.reason}`);
}
if (cpu.available) {
  lines.push(
    `  ${cpu.verdict.padEnd(12)} ${'CPU'.padEnd(24)} ` +
      `${cpu.meanUtilizationCores?.toFixed(2)} cores mean` +
      (cpu.cpuMsPerTurn !== null ? `, ${cpu.cpuMsPerTurn.toFixed(1)} ms/turn` : '')
  );
  lines.push(`               ${cpu.reason}`);
}
if (throughput.available) {
  lines.push(
    `  ${throughput.verdict.padEnd(12)} ${'Throughput held'.padEnd(24)} ` +
      `${throughput.firstQuarterTurnsPerSec.toFixed(1)} → ` +
      `${throughput.lastQuarterTurnsPerSec.toFixed(1)} turns/s`
  );
  lines.push(`               ${throughput.reason}`);
}
if (livenessBroken) {
  lines.push('');
  lines.push(`  !! ${report.livenessNote}`);
}
if (settle.available) {
  lines.push('');
  lines.push(
    `  after load (${(settle.idleWindowMs / 1000).toFixed(1)}s idle): ` +
      `released ${(settle.releasedKib / 1024).toFixed(1)} MiB, ` +
      `idle CPU ${settle.idleCpuFraction === null ? 'n/a' : `${(settle.idleCpuFraction * 100).toFixed(1)}% of one core`}`
  );
}
lines.push('');
process.stderr.write(`${lines.join('\n')}\n`);
process.stdout.write(`${rendered}\n`);

process.exit(overall === 'fail' ? 1 : 0);
