#!/usr/bin/env node
// Compute the next staging git tag without mutating any version files.
//
// Convention: `staging/vX.Y.Z-N`
//   X.Y.Z = current `app/package.json` version (kept untouched)
//   N     = monotonic counter of staging cuts already produced for that
//           base version, +1
//
// This keeps staging cadence noise-free (no version-bump commits) while
// still giving every staging artifact a stable, named ref. Production
// releases resolve the latest such tag and decide their own semver bump
// from there.
//
// Usage:
//   node scripts/release/next-staging-tag.js
//
// Inputs:
//   - `app/package.json` (version)
//   - existing tags discovered via `git tag --list 'staging/v<base>-*'`
//
// Outputs (stdout, one per line; also appended to GITHUB_OUTPUT when set):
//   version=X.Y.Z
//   tag=staging/vX.Y.Z-N
//   counter=N

"use strict";

const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..", "..");
const pkg = JSON.parse(
  fs.readFileSync(path.join(root, "app/package.json"), "utf8"),
);
const version = String(pkg.version || "");
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(
    `[next-staging-tag] app/package.json version must be SemVer X.Y.Z, found: ${version}`,
  );
  process.exit(1);
}

function listMatchingTags(pattern) {
  const out = execFileSync("git", ["tag", "--list", pattern], {
    cwd: root,
    encoding: "utf8",
  });
  return out
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
}

const pattern = `staging/v${version}-*`;
const existing = listMatchingTags(pattern);
let maxN = 0;
const suffixRe = new RegExp(
  `^staging/v${version.replace(/\./g, "\\.")}-(\\d+)$`,
);
for (const tag of existing) {
  const m = tag.match(suffixRe);
  if (m) {
    const n = Number(m[1]);
    if (Number.isInteger(n) && n > maxN) maxN = n;
  }
}
const counter = maxN + 1;
const tag = `staging/v${version}-${counter}`;

const lines = `version=${version}\ntag=${tag}\ncounter=${counter}\n`;
process.stdout.write(lines);

if (process.env.GITHUB_OUTPUT) {
  fs.appendFileSync(process.env.GITHUB_OUTPUT, lines);
}

console.error(
  `[next-staging-tag] base=${version} existing=${existing.length} -> ${tag}`,
);
