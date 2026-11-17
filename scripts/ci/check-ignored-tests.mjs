#!/usr/bin/env node
//
// Per-crate `#[ignore]` ratchet. The count may only go down.
//
// What this counts is `#[ignore]` ATTRIBUTES PRESENT IN THE SOURCE TREE — not
// tests libtest skips at runtime. The two numbers differ and the difference is
// not fully explained: `openhuman-core` has 96 attributes in source while a
// scoped `--lib` run reported 83 ignored. Feature gating accounts for exactly
// one of those (the listener-shutdown test, since moved to `openhuman-rpc`), so the rest is open. The static count
// is the right thing to ratchet anyway: it is what a reviewer actually adds in
// a diff, and it is deterministic without a build or a feature-set choice —
// which is why this gate can run in `rust-quality` before clippy instead of
// waiting on the full-suite lane (#5021).
//
// Usage:
//   node scripts/ci/check-ignored-tests.mjs
//   node scripts/ci/check-ignored-tests.mjs --write-baseline

import { readdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// `--root <dir>` points the scan and the baseline at a throwaway tree, which is
// how scripts/__tests__/ignored-test-ratchet.test.mjs exercises the failure
// paths without mutating the real crates.
const rootFlag = process.argv.indexOf("--root");
const repoRoot = rootFlag === -1
  ? resolve(dirname(fileURLToPath(import.meta.url)), "../..")
  : resolve(process.argv[rootFlag + 1]);
const baselinePath = resolve(repoRoot, "scripts/ci/ignored-test-baseline.json");
const writeBaselineMode = process.argv.includes("--write-baseline");

// Anchored at line start on purpose. An unanchored `#\[ignore` match also hits
// doc comments and prose that merely DISCUSS ignoring — there are nine such
// lines today, including two files that argue at length about ignored tests.
// Counting those would let a PR earn ratchet credit by deleting a comment,
// which rewards the wrong action while looking rigorous.
const IGNORE_ATTRIBUTE = /^\s*#\s*\[\s*ignore\b/;

// `#[cfg_attr(<cond>, ignore = "...")]` is a conditional ignore (for example
// `target_os = "linux"` for a test that only cannot run on one platform). It is
// counted exactly like a plain `#[ignore]`: one attribute, one count, so scoping
// an ignore to a platform never shrinks the number, and un-skipping it on every
// platform is what earns ratchet credit. Anchored at line start for the same
// reason as above.
const CONDITIONAL_IGNORE_ATTRIBUTE = /^\s*#\s*\[\s*cfg_attr\s*\(.*\bignore\b/;

async function rustFilesUnder(root) {
  const found = [];
  const walk = async (dir) => {
    let entries;
    try {
      entries = await readdir(dir, { withFileTypes: true });
    } catch {
      return; // A bucket that does not exist contributes nothing.
    }
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === "target" || entry.name === "node_modules") continue;
        await walk(path);
      } else if (entry.name.endsWith(".rs")) {
        found.push(path);
      }
    }
  };
  await walk(root);
  return found.sort();
}

// One bucket per crate, plus the root `tests/` tree. Root tests are compiled by
// openhuman-cli through explicit `[[test]] path = "../../tests/..."` entries,
// but they are their own reviewable surface, so folding them into the cli count
// would hide movement in both.
const buckets = new Map();

const crateDirs = (await readdir(resolve(repoRoot, "crates"), { withFileTypes: true }))
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort();

for (const [bucket, root] of [
  ...crateDirs.map((name) => [name, resolve(repoRoot, "crates", name)]),
  ["tests", resolve(repoRoot, "tests")],
]) {
  let count = 0;
  for (const path of await rustFilesUnder(root)) {
    const lines = (await readFile(path, "utf8")).split("\n");
    for (const line of lines) {
      if (IGNORE_ATTRIBUTE.test(line) || CONDITIONAL_IGNORE_ATTRIBUTE.test(line)) count += 1;
    }
  }
  buckets.set(bucket, count);
}

const actual = Object.fromEntries([...buckets].sort(([a], [b]) => a.localeCompare(b)));

if (writeBaselineMode) {
  await writeFile(baselinePath, `${JSON.stringify(actual, null, 2)}\n`);
  const total = Object.values(actual).reduce((sum, n) => sum + n, 0);
  console.log(`Wrote ${total} '#[ignore]' attributes across ${Object.keys(actual).length} buckets to ${relative(repoRoot, baselinePath)}.`);
  process.exit(0);
}

let baseline;
try {
  baseline = JSON.parse(await readFile(baselinePath, "utf8"));
} catch (error) {
  console.error(`Unable to read ${relative(repoRoot, baselinePath)}: ${error.message}`);
  process.exit(1);
}

const failures = [];
// A bucket absent from the baseline is 0, so a new crate that arrives carrying
// ignored tests reddens instead of being grandfathered in.
for (const [bucket, count] of Object.entries(actual)) {
  const allowed = baseline[bucket] ?? 0;
  if (count > allowed) {
    failures.push(`${bucket}: ${count} '#[ignore]' attributes, baseline ${allowed} (+${count - allowed}). This ratchet only goes down — un-skip the test, or delete it.`);
  } else if (count < allowed) {
    failures.push(`${bucket}: ${count} '#[ignore]' attributes, baseline ${allowed} (${count - allowed}). Tighten the baseline in the same PR so the count cannot drift back up.`);
  }
}
for (const bucket of Object.keys(baseline)) {
  if (!(bucket in actual)) {
    failures.push(`${bucket}: baselined but no longer scanned. Remove it from ${relative(repoRoot, baselinePath)}.`);
  }
}

if (failures.length) {
  console.error("Ignored-test ratchet: '#[ignore]' attributes present in the source tree no longer match the baseline.\n");
  for (const failure of failures) console.error(`  ${failure}`);
  console.error(`\nRe-baseline deliberately, never as a reflex:\n  pnpm rust:ignored-tests --write-baseline\nThen say in the PR body which tests moved and why. Raising a number needs a reason a reviewer can check.`);
  process.exit(1);
}

const total = Object.values(actual).reduce((sum, n) => sum + n, 0);
console.log(`Ignored-test ratchet holds (${total} '#[ignore]' attributes in source, matching the per-bucket baseline).`);
