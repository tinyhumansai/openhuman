#!/usr/bin/env node --test
/**
 * Tests for the agent-scale leak analyzer.
 *
 * The analyzer is the component whose failure mode is silence: if the math is
 * wrong it reports "pass" on a leaking run and nobody notices, which is worse
 * than not having the check at all. So the cases below drive it with synthetic
 * series whose correct verdict is known by construction — a steady leak, a
 * plateau, a flat line, thread and FD growth, and CPU drift.
 *
 * Run: node --test scripts/bench/analyze.test.mjs
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ANALYZE = path.join(HERE, 'analyze.mjs');

const INTERVAL_MS = 250;

/**
 * Build a sample series.
 *
 * @param {object} opts
 * @param {number} opts.count      number of samples
 * @param {(i: number, n: number) => number} opts.rssKib
 * @param {(i: number, n: number) => number} [opts.threads]
 * @param {(i: number, n: number) => number} [opts.openFds]
 * @param {(i: number, n: number) => number} [opts.cpuMs] cumulative total CPU
 */
function buildSamples({ count, rssKib, threads, openFds, cpuMs }) {
  const lines = [];
  for (let i = 0; i < count; i += 1) {
    const tMs = i * INTERVAL_MS;
    const total = cpuMs ? cpuMs(i, count) : i * 50;
    lines.push(
      JSON.stringify({
        tMs,
        epochMs: 1_700_000_000_000 + tMs,
        rssKib: Math.round(rssKib(i, count)),
        vmHwmKib: Math.round(rssKib(i, count)),
        pssKib: null,
        privateKib: null,
        // Split the cumulative total across user/system; the analyzer sums them.
        cpuUserMs: total * 0.8,
        cpuSystemMs: total * 0.2,
        threads: threads ? Math.round(threads(i, count)) : 24,
        openFds: openFds ? Math.round(openFds(i, count)) : 40,
      }),
    );
  }
  return `${lines.join('\n')}\n`;
}

/**
 * Build a turn log at a given rate over time.
 * @param {(tMs: number) => number} ratePerSec turns/sec at a point in the run
 */
function buildTurns(durationMs, ratePerSec, { failFrom = null } = {}) {
  const lines = [];
  const stepMs = 100;
  for (let tMs = 0; tMs < durationMs; tMs += stepMs) {
    const n = Math.round((ratePerSec(tMs) * stepMs) / 1000);
    for (let k = 0; k < n; k += 1) {
      const ok = failFrom === null || tMs < failFrom;
      lines.push(JSON.stringify({ tMs, workerId: 0, index: lines.length, latencyMs: 50, ok }));
    }
  }
  return `${lines.join('\n')}\n`;
}

function runAnalyzer(samplesText, driverSummary, extraArgs = [], turnsText = null) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'bench-analyze-'));
  try {
    const samplesPath = path.join(dir, 'samples.jsonl');
    const driverPath = path.join(dir, 'driver.json');
    fs.writeFileSync(samplesPath, samplesText);
    fs.writeFileSync(driverPath, JSON.stringify(driverSummary));

    const args = [ANALYZE, '--samples', samplesPath, '--driver', driverPath, ...extraArgs];
    if (turnsText !== null) {
      const turnsPath = path.join(dir, 'turns.jsonl');
      fs.writeFileSync(turnsPath, turnsText);
      args.push('--turns', turnsPath);
    }
    let stdout;
    let exitCode = 0;
    try {
      stdout = execFileSync(process.execPath, args, { encoding: 'utf8', stdio: 'pipe' });
    } catch (err) {
      // A failing verdict is a non-zero exit, which execFileSync throws on.
      // That is an expected outcome here, not an error.
      exitCode = err.status ?? 1;
      stdout = err.stdout ?? '';
    }
    return { report: JSON.parse(stdout), exitCode };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const driver = (overrides = {}) => ({
  config: { concurrency: 8, turns: 400, threadMode: 'fresh', warmupTurns: 10 },
  // The measured run lasted 50s — the same span the synthetic turn logs cover.
  // The throughput check keys off this rather than the last logged turn.
  wallMs: 50_000,
  turnsOk: 400,
  turnsFailed: 0,
  throughputTurnsPerSec: 10,
  latencyMs: { p50: 80, p99: 200 },
  errors: {},
  ...overrides,
});

test('steady unbounded RSS growth is reported as a leak', () => {
  // 200 samples at 250ms = 50s, growing 400 KiB/sample = a relentless climb.
  const samples = buildSamples({ count: 200, rssKib: (i) => 120_000 + i * 400 });
  const { report, exitCode } = runAnalyzer(samples, driver());

  const rss = report.memory.find((m) => m.field === 'rssKib');
  assert.equal(rss.verdict, 'fail', rss.reason);
  assert.ok(rss.kibPerTurn > 0, 'should attribute growth per turn');
  assert.equal(report.overall, 'fail');
  assert.equal(exitCode, 1, 'a failing verdict must exit non-zero');
});

test('growth that levels off is reported as a plateau, not a leak', () => {
  // Climbs hard through the first part of the ANALYZED window (which starts at
  // sample 50, after the warm-up head is dropped) and then stops dead at 130.
  // Overall slope is well over budget, but the final third is flat — the shape
  // of a cache filling to its working set. Distinguishing this from a leak is
  // the whole point of fitting the tail separately.
  const samples = buildSamples({
    count: 200,
    rssKib: (i) => 120_000 + Math.min(i, 130) * 400,
  });
  const { report, exitCode } = runAnalyzer(samples, driver());

  const rss = report.memory.find((m) => m.field === 'rssKib');
  assert.equal(rss.verdict, 'plateau', rss.reason);
  assert.ok(rss.kibPerTurn > rss.tailKibPerTurn, 'tail should grow slower than overall');
  assert.equal(report.overall, 'pass');
  assert.equal(exitCode, 0);
});

test('a flat RSS series passes', () => {
  // Small oscillation around a fixed level, no trend.
  const samples = buildSamples({
    count: 200,
    rssKib: (i) => 120_000 + Math.sin(i / 5) * 500,
  });
  const { report } = runAnalyzer(samples, driver());

  const rss = report.memory.find((m) => m.field === 'rssKib');
  assert.equal(rss.verdict, 'pass', rss.reason);
  assert.equal(report.overall, 'pass');
});

test('accumulating thread modes are not assessed for leaks', () => {
  // Same leaking series as the first test, but in a mode where growth is
  // expected. The analyzer must decline to call it rather than raise a false
  // alarm on conversation history.
  const samples = buildSamples({ count: 200, rssKib: (i) => 120_000 + i * 400 });
  const { report, exitCode } = runAnalyzer(
    samples,
    driver({ config: { threadMode: 'per-worker' } }),
  );

  const rss = report.memory.find((m) => m.field === 'rssKib');
  assert.equal(rss.verdict, 'not-assessed');
  assert.match(rss.reason, /--thread-mode fresh/);
  assert.equal(exitCode, 0, 'declining to assess is not a failure');
});

test('thread growth fails independently of memory', () => {
  const samples = buildSamples({
    count: 200,
    rssKib: () => 120_000, // memory perfectly flat
    threads: (i) => 24 + i * 0.5, // but threads climb
  });
  const { report, exitCode } = runAnalyzer(samples, driver());

  assert.equal(report.memory.find((m) => m.field === 'rssKib').verdict, 'pass');
  assert.equal(report.threads.verdict, 'fail', report.threads.reason);
  assert.equal(report.overall, 'fail');
  assert.equal(exitCode, 1);
});

test('file-descriptor growth fails independently of memory', () => {
  const samples = buildSamples({
    count: 200,
    rssKib: () => 120_000,
    openFds: (i) => 40 + i * 2,
  });
  const { report } = runAnalyzer(samples, driver());

  assert.equal(report.fds.verdict, 'fail', report.fds.reason);
  assert.equal(report.overall, 'fail');
});

test('stable threads and fds pass', () => {
  const samples = buildSamples({
    count: 200,
    rssKib: () => 120_000,
    threads: () => 24,
    openFds: (i) => 40 + (i % 3), // churn, but no trend
  });
  const { report } = runAnalyzer(samples, driver());

  assert.equal(report.threads.verdict, 'pass');
  assert.equal(report.fds.verdict, 'pass');
  assert.equal(report.overall, 'pass');
});

test('rising CPU cost per unit time is reported as drift', () => {
  // Quadratic cumulative CPU means a linearly rising rate: the same offered
  // load costing steadily more, which is the CPU analogue of a leak.
  const samples = buildSamples({
    count: 200,
    rssKib: () => 120_000,
    cpuMs: (i) => 0.02 * i * i,
  });
  const { report, exitCode } = runAnalyzer(samples, driver());

  assert.equal(report.cpu.verdict, 'fail', report.cpu.reason);
  assert.ok(report.cpu.drift > 0.25);
  assert.equal(exitCode, 1);
});

test('constant CPU rate passes and reports per-turn cost', () => {
  const samples = buildSamples({
    count: 200,
    rssKib: () => 120_000,
    cpuMs: (i) => i * 100, // steady 100ms CPU per 250ms wall
  });
  const { report } = runAnalyzer(samples, driver());

  assert.equal(report.cpu.verdict, 'pass', report.cpu.reason);
  assert.ok(report.cpu.cpuMsPerTurn > 0);
  assert.ok(report.cpu.meanUtilizationCores > 0);
});

test('warm-up samples are excluded from the analyzed window', () => {
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const { report } = runAnalyzer(samples, driver(), ['--warmup-frac', '0.5']);

  assert.equal(report.window.totalSamples, 200);
  assert.equal(report.window.warmupSamplesDropped, 100);
  assert.equal(report.window.analyzedSamples, 100);
});

test('a looser per-turn budget can accept growth a tight one rejects', () => {
  const samples = buildSamples({ count: 200, rssKib: (i) => 120_000 + i * 400 });

  const tight = runAnalyzer(samples, driver(), ['--rss-kib-per-turn', '1']);
  assert.equal(tight.report.memory.find((m) => m.field === 'rssKib').verdict, 'fail');

  const loose = runAnalyzer(samples, driver(), ['--rss-kib-per-turn', '100000']);
  assert.equal(loose.report.memory.find((m) => m.field === 'rssKib').verdict, 'pass');
});

const EPOCH0 = 1_700_000_000_000;

test('an idle tail after load does not mask a leak', () => {
  // The regression this guards: the sampler runs past the end of the load, so
  // the series ends with an idle stretch that is flat and consumes no CPU.
  // Analyzed naively, that tail makes "growth stopped" and "CPU fell" trivially
  // true and EVERY run passes — the shorter the run, the more certain the false
  // clean bill of health. Here memory climbs relentlessly for the whole load and
  // then goes idle; the verdict must still be a leak.
  const LOAD_SAMPLES = 150;
  const samples = buildSamples({
    count: 200,
    rssKib: (i) => 120_000 + Math.min(i, LOAD_SAMPLES) * 400,
    // CPU accrues under load, then stops entirely.
    cpuMs: (i) => Math.min(i, LOAD_SAMPLES) * 100,
  });
  const withWindow = driver({
    measureStartedAtMs: EPOCH0,
    wallMs: LOAD_SAMPLES * INTERVAL_MS,
  });

  const { report, exitCode } = runAnalyzer(samples, withWindow);

  assert.equal(report.window.clippedToLoadWindow, true, 'must clip to the load window');
  assert.ok(
    report.window.analyzedSamples < 200,
    'the idle tail must be excluded from the analyzed window',
  );
  const rss = report.memory.find((m) => m.field === 'rssKib');
  const expectedKibPerTurn =
    (400 * (report.window.analyzedSamples - 1)) / report.window.turnsInWindowApprox;
  assert.ok(Math.abs(rss.kibPerTurn - expectedKibPerTurn) < 1e-9);
  assert.equal(rss.verdict, 'fail');
  assert.equal(report.overall, 'fail');
  assert.equal(exitCode, 1);
});

test('a truncated final sample record is skipped without losing valid samples', () => {
  const valid = buildSamples({ count: 200, rssKib: () => 120_000 });
  const { report, exitCode } = runAnalyzer(`${valid}{"tMs":`, driver());

  assert.equal(report.window.totalSamples, 200);
  assert.equal(report.overall, 'pass');
  assert.equal(exitCode, 0);
});

test('the settle tail is reported separately from the verdict', () => {
  const LOAD_SAMPLES = 150;
  // Memory rises under load and is largely handed back once work stops — the
  // shape of a working set, which is exactly what the settle figures exist to
  // make visible rather than fold into the pass/fail decision.
  const samples = buildSamples({
    count: 200,
    rssKib: (i) =>
      i <= LOAD_SAMPLES ? 120_000 + i * 400 : 120_000 + LOAD_SAMPLES * 400 - (i - LOAD_SAMPLES) * 800,
    cpuMs: (i) => Math.min(i, LOAD_SAMPLES) * 100,
  });
  const { report } = runAnalyzer(
    samples,
    driver({ measureStartedAtMs: EPOCH0, wallMs: LOAD_SAMPLES * INTERVAL_MS }),
  );

  assert.equal(report.settle.available, true);
  assert.ok(report.settle.releasedKib > 0, 'should record memory handed back after load');
  assert.ok(report.settle.idleCpuFraction < 0.05, 'idle CPU should be near zero');
});

test('a short run is flagged as weak evidence even when it passes', () => {
  // 20 samples over 5s. The checks may well pass, but the report must not let a
  // window this small read as a confident clean bill of health.
  const samples = buildSamples({ count: 20, rssKib: () => 120_000 });
  const { report, exitCode } = runAnalyzer(samples, driver());

  assert.equal(report.overall, 'pass');
  assert.equal(report.underpowered, true);
  assert.match(report.underpoweredNote, /weak/i);
  assert.equal(exitCode, 0, 'weak evidence is a caveat, not a failure');
});

test('a long clean run is not flagged as underpowered', () => {
  // 200 samples at 250ms = 50s, comfortably over both thresholds.
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const { report } = runAnalyzer(samples, driver());

  assert.equal(report.underpowered, false);
  assert.equal(report.underpoweredNote, null);
});

test('clipping is skipped when it would leave too little to analyze', () => {
  // A load window of only a few samples must fall back to the full series
  // rather than exiting, and must say that it did not clip.
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const { report } = runAnalyzer(
    samples,
    driver({ measureStartedAtMs: EPOCH0, wallMs: 3 * INTERVAL_MS }),
  );

  assert.equal(report.window.clippedToLoadWindow, false);
  assert.equal(report.window.loadWindowSamples, 200);
});

test('a core that stops serving fails, and its resource passes are qualified', () => {
  // The regression this guards: a run where the core died two thirds of the way
  // in reported PASS on every resource check. All of them were true, and all of
  // them were true BECAUSE nothing was happening — flat memory, fallen CPU,
  // stable threads. A dead process looks exactly like a healthy idle one unless
  // liveness is judged on completed work.
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const turns = buildTurns(50_000, (t) => (t < 33_000 ? 20 : 0));

  const { report, exitCode } = runAnalyzer(samples, driver(), [], turns);

  assert.equal(report.throughput.verdict, 'fail', report.throughput.reason);
  assert.match(report.throughput.reason, /stopped serving/);
  assert.equal(report.livenessBroken, true);
  assert.match(report.livenessNote, /idle process/);
  assert.equal(report.overall, 'fail');
  assert.equal(exitCode, 1);
});

test('severe throughput degradation fails even when the core is still alive', () => {
  // Still serving, but at a fraction of its starting rate under constant load.
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const turns = buildTurns(50_000, (t) => (t < 12_500 ? 40 : 5));

  const { report } = runAnalyzer(samples, driver(), [], turns);

  assert.equal(report.throughput.verdict, 'fail', report.throughput.reason);
  assert.match(report.throughput.reason, /degrading/);
  assert.ok(report.throughput.retainedFraction < 0.5);
  assert.equal(report.overall, 'fail');
});

test('steady throughput passes and is not flagged as a liveness break', () => {
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const turns = buildTurns(50_000, () => 20);

  const { report, exitCode } = runAnalyzer(samples, driver(), [], turns);

  assert.equal(report.throughput.verdict, 'pass', report.throughput.reason);
  assert.equal(report.livenessBroken, false);
  assert.equal(report.livenessNote, null);
  assert.equal(report.overall, 'pass');
  assert.equal(exitCode, 0);
});

test('throughput quarters are aligned to the measured epoch window', () => {
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const measured = driver({ measureStartedAtMs: EPOCH0 });
  const turns = [
    ...Array.from({ length: 25 }, (_, i) => ({
      tMs: 49_000,
      epochMs: EPOCH0 - 5_000 + i,
      ok: true,
    })),
    ...Array.from({ length: 25 }, (_, i) => ({
      tMs: 49_000,
      epochMs: EPOCH0 + 1_000 + i * 400,
      ok: true,
    })),
    ...Array.from({ length: 25 }, (_, i) => ({
      tMs: 1_000,
      epochMs: EPOCH0 + 39_000 + i * 400,
      ok: true,
    })),
    ...Array.from({ length: 25 }, (_, i) => ({
      tMs: 1_000,
      epochMs: EPOCH0 + 55_000 + i,
      ok: true,
    })),
  ];
  const turnsText = `${turns.map(turn => JSON.stringify(turn)).join('\n')}\n`;

  const { report } = runAnalyzer(samples, measured, [], turnsText);

  assert.equal(report.throughput.firstQuarterTurnsPerSec, 2);
  assert.equal(report.throughput.lastQuarterTurnsPerSec, 2);
  assert.equal(report.throughput.verdict, 'pass');
});

test('throughput is simply unavailable when no turn log is supplied', () => {
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const { report, exitCode } = runAnalyzer(samples, driver());

  assert.equal(report.throughput.available, false);
  assert.equal(report.livenessBroken, false);
  assert.equal(exitCode, 0, 'a missing turn log is not a failure');
});

test('degradation short of an outage is not called a liveness break', () => {
  // The core kept serving, just more slowly. Throughput must fail, but the
  // resource verdicts stay meaningful — so the "this describes an idle process"
  // qualifier must NOT appear, because the process plainly was not idle.
  const samples = buildSamples({ count: 200, rssKib: () => 120_000 });
  const turns = buildTurns(50_000, (t) => (t < 12_500 ? 40 : 5));

  const { report } = runAnalyzer(samples, driver(), [], turns);

  assert.equal(report.throughput.verdict, 'fail');
  assert.equal(report.throughput.stopped, false);
  assert.equal(report.livenessBroken, false, 'degradation is not an outage');
  assert.equal(report.livenessNote, null);
});

test('a leaking RSS curve is marked confounded when the workspace grew a lot', () => {
  // `fresh` mode stops conversation history accumulating, but the agent still
  // persists memory chunks every turn. RSS tracking an index over data that
  // genuinely grew is not a leak, and the report must not claim otherwise.
  const samples = buildSamples({ count: 200, rssKib: (i) => 120_000 + i * 400 });
  const { report } = runAnalyzer(samples, driver(), [
    '--workspace-mib-before', '10',
    '--workspace-mib-after', '4000',
  ]);

  const rss = report.memory.find((m) => m.field === 'rssKib');
  assert.equal(rss.verdict, 'fail', 'still a failure — the caveat does not excuse it');
  assert.equal(rss.confounded, true);
  assert.match(rss.confoundNote, /rather than a leak/);
  assert.equal(report.workspace.growthMib, 3990);
});

test('a leaking RSS curve is NOT confounded when the workspace barely grew', () => {
  // Same leak, but nothing accumulated on disk to explain it. This is the
  // unambiguous case, and it must read as such.
  const samples = buildSamples({ count: 200, rssKib: (i) => 120_000 + i * 400 });
  const { report } = runAnalyzer(samples, driver(), [
    '--workspace-mib-before', '10',
    '--workspace-mib-after', '12',
  ]);

  const rss = report.memory.find((m) => m.field === 'rssKib');
  assert.equal(rss.verdict, 'fail');
  assert.notEqual(rss.confounded, true);
  assert.equal(report.workspace.growthMib, 2);
});

test('a series too short to analyze exits non-zero rather than guessing', () => {
  const samples = buildSamples({ count: 4, rssKib: () => 120_000 });
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'bench-analyze-'));
  try {
    const samplesPath = path.join(dir, 'samples.jsonl');
    fs.writeFileSync(samplesPath, samples);
    let exitCode = 0;
    try {
      execFileSync(process.execPath, [ANALYZE, '--samples', samplesPath], {
        encoding: 'utf8',
        stdio: 'pipe',
      });
    } catch (err) {
      exitCode = err.status ?? 1;
    }
    assert.notEqual(exitCode, 0);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
