#!/usr/bin/env node
// SaaS ambient-state ratchet.
//
// A SaaS core serves many users from one process, each under their own
// agent's `CoreContext`. Code that reaches past that context — a bare
// `tokio::spawn` that drops it, a direct `Config::load_or_init`, a process
// environment write, a `home_dir()` lookup — either hands one user's work the
// operator's state or fails in SaaS. Each existing site is baselined exactly;
// a new one fails CI, and a fixed one must be removed from the baseline.
//
// Rules (non-test Rust under crates/openhuman-core/src):
//   bare-spawn      tokio::spawn / tokio::task::spawn / spawn_blocking /
//                   std::thread::spawn — use DispatchCtx::spawn
//   ambient-config  Config::load_or_init( — use load_config_with_timeout or
//                   the config you were handed
//   env-mutation    std::env::set_var / remove_var — process-wide
//   home-dir        dirs::home_dir() / home_dir() — the host's home, not the user's
//   ambient-context CoreContext::current() / .session_agent() outside
//                   core/runtime/ — in SaaS, current() falls back to the
//                   operator's context; key tenant state through
//                   core::runtime::current_tenant / tenant_key instead
//
//   node scripts/ci/check-saas-ambient.mjs
//   node scripts/ci/check-saas-ambient.mjs --write-baseline
//   node scripts/ci/check-saas-ambient.mjs --root <dir>   (tests)

import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const rootFlag = process.argv.indexOf("--root");
const repoRoot =
  rootFlag === -1
    ? resolve(dirname(fileURLToPath(import.meta.url)), "../..")
    : resolve(process.argv[rootFlag + 1]);
const scanRoot = resolve(repoRoot, "crates/openhuman-core/src");
const baselinePath = resolve(repoRoot, "scripts/ci/saas-ambient-baseline.json");
const writeBaseline = process.argv.includes("--write-baseline");

export const RULES = [
  {
    rule: "bare-spawn",
    pattern:
      /\b(?:tokio::spawn|tokio::task::spawn|tokio::task::spawn_blocking|std::thread::spawn)\s*\(/,
    hint: "spawn through DispatchCtx::spawn / spawn_blocking, or justify a baselined bare spawn",
  },
  {
    rule: "ambient-config",
    pattern: /\bConfig::load_or_init\s*\(/,
    hint: "use load_config_with_timeout() or the config you were handed",
  },
  {
    rule: "env-mutation",
    pattern: /\benv::(?:set_var|remove_var)\s*\(/,
    hint: "the process environment is shared by every user",
  },
  {
    rule: "home-dir",
    pattern: /\b(?:dirs::)?home_dir\s*\(\s*\)/,
    hint: "the host's home directory is not a user's",
  },
  {
    rule: "ambient-context",
    pattern: /\bCoreContext::current\s*\(\s*\)|\.session_agent\s*\(\s*\)/,
    hint: "read the tenant through core::runtime::current_tenant() (fails closed in SaaS) and key tables with tenant_key / session_key",
  },
];

// The helpers that implement the scoped spawns are the one place a bare
// spawn belongs. Every other rule still applies to them.
const BARE_SPAWN_EXEMPT = new Set([
  "crates/openhuman-core/src/core/runtime/spawn.rs",
]);

// The runtime owns the context and the tenant rules built on it; the
// ambient-context rule applies everywhere else.
const AMBIENT_CONTEXT_EXEMPT_PREFIX = "crates/openhuman-core/src/core/runtime/";

/** Findings `rel` is exempt from. */
export function exempt(rel, finding) {
  if (finding.rule === "bare-spawn") return BARE_SPAWN_EXEMPT.has(rel);
  if (finding.rule === "ambient-context")
    return rel.startsWith(AMBIENT_CONTEXT_EXEMPT_PREFIX);
  return false;
}

function isTestFile(rel) {
  return (
    rel.endsWith("_tests.rs") ||
    rel.endsWith("_test_support.rs") ||
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
  for (const [index, raw] of source.split(/\r?\n/).entries()) {
    const code = codeOf(raw);
    if (!code.trim()) continue;
    for (const { rule, pattern } of RULES) {
      if (!pattern.test(code)) continue;
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
    findings.push(...found.filter((f) => !exempt(rel, f)));
  }

  if (writeBaseline) {
    await mkdir(dirname(baselinePath), { recursive: true });
    await writeFile(baselinePath, `${JSON.stringify(findings, null, 2)}\n`);
    console.log(
      `Wrote ${findings.length} baselined SaaS ambient-state sites to ${relative(repoRoot, baselinePath)}.`,
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
      `SaaS ambient-state ratchet holds (${findings.length} baselined sites; it only goes down).`,
    );
    return 0;
  }
  if (added.length) {
    console.error(`New SaaS ambient-state sites (${added.length}):`);
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
