#!/usr/bin/env node
// Fails when CI selects anything but the pinned, digest-checked release for
// loadable modules (tinyhumansai/openhuman#7604, epic #7523).
//
// `[modules] source` has three values: `release` (GitHub-released, pinned,
// digest-checked; the default), and `linked` / `source` (local development and
// local tests only). CI and shipped builds must run `release`, so nothing under
// `.github/workflows/` or the CI scripts may set:
//
//   - OPENHUMAN_MODULE_SOURCE or OPENHUMAN_MODULE_SOURCE_<ID>
//   - a `linked` / `source` override (`source = "linked"`, `"source": "source"`)
//   - the `linked-modules` feature
//   - scripts/modules/build.mjs, the local source build (E0.4)
//
// Comment lines are ignored, so a workflow may explain the rule. Pure text
// scan, no network.
//
// Usage: check-ci-module-source.mjs [repo-root]
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

const SELF = "scripts/ci/check-ci-module-source.mjs";
// check-module-boundaries.mjs only asks `cargo tree` for the dependency graph
// of the isolated `linked-modules` feature to prove the graph is rooted at
// openhuman-cli and keeps the feature out of every shipped host. It never
// builds, runs or selects a linked module source, so the name is the subject
// of the audit, not a CI override.
const EXEMPT = new Set(["scripts/ci/check-module-boundaries.mjs"]);

export const RULES = [
  {
    name: "module-source-env",
    re: /\bOPENHUMAN_MODULE_SOURCE(?:_[A-Z0-9_]+)?\b/,
    why: "sets a module source through the environment",
  },
  {
    name: "non-release-override",
    re: /\bsource["']?\s*[=:]\s*["'](?:linked|source)["']/,
    why: "selects a linked/source module override",
  },
  {
    name: "module-build-script",
    re: /\bmodules\/build\.mjs\b/,
    why: "runs the local module build script (CI uses the pinned releases)",
  },
  {
    name: "linked-modules-feature",
    re: /\blinked-modules\b/,
    why: "enables the linked-modules feature",
  },
];

const isComment = (line) => /^\s*(#|\/\/|\*|\/\*)/.test(line);

/** Violations in one file's text: `{ file, line, rule, why, text }`. */
export function scan(text, file) {
  const found = [];
  text.split("\n").forEach((line, index) => {
    if (isComment(line)) return;
    for (const rule of RULES) {
      if (rule.re.test(line))
        found.push({
          file,
          line: index + 1,
          rule: rule.name,
          why: rule.why,
          text: line.trim().slice(0, 160),
        });
    }
  });
  return found;
}

function walk(dir, out = []) {
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of entries) {
    const path = join(dir, name);
    if (name === "node_modules" || name === "__tests__") continue;
    if (statSync(path).isDirectory()) walk(path, out);
    else out.push(path);
  }
  return out;
}

/** Every file CI executes or configures: workflows and `scripts/ci/**`. */
export function ciFiles(root) {
  const files = [
    ...walk(join(root, ".github", "workflows")),
    ...walk(join(root, "scripts", "ci")),
    join(root, "scripts", "ci-cancel-aware.sh"),
  ];
  return files
    .map((path) => relative(root, path).split(sep).join("/"))
    .filter((file) => file !== SELF && !EXEMPT.has(file));
}

export function check(root) {
  const violations = [];
  for (const file of ciFiles(root)) {
    let text;
    try {
      text = readFileSync(join(root, file), "utf8");
    } catch {
      continue;
    }
    violations.push(...scan(text, file));
  }
  return violations;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const root = resolve(process.argv[2] ?? ".");
  const violations = check(root);
  if (violations.length) {
    for (const v of violations)
      console.error(`${v.file}:${v.line}: ${v.why} [${v.rule}]\n    ${v.text}`);
    console.error(
      `\ncheck-ci-module-source: ${violations.length} violation(s). CI runs the pinned release only; linked/source are for local development.`,
    );
    process.exit(1);
  }
  console.log("check-ci-module-source: CI selects the pinned release only");
}
