#!/usr/bin/env node
// Storage-bypass ratchet.
//
// Persistent state goes through the storage port (`crates/openhuman-core/src/
// storage/`, `tinystoragedrivers`), so one backend setting moves all of it:
// SQLite files, a file tree, or a shared MongoDB, with every record under the
// acting agent's scope. Code that opens its own database or writes its own
// file store behind the port makes state a cloud deployment cannot share or
// isolate. Each existing site is baselined exactly; a new one fails CI, and a
// fixed one must be removed from the baseline.
//
// Rules (non-test Rust under crates/openhuman-core/src, outside storage/):
//   sqlite-open  rusqlite `Connection::open(` - use the document port
//   json-write   fs::write / File::create / OpenOptions::new() / NamedTempFile - a raw file
//                store; use the document, stream or secret port
//
// A file may keep a rule when it is a read-only legacy importer, or when its
// state is per-machine, bootstrap or non-record bytes. Each is listed in ALLOW
// with the reason; the reason is the review. A store's own file fallback is
// never allowed: with no installed backend it uses the workspace default
// (`storage::local`).
//
//   node scripts/ci/check-storage-bypass.mjs
//   node scripts/ci/check-storage-bypass.mjs --write-baseline
//   node scripts/ci/check-storage-bypass.mjs --root <dir>   (tests)

import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const rootFlag = process.argv.indexOf("--root");
const repoRoot =
  rootFlag === -1
    ? resolve(dirname(fileURLToPath(import.meta.url)), "../..")
    : resolve(process.argv[rootFlag + 1]);
const scanRoot = resolve(repoRoot, "crates/openhuman-core/src");
const baselinePath = resolve(
  repoRoot,
  "scripts/ci/storage-bypass-baseline.json",
);
const writeBaseline = process.argv.includes("--write-baseline");

export const RULES = [
  {
    rule: "sqlite-open",
    pattern: /\bConnection::open(?:_with_flags)?\s*\(/,
    hint: "persist through the storage port (crate::storage::documents::Repo)",
  },
  {
    rule: "json-write",
    pattern:
      /\b(?:fs::write|File::create|OpenOptions::new|NamedTempFile::new_in)\s*\(/,
    hint: "persist through the storage port, or allowlist it with a reason in ALLOW",
  },
];

const SRC = "crates/openhuman-core/src/";
const LEGACY_READER =
  "the storage importer's read-only reader of a legacy database, opened on a scratch copy of it (never the real file) to bring older installs' rows into the backend";

/**
 * path -> { rule, site, reason }: the one site (its trimmed source line) a rule
 * does not apply to. Pinning the line means a new write in the same file is
 * still a finding, and removing the allowed line does not free a quota for
 * another.
 */
export const ALLOW = new Map(
  [
    [
      "config/schema/profile_layout.rs",
      "json-write",
      'std::fs::write(dir.join(PROFILE_LAYOUT_MARKER), "2\\n")',
      "bootstrap marker that says which on-disk layout a profile directory is; it must be readable before any storage backend can be opened (it decides where the backend lives)",
    ],
    [
      "util/classic_transcript_fixture.rs",
      "json-write",
      'std::fs::write(path, classic_jsonl(messages, meta)).expect("write the classic transcript");',
      "test-only fixture (compiled under cfg(test)) that writes a retired-layout `session_raw/*.jsonl` file for the importer and rewrite tests; no store reads it back",
    ],
    [
      "security/approval/store_import.rs",
      "sqlite-open",
      "let conn = Connection::open(&db_path).with_context(|| {",
      LEGACY_READER,
    ],
    [
      "security/devices/store_import.rs",
      "sqlite-open",
      "let conn = Connection::open(&db_path)",
      LEGACY_READER,
    ],
    [
      "desktop/notifications/store_import.rs",
      "sqlite-open",
      "let conn = Connection::open(&db_path).with_context(|| {",
      LEGACY_READER,
    ],
    [
      "integrations/task_sources/store_import.rs",
      "sqlite-open",
      "let mut conn = Connection::open(&db_path)",
      LEGACY_READER,
    ],
    [
      "modules/storage_import_tinymcp.rs",
      "sqlite-open",
      "Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE)",
      LEGACY_READER,
    ],
    [
      "config/workspace/state.rs",
      "sqlite-open",
      "let conn = Connection::open(db_path)?;",
      "vault watcher state is absolute local paths and their mtimes: per-machine filesystem state that is meaningless on, and must not be shared through, another host's database",
    ],
    [
      "desktop/control/ops.rs",
      "json-write",
      "let mut temporary = tempfile::NamedTempFile::new_in(parent)",
      "this machine's own consent to drive its desktop: per-machine, and it must fail closed rather than follow a shared backend to another host",
    ],
    [
      "desktop/app_state/ops/state_file.rs",
      "json-write",
      "let mut temp_file = NamedTempFile::new_in(parent)",
      "holds the local encryption key and keyring consent, which are needed before any storage backend is open (bootstrap state is never read from storage)",
    ],
    [
      "web3/wallet/ops/state.rs",
      "json-write",
      "let mut temp_file = NamedTempFile::new_in(parent)",
      "paired with the OS keychain mnemonic (and holds it when there is no keychain): moves with the secrets work, not as a plain document",
    ],
    [
      "agent/harness/artifact_offload/mod.rs",
      "json-write",
      "tokio::fs::write(&target, body)",
      "a derived view of a blob-stored artifact, placed in the action dir so file_read can open the path a pointer hands back; the tenant's blob store stays the source of truth",
    ],
    [
      "agent/harness/tool_result_artifacts/mod.rs",
      "json-write",
      "tokio::fs::write(&target, body).await?;",
      "a derived view of a blob-stored tool-result artifact, written under the workspace's read-only artifacts root so file_read can open the key a pointer hands back; re-derived from the tenant's blob store on every read, which stays the source of truth",
    ],
    [
      "modules/browser_task_report.rs",
      "json-write",
      "let mut options = tokio::fs::OpenOptions::new();",
      "opt-in debug trace (`[computer] trace`): the module's full task report, written as a file beside the module's own traces in the same trace directory so a person can open both; page text stays in a file only its owner can read and no store reads it back",
    ],
    [
      "agent/artifacts/files.rs",
      "json-write",
      "match tokio::fs::OpenOptions::new()",
      "reserves the artifact's content file in the user-visible files folder; the bytes stay a file on every backend (the port has no blob store), only the records moved",
    ],
    [
      "agent/artifacts/tiny_store.rs",
      "json-write",
      "tokio::fs::write(&dest, latest.body.as_bytes()).await?;",
      "writes a text artifact's newest version into its file in the user-visible files folder; the deliverable is a user file on every backend, the versioned record itself goes through the artifact store",
    ],
    [
      "media/generation/artifact_tool.rs",
      "json-write",
      "let mut file = tokio::fs::OpenOptions::new()",
      "copies a generated media blob into the artifact's reserved user-visible file; the finalized file remains the artifact bytes on every backend because the storage port has no blob store",
    ],
  ].reduce((map, [path, rule, site, reason]) => {
    const key = `${SRC}${path}`;
    map.set(key, [...(map.get(key) ?? []), { rule, site, reason }]);
    return map;
  }, new Map()),
);

/** Whether `finding` is the allowlisted site of its file (its first occurrence). */
export function allowed(rel, finding) {
  return (ALLOW.get(rel) ?? []).some(
    (entry) =>
      entry.rule === finding.rule &&
      entry.site === finding.text &&
      finding.occurrence === 1,
  );
}

/** Drops the allowlisted site from `rel`'s findings; anything else stays. */
export function withoutAllowed(rel, found) {
  return found.filter((f) => !allowed(rel, f));
}

function isTestFile(rel) {
  return (
    rel.startsWith("crates/openhuman-core/src/storage/") ||
    rel.includes("/tests/") ||
    rel.endsWith("_tests.rs") ||
    rel.includes(`${sep}test_support${sep}`) ||
    rel.includes("/test_support/")
  );
}

/** Code part of a line: drops `//` comments outside string literals. */
export function codeOf(line) {
  let quoted = false;
  let escaped = false;
  for (let i = 0; i < line.length; i += 1) {
    const c = line[i];
    if (c === '"' && !escaped) quoted = !quoted;
    if (!quoted && c === "/" && line[i + 1] === "/") return line.slice(0, i);
    escaped = c === "\\" && !escaped;
  }
  return line;
}

/** Findings in one file's source. */
export function scan(rel, source) {
  const found = [];
  const occurrences = new Map();
  const lines = source.split(/\r?\n/);
  for (const [index, raw] of lines.entries()) {
    const code = codeOf(raw);
    if (!code.trim()) continue;
    // A call may break between the path and its `(`: look one line ahead, and
    // count a match only when it starts on this line.
    const searchable = `${code}\n${codeOf(lines[index + 1] ?? "")}`;
    for (const { rule, pattern } of RULES) {
      const hit = pattern.exec(searchable);
      if (!hit || hit.index >= code.length) continue;
      const text = raw.trim();
      const key = `${rule}\0${text}`;
      const occurrence = (occurrences.get(key) ?? 0) + 1;
      occurrences.set(key, occurrence);
      found.push({ rule, path: rel, line: index + 1, text, occurrence });
    }
  }
  return found;
}

const identity = ({ rule, path, text, occurrence }) =>
  `${rule}\0${path}\0${text}\0${occurrence}`;

/** `added` must fail; `stale` must be removed from the baseline. */
export function compare(findings, baseline) {
  const expected = new Set(baseline.map(identity));
  const actual = new Set(findings.map(identity));
  return {
    added: findings.filter((f) => !expected.has(identity(f))),
    stale: baseline.filter((b) => !actual.has(identity(b))),
  };
}

async function rustFiles(dir) {
  const out = [];
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await rustFiles(path)));
    else if (entry.name.endsWith(".rs")) out.push(path);
  }
  return out.sort();
}

async function main() {
  const findings = [];
  for (const path of await rustFiles(scanRoot)) {
    const rel = relative(repoRoot, path).split(sep).join("/");
    if (isTestFile(rel)) continue;
    const found = scan(rel, await readFile(path, "utf8"));
    findings.push(...withoutAllowed(rel, found));
  }

  if (writeBaseline) {
    await mkdir(dirname(baselinePath), { recursive: true });
    await writeFile(baselinePath, `${JSON.stringify(findings, null, 2)}\n`);
    console.log(
      `Wrote ${findings.length} baselined storage-bypass sites to ${relative(repoRoot, baselinePath)}.`,
    );
    return 0;
  }

  let baseline;
  try {
    baseline = JSON.parse(await readFile(baselinePath, "utf8"));
  } catch (error) {
    console.error(
      `Unable to read ${relative(repoRoot, baselinePath)}: ${error.message}`,
    );
    return 1;
  }

  const { added, stale } = compare(findings, baseline);
  if (added.length === 0 && stale.length === 0) {
    console.log(
      `Storage-bypass ratchet holds (${findings.length} baselined sites; it only goes down).`,
    );
    return 0;
  }
  if (added.length) {
    console.error(`New storage-bypass sites (${added.length}):`);
    for (const f of added) {
      const hint = RULES.find((r) => r.rule === f.rule)?.hint ?? "";
      console.error(
        `  ${f.rule}: ${f.path}:${f.line}: ${f.text}\n    -> ${hint}`,
      );
    }
  }
  if (stale.length) {
    console.error(
      `\nFixed sites still in the baseline (${stale.length}); remove them:`,
    );
    for (const f of stale)
      console.error(`  ${f.rule}: ${f.path}: ${f.text} [${f.occurrence}]`);
    console.error(
      "\nTighten the baseline with --write-baseline once nothing new was added.",
    );
  }
  return 1;
}

if (fileURLToPath(import.meta.url) === resolve(process.argv[1] ?? "")) {
  process.exit(await main());
}
