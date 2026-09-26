import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const HOOK = resolve(REPO_ROOT, '.husky/pre-push');
const ZERO_OID = '0'.repeat(40);
// Keep every git process off the developer's own config: a global
// `commit.gpgsign` or `core.hooksPath` would fail these commits for reasons
// that have nothing to do with the hook under test.
const GIT_ENV = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' };

const workspaces = [];
after(() => {
  for (const dir of workspaces) rmSync(dir, { recursive: true, force: true });
});

function git(cwd, ...args) {
  return execFileSync('git', args, { cwd, encoding: 'utf8', env: GIT_ENV }).trim();
}

/**
 * A git repo with a bare origin and a `pnpm` stub on PATH.
 *
 * The stub records every invocation and fails the ones named in `PNPM_FAIL`,
 * so a test asserts on what the hook DID (which checks it ran, and its exit)
 * without a real format, lint, tsc or clippy run.
 */
function workspace() {
  const dir = mkdtempSync(join(tmpdir(), 'pre-push-hook-'));
  workspaces.push(dir);
  const repo = join(dir, 'repo');
  const origin = join(dir, 'origin.git');
  const bin = join(dir, 'bin');
  const log = join(dir, 'pnpm.log');
  mkdirSync(repo);
  mkdirSync(bin);

  execFileSync('git', ['init', '--quiet', '--bare', origin], { env: GIT_ENV });
  git(repo, 'init', '--quiet', '--initial-branch=main');
  git(repo, 'config', 'user.email', 'hook@test.local');
  git(repo, 'config', 'user.name', 'hook test');
  git(repo, 'remote', 'add', 'origin', origin);

  writeFileSync(
    join(bin, 'pnpm'),
    [
      '#!/bin/sh',
      'echo "$*" >> "$PNPM_LOG"',
      'for failing in $PNPM_FAIL; do',
      '  case "$*" in',
      '    *"$failing"*) exit 1 ;;',
      '  esac',
      'done',
      'exit 0',
      '',
    ].join('\n')
  );
  chmodSync(join(bin, 'pnpm'), 0o755);

  return { dir, repo, origin, bin, log };
}

function commit(ws, path, body, message) {
  const full = join(ws.repo, path);
  mkdirSync(dirname(full), { recursive: true });
  writeFileSync(full, body);
  git(ws.repo, 'add', path);
  git(ws.repo, 'commit', '--quiet', '-m', message);
  return git(ws.repo, 'rev-parse', 'HEAD');
}

function runHook(ws, refLines, { fail = '', env = {} } = {}) {
  writeFileSync(ws.log, '');
  const result = spawnSync('sh', [HOOK, 'origin', ws.origin], {
    cwd: ws.repo,
    encoding: 'utf8',
    input: refLines.map(line => `${line}\n`).join(''),
    env: {
      ...GIT_ENV,
      PATH: `${ws.bin}:${process.env.PATH}`,
      PNPM_LOG: ws.log,
      PNPM_FAIL: fail,
      PRE_PUSH_FORCE_CLIPPY: '',
      ...env,
    },
  });
  return {
    status: result.status,
    stdout: result.stdout ?? '',
    calls: readFileSync(ws.log, 'utf8').split('\n').filter(Boolean),
  };
}

function ranClippy(run) {
  return run.calls.includes('rust:clippy');
}

/** A repo whose `main` the origin already has, plus one extra local commit. */
function pushOf(path, body) {
  const ws = workspace();
  commit(ws, 'README.md', 'base\n', 'base');
  git(ws.repo, 'push', '--quiet', 'origin', 'main');
  const before = git(ws.repo, 'rev-parse', 'HEAD');
  const after_ = commit(ws, path, body, `touch ${path}`);
  return { ws, line: `refs/heads/main ${after_} refs/heads/main ${before}` };
}

// ── the clippy gate (#6533 defect 1) ───────────────────────────────────────

test('a push with no Rust changes does not run rust:clippy', () => {
  // The defect: `pnpm rust:clippy` compiles three crates plus the shell with
  // `-D warnings`, so one stale import anywhere blocked EVERY push, including
  // a README edit that cannot possibly fix it.
  const { ws, line } = pushOf('docs/guide.md', 'docs only\n');
  const run = runHook(ws, [line]);
  assert.equal(run.status, 0, run.stdout);
  assert.equal(ranClippy(run), false, `clippy ran for a docs-only push: ${run.calls}`);
  assert.match(run.stdout, /skipping rust:clippy/);
  // The rest of the hook is untouched by the gate.
  assert.deepEqual(run.calls, [
    'format:check',
    'lint',
    'compile',
    '--dir app run lint:commands-tokens',
    '--dir app run lint:ui-tokens',
  ]);
});

test('a push that touches a .rs file still runs rust:clippy', () => {
  const { ws, line } = pushOf('crates/openhuman-core/src/lib.rs', 'pub fn a() {}\n');
  assert.equal(ranClippy(runHook(ws, [line])), true);
});

test('a push that touches a manifest still runs rust:clippy', () => {
  // A feature or dependency edit changes what compiles without touching a
  // single `.rs` file.
  const { ws, line } = pushOf('Cargo.toml', '[workspace]\n');
  assert.equal(ranClippy(runHook(ws, [line])), true);
});

test('a push that touches .cargo config still runs rust:clippy', () => {
  // Compiler flags, linker and target settings live there; they change what
  // clippy sees without touching a `.rs` file.
  const { ws, line } = pushOf('.cargo/config.toml', '[build]\n');
  assert.equal(ranClippy(runHook(ws, [line])), true);
});

test('a push that touches a vendored submodule path still runs rust:clippy', () => {
  const { ws, line } = pushOf('vendor/tinyagents/notes.md', 'x\n');
  assert.equal(ranClippy(runHook(ws, [line])), true);
});

test('a push that bumps a submodule pointer still runs rust:clippy', () => {
  // `[patch.crates-io]` resolves every vendored crate by path, so a pointer
  // bump changes what the workspace compiles against. The Rust CI filters
  // list `.gitmodules` for the same reason.
  const { ws, line } = pushOf('.gitmodules', '[submodule "vendor/tinyagents"]\n');
  assert.equal(ranClippy(runHook(ws, [line])), true);
});

test('a clippy failure in a Rust push still blocks it', () => {
  // The gate must not weaken the check it gates.
  const { ws, line } = pushOf('crates/openhuman-core/src/lib.rs', 'pub fn a() {}\n');
  const run = runHook(ws, [line], { fail: 'rust:clippy' });
  assert.equal(run.status, 1);
  assert.match(run.stdout, /Pre-push checks failed/);
});

test('PRE_PUSH_FORCE_CLIPPY runs it regardless of what changed', () => {
  const { ws, line } = pushOf('docs/guide.md', 'docs only\n');
  assert.equal(ranClippy(runHook(ws, [line], { env: { PRE_PUSH_FORCE_CLIPPY: '1' } })), true);
});

test('a hook run with no ref lines runs rust:clippy rather than skipping', () => {
  // Fail safe. Run by hand, or by anything that does not speak the pre-push
  // protocol, there is no range to read — guessing "no Rust" there would skip
  // the check silently, which is the worse of the two wrong answers.
  const { ws } = pushOf('docs/guide.md', 'docs only\n');
  assert.equal(ranClippy(runHook(ws, [])), true);
});

test('a new branch the remote has not seen is scoped to its own commits', () => {
  // The remote OID is all zeros here, so there is no `remote..local` range and
  // the hook has to work out what the push adds on its own. It must not fall
  // back to the whole history, or the first push of any branch would always
  // look like it touches Rust.
  const ws = workspace();
  commit(ws, 'crates/openhuman-core/src/lib.rs', 'pub fn a() {}\n', 'rust on main');
  git(ws.repo, 'push', '--quiet', 'origin', 'main');
  git(ws.repo, 'checkout', '--quiet', '-b', 'docs-branch');
  const head = commit(ws, 'docs/guide.md', 'docs only\n', 'docs on branch');
  const run = runHook(ws, [`refs/heads/docs-branch ${head} refs/heads/docs-branch ${ZERO_OID}`]);
  assert.equal(ranClippy(run), false, `clippy ran for a docs-only branch: ${run.calls}`);
});

test('a new branch that adds Rust runs rust:clippy', () => {
  const ws = workspace();
  commit(ws, 'README.md', 'base\n', 'base');
  git(ws.repo, 'push', '--quiet', 'origin', 'main');
  git(ws.repo, 'checkout', '--quiet', '-b', 'rust-branch');
  const head = commit(ws, 'crates/openhuman-core/src/lib.rs', 'pub fn a() {}\n', 'rust');
  const run = runHook(ws, [`refs/heads/rust-branch ${head} refs/heads/rust-branch ${ZERO_OID}`]);
  assert.equal(ranClippy(run), true);
});

test('a branch deletion has no content to check', () => {
  const ws = workspace();
  commit(ws, 'README.md', 'base\n', 'base');
  git(ws.repo, 'push', '--quiet', 'origin', 'main');
  const head = git(ws.repo, 'rev-parse', 'HEAD');
  const run = runHook(ws, [`(delete) ${ZERO_OID} refs/heads/gone ${head}`]);
  assert.equal(run.status, 0, run.stdout);
  assert.equal(ranClippy(run), false);
});

test('one Rust ref among several decides for the whole push', () => {
  const ws = workspace();
  commit(ws, 'README.md', 'base\n', 'base');
  git(ws.repo, 'push', '--quiet', 'origin', 'main');
  const base = git(ws.repo, 'rev-parse', 'HEAD');
  const docs = commit(ws, 'docs/guide.md', 'docs\n', 'docs');
  git(ws.repo, 'checkout', '--quiet', '-b', 'side', base);
  git(ws.repo, 'push', '--quiet', 'origin', 'side');
  const sideBase = git(ws.repo, 'rev-parse', 'HEAD');
  const rust = commit(ws, 'crates/openhuman-core/src/lib.rs', 'pub fn a() {}\n', 'rust');
  const run = runHook(ws, [
    `refs/heads/main ${docs} refs/heads/main ${base}`,
    `refs/heads/side ${rust} refs/heads/side ${sideBase}`,
  ]);
  assert.equal(ranClippy(run), true);
});

// ── the token gates (#6533 defect 2) ───────────────────────────────────────

test('a lint:commands-tokens failure fails the hook', () => {
  // Regression for the defect: `$?` was read after `lint:ui-tokens`, so the
  // commands-tokens exit was discarded and this push passed.
  const { ws, line } = pushOf('docs/guide.md', 'docs only\n');
  const run = runHook(ws, [line], { fail: 'lint:commands-tokens' });
  assert.equal(run.status, 1, 'a failing lint:commands-tokens must fail the hook');
  assert.match(run.stdout, /cmd-tokens/);
});

test('a lint:ui-tokens failure still fails the hook', () => {
  const { ws, line } = pushOf('docs/guide.md', 'docs only\n');
  assert.equal(runHook(ws, [line], { fail: 'lint:ui-tokens' }).status, 1);
});

test('both token lints run even when the first one fails', () => {
  // Each has to report on its own; stopping at the first would hide the other.
  const { ws, line } = pushOf('docs/guide.md', 'docs only\n');
  const run = runHook(ws, [line], { fail: 'lint:commands-tokens' });
  assert.ok(run.calls.includes('--dir app run lint:commands-tokens'));
  assert.ok(run.calls.includes('--dir app run lint:ui-tokens'));
});

test('a clean push exits 0', () => {
  const { ws, line } = pushOf('docs/guide.md', 'docs only\n');
  assert.equal(runHook(ws, [line]).status, 0);
});
