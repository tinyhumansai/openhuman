#!/usr/bin/env node
// Local, advisory pin status for every registry module
// (tinyhumansai/openhuman#7615, epic #7523).
//
//   node scripts/modules/pin-status.mjs [--strict] [--hint] [--print-env] [--root <dir>]
//
// For each module it compares `git -C vendor/<sub> describe --tags` with the
// registry record's `version` and reports one of: in sync, ahead (the gitlink
// is past the released pin: you are running a locally compiled module that CI
// will refuse), behind/diverged, or exempt (a declared exemption).
//
//   --hint       print only the modules that are not in sync (for .husky/pre-push)
//   --print-env  also print OPENHUMAN_MODULE_PIN_AHEAD=<ids>, which a debug core
//                reads to warn once per id when it serves such a module
//   --strict     exit 1 when any module is not in sync/exempt
//
// Without --strict this never fails the shell: it is a hint. The strict gate is
// scripts/ci/check-module-pins.mjs, which is unchanged. A submodule that is not
// checked out is reported as "unavailable", not as a pass.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { parseRecords } from "../lib/module-pins.mjs";
import {
  aheadIds,
  classifyStatus,
  formatStatus,
} from "../lib/module-pin-status.mjs";
import { parseSourceSpecs } from "../lib/module-sources.mjs";
import { readRegistry } from "./build.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
export const DEFAULT_ROOT = resolve(HERE, "..", "..");

function git(args, cwd) {
  try {
    return execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return null;
  }
}

/** Status of every module, in registry order. */
export function collectStatuses(root = DEFAULT_ROOT) {
  const src = readRegistry(root);
  const records = [...parseRecords(src).values()].filter((r) => r.id);
  const specs = parseSourceSpecs(src);
  const exemptions = JSON.parse(
    readFileSync(join(root, "scripts/ci/module-pin-exemptions.json"), "utf8"),
  ).exemptions;
  const byId = new Map((exemptions ?? []).map((e) => [e.id, e]));

  return records.map((rec) => {
    const sub = specs.get(rec.id)?.submodule;
    if (!sub) {
      return {
        id: rec.id,
        kind: "unavailable",
        message: "no SourceSpec submodule in the registry record",
      };
    }
    const path = `vendor/${sub}`;
    const dir = join(root, path);
    if (!existsSync(join(dir, ".git"))) {
      return {
        id: rec.id,
        kind: "unavailable",
        message: `${path} is not checked out`,
      };
    }
    const actual = git(["describe", "--tags", "--abbrev=8", "HEAD"], dir);
    if (!actual) {
      return {
        id: rec.id,
        kind: "unavailable",
        message: `git describe failed in ${path}`,
      };
    }
    const tag = `v${rec.version}`;
    let wantedIsAncestor = null;
    if (git(["rev-parse", "--verify", "--quiet", `refs/tags/${tag}`], dir)) {
      wantedIsAncestor =
        git(["merge-base", "--is-ancestor", tag, "HEAD"], dir) !== null;
    }
    return classifyStatus({
      id: rec.id,
      version: rec.version,
      submodulePath: path,
      actual,
      wantedIsAncestor,
      exemption: byId.get(rec.id),
    });
  });
}

export function main(argv, root = DEFAULT_ROOT) {
  const flags = new Set(argv.filter((a) => a.startsWith("--") && a !== "--root"));
  const rootIdx = argv.indexOf("--root");
  if (rootIdx >= 0) root = resolve(argv[rootIdx + 1]);
  const statuses = collectStatuses(root);
  const okKinds = new Set(["in-sync", "exempt"]);
  const shown = flags.has("--hint")
    ? statuses.filter((s) => !okKinds.has(s.kind))
    : statuses;
  const lines = shown.map(formatStatus);
  if (flags.has("--hint") && lines.length > 0) {
    lines.unshift(
      "module pin status (hint, not blocking; CI runs the strict check-module-pins.mjs):",
    );
    lines.push("fix: node scripts/modules/release-and-repin.mjs <id>");
  }
  if (flags.has("--print-env")) {
    lines.push(`OPENHUMAN_MODULE_PIN_AHEAD=${aheadIds(statuses).join(",")}`);
  }
  const failing = statuses.some((s) => !okKinds.has(s.kind));
  return { lines, code: flags.has("--strict") && failing ? 1 : 0 };
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    const { lines, code } = main(process.argv.slice(2));
    if (lines.length > 0) process.stdout.write(`${lines.join("\n")}\n`);
    process.exit(code);
  } catch (error) {
    // Advisory: a broken hint must not block a push, but must be visible.
    process.stderr.write(`pin-status: ${error.message}\n`);
    process.exit(process.argv.includes("--strict") ? 1 : 0);
  }
}
