#!/usr/bin/env node
// Build a loadable module from its vendored gitlink, for LOCAL development and
// local tests (tinyhumansai/openhuman#7613, epic #7523).
//
//   node scripts/modules/build.mjs <id|all> [--profile release|dev]
//                                  [--out <dir>] [--print-env] [--force]
//
// For each module it builds the cdylib named by the registry record's
// `SourceSpec` (`cargo build --manifest-path vendor/<submodule>/<manifest>
// -p <package>`), copies it to `<out>/<id>/` and writes an operator-trust
// `modules.toml` beside it holding the file's sha256. The core's `source` mode
// (`OPENHUMAN_MODULE_SOURCE=source` plus `OPENHUMAN_MODULE_SOURCE_DIR=<out>`)
// loads exactly that file; so does `OPENHUMAN_MODULE_PATH=<out>/<id>`.
//
// stdout is KEY=VALUE lines to export (progress goes to stderr); with
// `--print-env` they are `export KEY='value'` lines for `eval "$(...)"`.
//
// Local digests are operator trust, not registry pins: this script never edits
// records_*.rs. CI downloads the pinned releases and must not call it (enforced
// by scripts/ci/check-ci-module-source.mjs).
//
// Cache: a built library is stored under `$CI_CACHE_DIR/blobs` (else
// `<out>/.blobs`) keyed by (submodule tree, uncommitted diff, rustc -vV, target
// triple, profile, features, package, manifest). A hit never invokes cargo.
//
// Target dir: the build uses cargo's normal target directory for the checkout
// (the shared one when `.cargo/config.toml` sets it). No `--target-dir`, and
// CARGO_TARGET_DIR is not set.
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  artifactFilename,
  cacheKey,
  moduleSourceEnv,
  parseSourceSpecs,
  shellQuote,
  trustToml,
} from "../lib/module-sources.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
export const DEFAULT_ROOT = resolve(HERE, "..", "..");

/** The registry source text: registry.rs plus its records_*.rs fragments. */
export function readRegistry(root) {
  const dir = join(root, "crates/openhuman-core/src/modules");
  const fragments = readdirSync(join(dir, "registry"))
    .filter((f) => /^records_.*\.rs$/.test(f))
    .sort()
    .map((f) => readFileSync(join(dir, "registry", f), "utf8"));
  return [readFileSync(join(dir, "registry.rs"), "utf8"), ...fragments].join(
    "\n",
  );
}

export function parseArgs(argv) {
  const opts = {
    target: undefined,
    profile: "release",
    out: undefined,
    printEnv: false,
    force: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--profile") opts.profile = argv[++i];
    else if (arg === "--out") opts.out = argv[++i];
    else if (arg === "--print-env") opts.printEnv = true;
    else if (arg === "--force") opts.force = true;
    else if (arg.startsWith("--")) throw new Error(`unknown option ${arg}`);
    else if (opts.target === undefined) opts.target = arg;
    else throw new Error(`unexpected argument ${arg}`);
  }
  if (!opts.target) throw new Error("usage: build.mjs <id|all> [options]");
  if (!["release", "dev"].includes(opts.profile)) {
    throw new Error(`--profile must be release or dev, got ${opts.profile}`);
  }
  return opts;
}

function run(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { encoding: "utf8", ...options });
  if (result.error) throw result.error;
  return result;
}

function toolchain(env) {
  const out = run(env.RUSTC ?? "rustc", ["-vV"], { env });
  if (out.status !== 0) throw new Error("rustc -vV failed");
  return {
    rustc: out.stdout,
    triple: out.stdout.match(/^host: (.+)$/m)?.[1] ?? "unknown",
  };
}

function gitState(root, submodule) {
  const dir = join(root, "vendor", submodule);
  const tree = run("git", ["-C", dir, "rev-parse", "HEAD^{tree}"]);
  if (tree.status !== 0) {
    throw new Error(
      `vendor/${submodule} is not an initialised checkout (git submodule update --init --recursive vendor/${submodule})`,
    );
  }
  const diff = run("git", ["-C", dir, "diff", "HEAD"]);
  return {
    tree: tree.stdout.trim(),
    dirty: diff.stdout ? diff.stdout : "",
  };
}

/** Run cargo for `spec`; returns the built cdylib's path. */
function cargoBuild({ root, spec, profile, env, log }) {
  const manifest = join(root, "vendor", spec.submodule, spec.manifest);
  const args = [
    "build",
    "--manifest-path",
    manifest,
    "-p",
    spec.package,
    "--message-format=json-render-diagnostics",
  ];
  if (profile === "release") args.push("--release");
  if (spec.features.length) args.push("--features", spec.features.join(","));
  log(`building ${spec.id} (${profile})`);
  const result = run(env.CARGO ?? "cargo", args, {
    cwd: root,
    env,
    maxBuffer: 1 << 28,
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.status !== 0) {
    throw new Error(`cargo build failed for ${spec.id}`);
  }
  const wanted = artifactFilename(spec);
  let built;
  for (const line of result.stdout.split("\n")) {
    if (!line.startsWith("{")) continue;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }
    if (message.reason !== "compiler-artifact") continue;
    if (!message.target?.kind?.includes("cdylib")) continue;
    const file = (message.filenames ?? []).find((f) => f.endsWith(wanted));
    if (file) built = file;
  }
  if (!built) {
    throw new Error(`${spec.id}: cargo produced no ${wanted} for ${spec.package}`);
  }
  return built;
}

/** Build (or reuse) one module and stage it under `<out>/<id>/`. */
export function buildModule({ root, spec, profile, out, blobs, force, env, log }) {
  const { rustc, triple } = toolchain(env);
  const state = gitState(root, spec.submodule);
  const key = cacheKey({
    ...state,
    rustc,
    triple,
    profile,
    features: spec.features,
    spec,
  });
  const filename = artifactFilename(spec);
  mkdirSync(blobs, { recursive: true });
  const blob = join(blobs, `module-${spec.id}-${key.slice(0, 24)}`);
  let cached = false;
  if (!force && existsSync(blob)) {
    cached = true;
    log(`${spec.id}: cache hit ${key.slice(0, 12)}`);
  } else {
    const built = cargoBuild({ root, spec, profile, env, log });
    copyFileSync(built, `${blob}.part`);
    renameSync(`${blob}.part`, blob);
  }
  const dest = join(out, spec.id);
  mkdirSync(dest, { recursive: true });
  const artifact = join(dest, filename);
  copyFileSync(blob, artifact);
  writeFileSync(join(dest, "modules.toml"), trustToml(filename, readFileSync(artifact)));
  return { id: spec.id, dir: dest, artifact, cached, key };
}

export function main(argv, env = process.env, root = DEFAULT_ROOT) {
  const log = (message) => process.stderr.write(`[modules:build] ${message}\n`);
  const opts = parseArgs(argv);
  const specs = parseSourceSpecs(readRegistry(root));
  let ids;
  if (opts.target === "all") ids = [...specs.keys()];
  else if (specs.has(opts.target)) ids = [opts.target];
  else {
    throw new Error(
      `unknown module id "${opts.target}" (known: ${[...specs.keys()].join(", ")})`,
    );
  }
  const out = resolve(opts.out ?? join(root, "target", "module-sources"));
  const blobs = env.CI_CACHE_DIR ? join(env.CI_CACHE_DIR, "blobs") : join(out, ".blobs");
  const built = ids.map((id) =>
    buildModule({
      root,
      spec: specs.get(id),
      profile: opts.profile,
      out,
      blobs,
      force: opts.force,
      env,
      log,
    }),
  );

  const lines = [["OPENHUMAN_MODULE_SOURCE_DIR", out]];
  for (const item of built) lines.push([moduleSourceEnv(item.id), "source"]);
  const search = [...built.map((b) => b.dir), env.OPENHUMAN_MODULE_PATH]
    .filter(Boolean)
    .join(":");
  lines.push(["OPENHUMAN_MODULE_PATH", search]);
  return lines
    .map(([k, v]) => (opts.printEnv ? `export ${k}=${shellQuote(v)}` : `${k}=${v}`))
    .join("\n");
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    process.stdout.write(`${main(process.argv.slice(2))}\n`);
  } catch (error) {
    process.stderr.write(`build.mjs: ${error.message}\n`);
    process.exit(1);
  }
}
