#!/usr/bin/env node
// Run the TypeScript type-check gate (`tsc`, noEmit) in parallel with the Vite
// bundle instead of sequentially (`tsc && vite build`).
//
// Vite/esbuild strips types itself and never consumes tsc's output, so tsc is a
// pure *gate*: running it alongside the bundle makes wall-clock max(tsc, vite)
// rather than tsc + vite. The build still fails on a type error because we exit
// non-zero if EITHER job fails — the exit code is the gate, not the presence of
// build artifacts (CI keys off the exit code, and a failed build's artifacts
// are discarded).
//
// Any extra args are forwarded to `vite build` (e.g. `--mode development`).
// Env vars (e.g. VITE_OPENHUMAN_TARGET set via cross-env) are inherited by both
// children. `shell: true` resolves the `tsc`/`vite` shims from node_modules/.bin
// on every platform, including the `.cmd` wrappers on Windows.
import { spawn } from 'node:child_process';

const viteArgs = process.argv.slice(2);

const jobs = [
  { name: 'tsc', cmd: 'tsc', args: [] },
  { name: 'vite', cmd: 'vite', args: ['build', ...viteArgs] },
];

const active = new Map();
let firstFailure = null;

function stopSiblings(failedName) {
  for (const [name, child] of active) {
    if (name !== failedName && !child.killed) {
      child.kill();
    }
  }
}

function run({ name, cmd, args }) {
  return new Promise(resolve => {
    const child = spawn(cmd, args, { stdio: 'inherit', shell: true });
    active.set(name, child);
    child.on('exit', (code, signal) => {
      active.delete(name);
      const exitCode = code ?? (signal ? 1 : 0);
      if (exitCode !== 0 && firstFailure === null) {
        firstFailure = name;
        stopSiblings(name);
      }
      resolve({ name, code: exitCode });
    });
    child.on('error', err => {
      active.delete(name);
      console.error(`[build-parallel] failed to start ${name}: ${err.message}`);
      if (firstFailure === null) {
        firstFailure = name;
        stopSiblings(name);
      }
      resolve({ name, code: 1 });
    });
  });
}

const results = await Promise.all(jobs.map(run));
const failed = results.filter(r => r.code !== 0);
if (failed.length > 0) {
  console.error(`[build-parallel] failed: ${failed.map(f => f.name).join(', ')}`);
  process.exit(1);
}
