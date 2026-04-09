#!/usr/bin/env node
// Bump version in package.json, tauri.conf.json, and both Cargo.toml manifests.
//
// Usage:
//   node scripts/release/bump-version.js <patch|minor|major>
//
// Outputs (to stdout, one per line):
//   version=X.Y.Z
//   tag=vX.Y.Z
//
// When GITHUB_OUTPUT is set (CI), the same key=value pairs are appended there.

'use strict';

const fs = require('fs');
const path = require('path');

const RELEASE_TYPE = process.argv[2] || process.env.RELEASE_TYPE;
const allowed = new Set(['patch', 'minor', 'major']);
if (!allowed.has(RELEASE_TYPE)) {
  console.error(`Usage: bump-version.js <patch|minor|major>  (got: "${RELEASE_TYPE}")`);
  process.exit(1);
}

const root = path.resolve(__dirname, '..', '..');
const packagePath = path.join(root, 'app/package.json');
const tauriPath = path.join(root, 'app/src-tauri/tauri.conf.json');
const tauriCargoPath = path.join(root, 'app/src-tauri/Cargo.toml');
const coreCargoPath = path.join(root, 'Cargo.toml');

// ── Read current version ────────────────────────────────────────────────────
const pkg = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
const match = String(pkg.version || '').match(/^(\d+)\.(\d+)\.(\d+)$/);
if (!match) {
  throw new Error(`package.json version must be SemVer X.Y.Z, found: ${pkg.version}`);
}

let major = Number(match[1]);
let minor = Number(match[2]);
let patch = Number(match[3]);

// ── Bump ────────────────────────────────────────────────────────────────────
if (RELEASE_TYPE === 'major') {
  major += 1; minor = 0; patch = 0;
} else if (RELEASE_TYPE === 'minor') {
  minor += 1; patch = 0;
} else {
  patch += 1;
}
const nextVersion = `${major}.${minor}.${patch}`;

// ── Write package.json ──────────────────────────────────────────────────────
pkg.version = nextVersion;
fs.writeFileSync(packagePath, `${JSON.stringify(pkg, null, 2)}\n`);

// ── Write tauri.conf.json ───────────────────────────────────────────────────
const tauri = JSON.parse(fs.readFileSync(tauriPath, 'utf8'));
tauri.version = nextVersion;
fs.writeFileSync(tauriPath, `${JSON.stringify(tauri, null, 2)}\n`);

function bumpCargoVersion(filePath, nextVersion) {
  const cargo = fs.readFileSync(filePath, 'utf8');
  const updatedCargo = cargo.replace(
    /(\[package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m,
    `$1${nextVersion}$3`,
  );
  if (updatedCargo === cargo) {
    throw new Error(`Failed to update [package].version in ${path.relative(root, filePath)}`);
  }
  fs.writeFileSync(filePath, updatedCargo);
}

// ── Write Cargo.toml files ──────────────────────────────────────────────────
bumpCargoVersion(tauriCargoPath, nextVersion);
bumpCargoVersion(coreCargoPath, nextVersion);

// ── Output ──────────────────────────────────────────────────────────────────
const lines = `version=${nextVersion}\ntag=v${nextVersion}\n`;
process.stdout.write(lines);

if (process.env.GITHUB_OUTPUT) {
  fs.appendFileSync(process.env.GITHUB_OUTPUT, lines);
}

console.error(`[bump-version] ${pkg.version} → ${nextVersion}`);
