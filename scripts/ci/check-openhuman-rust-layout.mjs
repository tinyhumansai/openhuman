#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { rootRustTargetNames } from "../lib/root-rust-targets.mjs";

const ROOT = "crates/openhuman-core/src";
const CRATES_ROOT = "crates";
const CORE_MANIFEST = "crates/openhuman-core/Cargo.toml";
// Root `tests/*.rs` / `examples/*.rs` targets are declared by the CLI crate,
// which hosts the binary and every integration suite; the core is a library.
const CLI_MANIFEST = "crates/openhuman-cli/Cargo.toml";
const LINE_LIMIT = 750;
// Files past this size are reported (without failing) so an author sees the
// margin in their own PR's log instead of a later PR turning `main` red.
const WARN_AT = 725;

// Files over `LINE_LIMIT` that still need semantic decomposition. Each pin is
// the file's exact size and is a ratchet, enforced below: a file may not grow
// past its pin, and a file that shrinks must lower its pin in the same change
// (or drop the entry once it fits `LINE_LIMIT`), so recovered lines cannot be
// spent again. No new exception can appear, and one file may be listed only
// once (a Map keeps the last duplicate, which silently discards the other pin).
const LEGACY_LIMIT_ENTRIES = [
  // These orchestration files crossed the general limit in the already-merged
  // runtime compatibility work. They are being split along semantic seams.
  // `spawn_async_subagent_execute.rs`: 847 -> 821 when the argument prologue
  // moved to `spawn_async_subagent_args.rs`, 821 -> 809 via
  // `AbortReport::deliver`, 809 -> 795 via `CompletionTarget`, 795 -> 791 when
  // the detached-usage comments were reflowed. Still exempt because the rest of that function's
  // phases close over locals whose types are not nameable from this module
  // (see that fragment's header); taking it under 750 needs a visibility
  // change in `subagent_sessions`.
  [
    "crates/openhuman-core/src/agent/orchestration/tools/spawn_async_subagent_execute.rs",
    791,
  ],
  // `spawn_subagent_tool_impl.rs` had its entry DELETED, not lowered: the
  // parameter schema moved to `spawn_subagent_parameters.rs` and the file is
  // under the general 750 limit, so it needs no exception at all.
  // The session-todo integration added transcript metadata construction to
  // this already-exempt composition seam. Keep its allowance exact.
  ["crates/openhuman-core/src/agent/session_host/runtime_session.rs", 1293],
  // Session-host factory still assembles the product's deliberately coupled
  // provider, security, memory, tool and prompt policy.  Generic session
  // state moved to tinyagents-runtime; this remaining composition is split in
  // a follow-up without reintroducing an old harness/session exception.
  ["crates/openhuman-core/src/agent/session_host/builder/factory.rs", 973],
  ["crates/openhuman-core/src/agent/subagent_host/lifecycle.rs", 1247],
  ["crates/openhuman-core/src/agent/subagent_host/ops/runner.rs", 1132],
  ["crates/openhuman-core/src/tools/ops.rs", 970],
  ["crates/openhuman-core/src/web_chat/progress_bridge.rs", 1264],
  // These established external test modules grew with upstream coverage. Pin
  // their current sizes while follow-up work separates their test concerns.
  // `core/` was pruned from the line limit by name until these pins; its
  // oversized files are pinned at the size they had when enforcement began.
  ["crates/openhuman-core/src/core/all.rs", 1349],
  ["crates/openhuman-core/src/core/all_tests.rs", 1522],
  ["crates/openhuman-core/src/core/events.rs", 1789],
  ["crates/openhuman-core/src/core/events_tests.rs", 997],
  ["crates/openhuman-core/src/core/observability.rs", 3493],
];
const LEGACY_LIMITS = new Map(LEGACY_LIMIT_ENTRIES);

// Every Rust file under a directory, no exclusions. The line limit applies to
// every file under `ROOT` (an oversized file is pinned in
// `LEGACY_LIMIT_ENTRIES`, never skipped); the naming and inline-test-module
// checks apply to every crate.
function allRustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "target") return [];
      return allRustFiles(file);
    }
    return entry.isFile() && entry.name.endsWith(".rs") ? [file] : [];
  });
}

const INLINE_TEST_MODULE_RE =
  /^\s*#\[cfg\([^\n]*\btest\b[^\n]*\)\]\s*\n(?:\s*#\[[^\n]+\]\s*\n)*\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{/m;

const failures = [];
const warnings = [];
for (const file of allRustFiles(ROOT)) {
  const source = fs.readFileSync(file, "utf8");
  const lineCount = source.split("\n").length - (source.endsWith("\n") ? 1 : 0);
  const portableFile = file.split(path.sep).join("/");
  const legacyLimit = LEGACY_LIMITS.get(portableFile);
  if (lineCount > (legacyLimit ?? LINE_LIMIT)) {
    failures.push(
      `${file}: ${lineCount} lines (limit ${legacyLimit ?? LINE_LIMIT})`,
    );
  } else if (legacyLimit !== undefined && lineCount < legacyLimit) {
    failures.push(
      lineCount <= LINE_LIMIT
        ? `${file}: ${lineCount} lines now fits the ${LINE_LIMIT}-line limit; remove its legacy exception`
        : `${file}: ${lineCount} lines, below its legacy pin ${legacyLimit}; lower the pin to ${lineCount}`,
    );
  } else if (legacyLimit === undefined && lineCount > WARN_AT) {
    warnings.push(
      `${file}: ${lineCount} lines, ${LINE_LIMIT - lineCount} below the ${LINE_LIMIT}-line limit`,
    );
  }
}

for (const file of allRustFiles(CRATES_ROOT)) {
  const source = fs.readFileSync(file, "utf8");
  if (
    ["tests.rs", "test.rs"].includes(path.basename(file)) ||
    path.basename(file).endsWith("_test.rs")
  ) {
    failures.push(
      `${file}: test modules must use a descriptive *_tests.rs filename`,
    );
  }
  if (INLINE_TEST_MODULE_RE.test(source)) {
    failures.push(
      `${file}: inline test module; move it to a sibling *_tests.rs file`,
    );
  }
}

// A Map silently keeps the last of two entries for one file, so a duplicate
// hides whichever pin was written first.
const seenLegacy = new Set();
for (const [file] of LEGACY_LIMIT_ENTRIES) {
  if (seenLegacy.has(file))
    failures.push(`${file}: duplicate legacy exception; keep one entry`);
  seenLegacy.add(file);
}

for (const file of LEGACY_LIMITS.keys()) {
  if (!fs.existsSync(file))
    failures.push(`${file}: stale legacy exception; remove it from the gate`);
}

// Integration tests and examples remain repository-level for now, so Cargo
// cannot auto-discover them. Keep the explicit target list
// exhaustive: otherwise adding a file can make `cargo test` silently run
// nothing for it while still exiting successfully.
const manifest = fs.readFileSync(CLI_MANIFEST, "utf8");
const coreManifest = fs.readFileSync(CORE_MANIFEST, "utf8");
for (const table of ["bin", "test", "example"]) {
  if (coreManifest.includes(`[[${table}]]`))
    failures.push(
      `${CORE_MANIFEST}: [[${table}]] target declared in the library crate; it belongs in ${CLI_MANIFEST}`,
    );
}
function declaredTargets(table) {
  const targets = new Set();
  for (const block of manifest.split(`[[${table}]]`).slice(1)) {
    const name = block.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
    if (name) targets.add(name);
  }
  return targets;
}

for (const [directory, table] of [
  ["tests", "test"],
  ["examples", "example"],
]) {
  const files = rootRustTargetNames(directory);

  const declared = declaredTargets(table);
  for (const name of files) {
    if (!declared.has(name))
      failures.push(
        `${directory}/${name}.rs: missing [[${table}]] entry in ${CLI_MANIFEST}`,
      );
  }
  for (const name of declared) {
    if (!files.has(name))
      failures.push(`${CLI_MANIFEST}: stale [[${table}]] target ${name}`);
  }
}

if (warnings.length) {
  console.warn("OpenHuman Rust layout check: files near the line limit:");
  warnings.forEach((warning) => console.warn(`  - ${warning}`));
}

if (failures.length) {
  console.error("OpenHuman Rust layout check failed:");
  failures.forEach((failure) => console.error(`  - ${failure}`));
  process.exit(1);
}

console.log(
  `OpenHuman Rust layout check passed (new files <= ${LINE_LIMIT} lines; tests external).`,
);
