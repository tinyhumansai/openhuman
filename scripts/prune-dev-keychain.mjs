#!/usr/bin/env node
/**
 * Report — and optionally remove — test-leaked entries in a developer's
 * `dev-keychain.json`.
 *
 * Why this exists
 * ---------------
 * Until the keyring test-isolation fix, `cfg(test)` builds resolved their
 * credential store from process-global state (`OPENHUMAN_WORKSPACE` / the
 * `WORKSPACE_DIR` OnceLock). When no test held a workspace guard, that resolved
 * to the developer's real `~/.openhuman`, so every `cargo test` run appended
 * entries keyed by the basename of a `TempDir` that no longer exists. These
 * accumulate forever and are never read again.
 *
 * Test builds no longer write this file at all, so this is a one-off cleanup.
 *
 * Safety
 * ------
 * - Dry run by default. Nothing is written without `--apply`.
 * - Only entries whose user-id segment matches a `tempfile` `TempDir` basename
 *   (`.tmpXXXXXX`) are considered leaked. Anything else — including real
 *   sessions — is always kept and listed.
 * - `--apply` writes a timestamped backup next to the file first, then rewrites
 *   it atomically (temp file + rename) with 0600 permissions.
 *
 * Usage
 * -----
 *   node scripts/prune-dev-keychain.mjs                 # report only
 *   node scripts/prune-dev-keychain.mjs --apply         # prune
 *   node scripts/prune-dev-keychain.mjs --file <path>   # non-default location
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/** `tempfile`'s default TempDir basename: `.tmp` + 6 random alphanumerics. */
const LEAKED_USER_ID = /^\.tmp[A-Za-z0-9]{6}$/;

function parseArgs(argv) {
  const args = { apply: false, file: null };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--apply") {
      args.apply = true;
    } else if (arg === "--file") {
      args.file = argv[i + 1];
      i += 1;
    } else if (arg === "--help" || arg === "-h") {
      args.help = true;
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  return args;
}

function defaultKeychainPath() {
  const workspace = process.env.OPENHUMAN_WORKSPACE?.trim();
  if (workspace) return path.join(workspace, "dev-keychain.json");
  const dir =
    process.env.OPENHUMAN_APP_ENV === "staging"
      ? ".openhuman-staging"
      : ".openhuman";
  return path.join(os.homedir(), dir, "dev-keychain.json");
}

/** An entry is leaked when its user-id segment is a dead TempDir basename. */
function isLeaked(key) {
  const separator = key.indexOf(":");
  if (separator <= 0) return false;
  return LEAKED_USER_ID.test(key.slice(0, separator));
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(
      "Usage: node scripts/prune-dev-keychain.mjs [--apply] [--file <path>]\n\n" +
        "Reports test-leaked dev-keychain entries. Writes nothing without --apply.",
    );
    return;
  }

  const file = args.file ?? defaultKeychainPath();
  if (!fs.existsSync(file)) {
    console.log(`No dev keychain at ${file} — nothing to do.`);
    return;
  }

  const raw = fs.readFileSync(file, "utf8");
  let entries;
  try {
    entries = JSON.parse(raw);
  } catch (error) {
    console.error(
      `Refusing to touch ${file}: it is not valid JSON (${error.message}).`,
    );
    process.exitCode = 1;
    return;
  }
  if (
    entries === null ||
    typeof entries !== "object" ||
    Array.isArray(entries)
  ) {
    console.error(
      `Refusing to touch ${file}: expected a JSON object of key → secret.`,
    );
    process.exitCode = 1;
    return;
  }

  const keys = Object.keys(entries);
  const leaked = keys.filter(isLeaked);
  const kept = keys.filter((key) => !isLeaked(key));

  console.log(`File:    ${file}`);
  console.log(`Size:    ${(raw.length / 1024 / 1024).toFixed(2)} MiB`);
  console.log(`Entries: ${keys.length}`);
  console.log(`Leaked:  ${leaked.length} (temp-dir-keyed, safe to remove)`);
  console.log(`Kept:    ${kept.length}`);
  for (const key of kept) console.log(`  keep  ${key}`);

  if (leaked.length === 0) {
    console.log("\nNothing to prune.");
    return;
  }

  if (!args.apply) {
    console.log("\nDry run — nothing written. Re-run with --apply to prune.");
    return;
  }

  const backup = `${file}.backup-${new Date().toISOString().replace(/[:.]/g, "-")}`;
  fs.copyFileSync(file, backup);

  const pruned = Object.fromEntries(kept.map((key) => [key, entries[key]]));
  const tmp = `${file}.prune-tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(pruned, null, 2)}\n`, {
    mode: 0o600,
  });
  fs.renameSync(tmp, file);

  console.log(`\nBackup:  ${backup}`);
  console.log(`Pruned ${leaked.length} leaked entries; ${kept.length} remain.`);
}

main();
