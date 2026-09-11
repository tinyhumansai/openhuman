#!/usr/bin/env node
// Fails when a `vendor/*` submodule pin moves BACKWARDS — onto a commit that is
// an ancestor of the one the base branch already had.
//
// This is the other half of openhuman#5727, and on the evidence it is the half
// that actually bites. A check that only asserts "the pins agree with each
// other" would have passed the commit that caused the most recent regression,
// because that commit WAS internally consistent — it was consistent with an
// older release.
//
//   14a23b994 "Pin v1.10.0 and route memory-source sync through the driver"
//     vendor/tinyagents  e0f3210 -> bbcd0a6
//
//   $ git -C vendor/tinyagents merge-base --is-ancestor bbcd0a6 e0f3210 && echo backwards
//   backwards
//
// e0f3210 is tinyagents#122 (2026-08-25); bbcd0a6 is #121 (2026-08-22). The bump
// silently removed a shipped fix from the product and nobody noticed until #5796
// put it back. #5787 was raised to undo the same shape on #5725.
//
// Note what the registry-pin check could NOT have done here: `tinyagents` has no
// record in `modules::registry::ALL` at all — no version, no digest. It is a
// compile-time dependency with exactly ONE pin, so "do the two pins agree?" has
// nothing to compare. Only movement direction catches it. That is why this is a
// separate gate over ALL sixteen submodules rather than an extra assertion in
// check-module-pins.mjs, which can only ever see the seven that are also modules.
//
// A backwards move is not always wrong — reverting a bad bump is a backwards
// move and is exactly right. It just must be deliberate, so it is declared: put
// `[pin-rewind]` in the PR title or any commit message on the branch, naming why.
//
// No `gh` and no API: this reads gitlinks out of the two trees with git alone,
// so it cannot be defeated by `gh pr view --json files` silently capping at 100
// files — which is how the #5725 gitlink went unreviewed in the first place.
//
// Usage: check-submodule-monotonic.mjs [base-ref] [head-ref]
//   Defaults: origin/$GITHUB_BASE_REF (or origin/main) .. HEAD
import { execFileSync } from 'node:child_process';

import { classifyMove, parseGitlinks, rewindDeclared } from '../lib/module-pins.mjs';

// Any throw below is a check that could not be COMPLETED, which is not the same
// as a check that passed. Report it as a failure with a legible message rather
// than a stack trace, and never exit 0. Two gates have shipped here that
// swallowed a git error and reported clean having scanned nothing.
process.on('uncaughtException', (error) => {
  console.error(`\nSubmodule monotonicity check FAILED\n`);
  console.error(`  \u2717 ${error.message}\n`);
  console.error('  This gate could not complete. That is a failure, not a pass.\n');
  process.exit(1);
});

const rawBase = process.argv[2] ?? (process.env.GITHUB_BASE_REF ? `origin/${process.env.GITHUB_BASE_REF}` : 'origin/main');
const HEAD = process.argv[3] ?? 'HEAD';

function run(args, cwd = '.') {
  return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}

/** Run, or throw with context. Never returns a value a caller could read as "fine". */
function mustRun(args, cwd, what) {
  try {
    return run(args, cwd);
  } catch (error) {
    const detail = (error.stderr || error.message || '').toString().trim().split('\n')[0];
    throw new Error(`${what}: \`git ${args.join(' ')}\` failed in ${cwd}: ${detail}`);
  }
}

/** path -> gitlink sha, for every submodule recorded in `ref`'s tree. */
function gitlinks(ref) {
  return parseGitlinks(mustRun(['ls-tree', '-r', ref], '.', `read tree of ${ref}`));
}

// Resolve the base. A base we cannot resolve is a hard stop: continuing would
// scan zero submodules and print a pass.
let base;
try {
  base = run(['rev-parse', '--verify', `${rawBase}^{commit}`]);
} catch {
  try {
    base = run(['rev-parse', '--verify', `${rawBase.replace(/^origin\//, '')}^{commit}`]);
  } catch {
    console.error(
      `Submodule monotonicity check FAILED\n\n` +
        `  ✗ cannot resolve base ref "${rawBase}".\n` +
        `    Fetch it before running this gate (actions/checkout needs fetch-depth: 0,\n` +
        `    or an explicit \`git fetch origin <base>\`). Refusing to pass having compared\n` +
        `    nothing.\n`,
    );
    process.exit(1);
  }
}

const mergeBase = mustRun(['merge-base', base, HEAD], '.', 'find merge base');
const before = gitlinks(mergeBase);
const after = gitlinks(HEAD);

if (after.size === 0) {
  console.error('Submodule monotonicity check FAILED\n\n  ✗ HEAD records zero submodules — this gate scanned nothing.\n');
  process.exit(1);
}

// A deliberate rewind is declared on the branch, not configured in a file: the
// declaration should travel with the commit that does it.
const branchText = [
  process.env.PR_TITLE ?? '',
  (() => {
    try { return run(['log', '--format=%B', `${mergeBase}..${HEAD}`]); } catch { return ''; }
  })(),
].join('\n');
const declaredRewind = rewindDeclared(branchText);

const moved = [];
for (const [path, headSha] of after) {
  const baseSha = before.get(path);
  if (!baseSha || baseSha === headSha) continue;
  moved.push({ path, baseSha, headSha, added: !before.has(path) });
}

const failures = [];
const forward = [];

for (const { path, baseSha, headSha } of moved) {
  // Both commits must be present in the submodule's object store to answer the
  // question. If they are not, say so and fail — do not assume forward.
  let haveBoth = true;
  for (const sha of [baseSha, headSha]) {
    try {
      run(['cat-file', '-e', `${sha}^{commit}`], path);
    } catch {
      haveBoth = false;
      try {
        run(['fetch', '--quiet', 'origin', sha], path);
        run(['cat-file', '-e', `${sha}^{commit}`], path);
        haveBoth = true;
      } catch {
        failures.push(
          `${path}: cannot verify direction — commit ${sha.slice(0, 9)} is not in the submodule's\n` +
            `    object store and could not be fetched. This gate does not guess: a pin whose\n` +
            `    direction cannot be established is treated as unverified, not as forward.`,
        );
      }
    }
    if (!haveBoth) break;
  }
  if (!haveBoth) continue;

  /** `merge-base --is-ancestor` answers 0 (yes) or 1 (no); anything else is broken. */
  const isAncestor = (a, b) => {
    try {
      execFileSync('git', ['merge-base', '--is-ancestor', a, b], { cwd: path, stdio: 'ignore' });
      return true;
    } catch (error) {
      if (error.status === 1) return false;
      throw new Error(`ancestry query failed (exit ${error.status})`);
    }
  };

  // BOTH directions. Asking only "is head an ancestor of base?" and reading a
  // failed query as forward mistakes a DIVERGENT pin — head and base on sibling
  // branches, or a rebased submodule history — for a fast-forward. Divergence
  // loses the base's commits exactly as a rewind does, so it is not forward.
  let move;
  try {
    move = classifyMove({
      headIsAncestorOfBase: isAncestor(headSha, baseSha),
      baseIsAncestorOfHead: isAncestor(baseSha, headSha),
    });
  } catch (error) {
    failures.push(`${path}: ${error.message} — treating as unverified, not as forward.`);
    continue;
  }

  if (move === 'rewind' || move === 'divergent') {
    const oldSubject = (() => { try { return run(['log', '--format=%h %ad %s', '--date=short', '-1', baseSha], path); } catch { return baseSha.slice(0, 9); } })();
    const newSubject = (() => { try { return run(['log', '--format=%h %ad %s', '--date=short', '-1', headSha], path); } catch { return headSha.slice(0, 9); } })();
    const behind = (() => { try { return run(['rev-list', '--count', `${headSha}..${baseSha}`], path); } catch { return '?'; } })();
    const headline =
      move === 'rewind'
        ? `${path} moved BACKWARDS by ${behind} commit(s).`
        : `${path} moved SIDEWAYS — the new pin and the base pin have diverged, and ` +
          `${behind} commit(s) on the base side are not reachable from it.`;
    failures.push(
      `${headline}\n` +
        `    base : ${oldSubject}\n` +
        `    head : ${newSubject}\n` +
        `    The new pin is an ancestor of the one on the base branch, so everything\n` +
        `    merged into that submodule in between is removed from this build. That is\n` +
        `    how 14a23b994 dropped tinyagents#122 (restored later by #5796) and how #5725\n` +
        `    dropped eight tinyagents commits (undone by #5787). Neither showed up as a\n` +
        `    code change — a gitlink bump is two lines and no diff.\n` +
        `    If the rewind is deliberate, say so: put [pin-rewind] in the PR title or a\n` +
        `    commit message on this branch, with the reason.`,
    );
  } else {
    forward.push(path);
  }
}

if (failures.length > 0 && declaredRewind) {
  const isMove = (f) => f.includes('moved BACKWARDS') || f.includes('moved SIDEWAYS');
  const rewinds = failures.filter(isMove);
  const other = failures.filter((f) => !isMove(f));
  if (other.length === 0) {
    console.log('Submodule monotonicity check OK — backwards move(s) declared with [pin-rewind]:\n');
    for (const r of rewinds) console.log(`  ~ ${r.split('\n')[0]}`);
    process.exit(0);
  }
  // An unverifiable pin is never waived by [pin-rewind]: the marker declares a
  // known rewind, it does not license skipping the check.
  console.error('\nSubmodule monotonicity check FAILED\n');
  for (const f of other) console.error(`  ✗ ${f}\n`);
  process.exit(1);
}

if (failures.length > 0) {
  console.error('\nSubmodule monotonicity check FAILED\n');
  for (const f of failures) console.error(`  ✗ ${f}\n`);
  process.exit(1);
}

const changed = moved.length;
console.log(
  changed === 0
    ? `Submodule monotonicity check OK — ${after.size} submodule(s), none moved against ${rawBase}.`
    : `Submodule monotonicity check OK — ${changed} submodule pin(s) moved forward: ${forward.join(', ')}`,
);
