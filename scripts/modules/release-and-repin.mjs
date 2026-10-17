#!/usr/bin/env node
// Release a module from its own repository and re-pin the host to it
// (tinyhumansai/openhuman#7615, epic #7523). This automates Steps 3-4 of the
// migrate-to-submodule skill; the pins stay strict.
//
//   node scripts/modules/release-and-repin.mjs <id> [--bump patch|minor|major]
//                                              [--version X.Y.Z] [--yes]
//                                              [--dry-run] [--root <dir>]
//
// 1. dispatch `release.yml` in tinyhumansai/<id> (`-f bump=...`) and watch it.
//    Publishing tags is hard to undo, so this ASKS FIRST (`--yes` answers it,
//    and a non-interactive shell without `--yes` refuses). With `--version`
//    the release already exists and nothing is dispatched.
// 2. download the release's `checksum.toml`.
// 3. rewrite the record in records_*.rs: `version`, `release_url` and every
//    asset `archive` + `sha256`, VERBATIM from checksum.toml. A digest is never
//    computed here, from a local build or anything else.
// 4. check out the tag in vendor/<sub> and stage the gitlink.
// 5. update the other pins that name the version (module-provider-pins.json for
//    a provider record; a now-stale module-pin-exemptions.json entry).
// 6. run check-module-pins.mjs and check-submodule-monotonic.mjs.
//
// It never commits and never pushes.
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

import { parseRecords } from "../lib/module-pins.mjs";
import {
  bumpVersion,
  dropExemption,
  parseChecksumToml,
  rewriteRecord,
  unusedChecksums,
  updateProviderPins,
} from "../lib/module-repin.mjs";
import { parseSourceSpecs } from "../lib/module-sources.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
export const DEFAULT_ROOT = resolve(HERE, "..", "..");
const ORG = "tinyhumansai";

export function parseArgs(argv) {
  const opts = { bump: "patch", yes: false, dryRun: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--bump") opts.bump = argv[++i];
    else if (arg === "--version") opts.version = argv[++i];
    else if (arg === "--root") opts.root = argv[++i];
    else if (arg === "--yes") opts.yes = true;
    else if (arg === "--dry-run") opts.dryRun = true;
    else if (arg.startsWith("--")) throw new Error(`unknown option ${arg}`);
    else if (opts.id === undefined) opts.id = arg;
    else throw new Error(`unexpected argument ${arg}`);
  }
  if (!opts.id) throw new Error("usage: release-and-repin.mjs <id> [options]");
  bumpVersion("0.0.0", opts.bump); // validates --bump
  if (opts.version && !/^\d+\.\d+\.\d+$/.test(opts.version)) {
    throw new Error(`--version must be X.Y.Z, got ${opts.version}`);
  }
  return opts;
}

/** The records_*.rs file holding `id`, with its text. */
function findRecordFile(root, id) {
  const dir = join(root, "crates/openhuman-core/src/modules/registry");
  for (const name of readdirSync(dir).sort()) {
    if (!/^records_.*\.rs$/.test(name)) continue;
    const path = join(dir, name);
    const text = readFileSync(path, "utf8");
    for (const rec of parseRecords(text).values()) {
      if (rec.id === id) return { path, text, record: rec };
    }
  }
  throw new Error(`no registry record with id "${id}"`);
}

function defaultRun(cmd, args, { cwd, inherit = false } = {}) {
  const res = spawnSync(cmd, args, {
    cwd,
    encoding: "utf8",
    stdio: inherit ? "inherit" : ["ignore", "pipe", "pipe"],
  });
  if (res.error) throw res.error;
  return { code: res.status ?? 1, stdout: res.stdout ?? "", stderr: res.stderr ?? "" };
}

async function defaultConfirm(question) {
  if (!process.stdin.isTTY) return false;
  const rl = createInterface({ input: process.stdin, output: process.stderr });
  try {
    return /^y(es)?$/i.test((await rl.question(`${question} [y/N] `)).trim());
  } finally {
    rl.close();
  }
}

/**
 * Run the whole flow. `deps.run(cmd, args, {cwd})` and `deps.confirm(q)` are
 * injectable so tests never touch GitHub or git.
 */
export async function repin(opts, deps = {}) {
  const root = resolve(opts.root ?? DEFAULT_ROOT);
  const run = deps.run ?? defaultRun;
  const confirm = deps.confirm ?? defaultConfirm;
  const log = deps.log ?? ((m) => process.stderr.write(`${m}\n`));
  const must = (cmd, args, o) => {
    const res = run(cmd, args, o);
    if (res.code !== 0) {
      throw new Error(
        `\`${cmd} ${args.join(" ")}\` failed: ${(res.stderr || res.stdout).trim().split("\n")[0]}`,
      );
    }
    return res.stdout.trim();
  };

  const file = findRecordFile(root, opts.id);
  const registrySrc = readFileSync(file.path, "utf8");
  const spec = parseSourceSpecs(registrySrc).get(opts.id);
  if (!spec) throw new Error(`${opts.id}: record has no SourceSpec submodule`);
  const repo = `${ORG}/${spec.submodule}`;
  const subPath = `vendor/${spec.submodule}`;
  const subDir = join(root, subPath);
  const current = file.record.version;

  let version = opts.version;
  if (opts.dryRun) {
    log(
      `[dry-run] ${opts.id}: registry ${current}; ` +
        (version
          ? `would re-pin to existing release v${version}`
          : `would dispatch release.yml in ${repo} (bump=${opts.bump}) after confirmation`) +
        `; would check out the tag in ${subPath}`,
    );
    return { version: version ?? null, dryRun: true };
  }

  if (!version) {
    const ok =
      opts.yes ||
      (await confirm(
        `Dispatch release.yml in ${repo} with bump=${opts.bump}? This publishes a tag and release artifacts and is hard to undo.`,
      ));
    if (!ok) throw new Error("release not confirmed; nothing was dispatched");
    must("gh", ["workflow", "run", "release.yml", "-R", repo, "-f", `bump=${opts.bump}`]);
    log(`dispatched release.yml in ${repo}; waiting for the run`);
    const runId = must("gh", [
      "run", "list", "-R", repo, "-w", "release.yml", "-L", "1",
      "--json", "databaseId", "-q", ".[0].databaseId",
    ]);
    must("gh", ["run", "watch", runId, "-R", repo, "--exit-status"], { inherit: true });
    const tag = must("gh", ["release", "view", "-R", repo, "--json", "tagName", "-q", ".tagName"]);
    version = tag.replace(/^v/, "");
  }
  const tag = `v${version}`;

  const scratch = mkdtempSync(join(tmpdir(), "repin-"));
  let checksums;
  try {
    must("gh", ["release", "download", tag, "-R", repo, "-p", "checksum.toml", "-D", scratch, "--clobber"]);
    checksums = parseChecksumToml(readFileSync(join(scratch, "checksum.toml"), "utf8"));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }

  const rewritten = rewriteRecord(file.text, opts.id, version, checksums);
  for (const extra of unusedChecksums(rewritten.src, opts.id, checksums)) {
    log(`note: ${tag} publishes ${extra}, which no registry asset uses`);
  }
  writeFileSync(file.path, rewritten.src);
  log(`rewrote ${opts.id} ${current} -> ${version} from checksum.toml`);

  must("git", ["fetch", "--tags"], { cwd: subDir });
  must("git", ["checkout", tag], { cwd: subDir });
  must("git", ["add", subPath], { cwd: root });
  const head = must("git", ["rev-parse", "HEAD"], { cwd: subDir });

  const providersPath = join(root, "scripts/ci/module-provider-pins.json");
  const providers = updateProviderPins(JSON.parse(readFileSync(providersPath, "utf8")), opts.id, version, head);
  if (providers.changed) {
    writeFileSync(providersPath, `${JSON.stringify(providers.json, null, 2)}\n`);
    log("updated scripts/ci/module-provider-pins.json");
  }
  const exemptPath = join(root, "scripts/ci/module-pin-exemptions.json");
  if (existsSync(exemptPath)) {
    const ex = dropExemption(JSON.parse(readFileSync(exemptPath, "utf8")), opts.id);
    if (ex.changed) {
      writeFileSync(exemptPath, `${JSON.stringify(ex.json, null, 2)}\n`);
      log(`dropped the now-stale ${opts.id} entry from module-pin-exemptions.json`);
    }
  }

  // The monotonic gate takes a base ref (default origin/main), not a root.
  const checks = [
    ["check-module-pins.mjs", [root]],
    ["check-submodule-monotonic.mjs", []],
  ];
  const failed = [];
  for (const [script, args] of checks) {
    const res = run("node", [join("scripts/ci", script), ...args], { cwd: root });
    log(`${script}: ${res.code === 0 ? "ok" : "FAILED"}`);
    if (res.code !== 0) {
      failed.push(script);
      log((res.stderr || res.stdout).trim());
    }
  }
  return { version, failed, dryRun: false };
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    const res = await repin(parseArgs(process.argv.slice(2)));
    process.exit(res.failed?.length ? 1 : 0);
  } catch (error) {
    process.stderr.write(`release-and-repin: ${error.message}\n`);
    process.exit(1);
  }
}
