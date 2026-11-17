#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { stripRustComments } from "./check-crate-chain.mjs";

const CORE = "crates/openhuman-core/src/";
const MANIFEST = "scripts/ci/process-statics.json";
const production = (file) =>
  typeof file === "string" &&
  file.startsWith(CORE) &&
  file.endsWith(".rs") &&
  !file.endsWith("_tests.rs");
const signature = (entry) =>
  JSON.stringify([entry.file, entry.scope ?? "", entry.name, entry.type]);

// Track explicit inline modules through every balanced source brace. Anonymous
// blocks and macro expansion are outside this source-level identity model.
function moduleScopes(code) {
  const openings = new Map();
  for (const match of code.matchAll(
    /\bmod\s+((?:r#)?[_\p{ID_Start}][_\p{ID_Continue}]*)\s*\{/gu,
  ))
    openings.set(match.index + match[0].lastIndexOf("{"), match[1]);
  const stack = [];
  const scopes = [];
  for (let index = 0; index < code.length; index++) {
    if (code[index] === "{") stack.push(openings.get(index) ?? null);
    else if (code[index] === "}") stack.pop();
    if (openings.has(index) || code[index] === "}")
      scopes.push([index, stack.filter(Boolean).join("::")]);
  }
  return scopes;
}

/** Source declarations only: includes aliases and lazy_static!, not macro expansion. */
export function declarations(text, file) {
  const code = stripRustComments(text);
  const found = [];
  const scopes = moduleScopes(code);
  let scopeIndex = 0;
  let scope = "";
  const pattern =
    /(?<!')\bstatic\s+(?:(mut|ref)\s+)?((?:r#)?[_\p{ID_Start}][_\p{ID_Continue}]*)\s*:(?!:)/gu;
  for (const match of code.matchAll(pattern)) {
    while (scopeIndex < scopes.length && scopes[scopeIndex][0] < match.index)
      scope = scopes[scopeIndex++][1];
    let end = match.index + match[0].length;
    const start = end;
    const stack = [];
    const arrayLengths = new Set();
    const macroTrees = new Set();
    for (; end < code.length; end++) {
      const char = code[end];
      if (!stack.length && (char === "=" || char === ";")) break;
      if (char === ";" && stack.at(-1) === "[") arrayLengths.add(stack.length);
      // Operators in const blocks and array lengths are not type brackets.
      if (
        (char === "<" || char === ">") &&
        (stack.includes("{") || arrayLengths.size || macroTrees.size)
      )
        continue;
      if ("<([{".includes(char)) {
        if (
          "([{".includes(char) &&
          code.slice(start, end).trimEnd().endsWith("!")
        )
          macroTrees.add(stack.length + 1);
        stack.push(char);
      } else if (">)]}".includes(char) && stack.length) {
        // An arrow inside a function type is not a generic closing bracket.
        if (char === ">" && code[end - 1] === "-") continue;
        const expected = { ">": "<", ")": "(", "]": "[", "}": "{" }[char];
        if (stack.at(-1) === expected) {
          arrayLengths.delete(stack.length);
          macroTrees.delete(stack.length);
          stack.pop();
        }
      }
    }
    if (end === code.length)
      throw new Error(`${file}: cannot parse static ${match[2]}`);
    const type = stripRustComments(text.slice(start, end), {
      preserveLiterals: true,
      compact: true,
      preserveTokenBoundaries: true,
    });
    if (!type) throw new Error(`${file}: static ${match[2]} has no type`);
    found.push({
      file,
      scope,
      name: match[2],
      type: (match[1] ? match[1] + " " : "") + type,
      line: code.slice(0, match.index).split("\n").length,
    });
  }
  return found;
}

function counts(entries) {
  const result = new Map();
  for (const entry of entries) {
    const key = signature(entry);
    result.set(key, (result.get(key) ?? 0) + 1);
  }
  return result;
}

/** Require exact, non-stale ownership records for added or changed declaration types. */
export function reviewChanges(
  before,
  after,
  approvals,
  previousApprovals = [],
) {
  if (!Array.isArray(approvals))
    throw new Error("process-statics.json must contain an array");
  const oldCounts = counts(before);
  const newCounts = counts(after);
  const reviewed = new Map();
  const errors = [];
  for (const entry of approvals) {
    if (!entry || typeof entry !== "object") {
      errors.push("Invalid ownership record");
      continue;
    }
    const key = signature(entry);
    if (reviewed.has(key)) errors.push(`Duplicate ownership record: ${key}`);
    reviewed.set(key, entry);
    if (
      !production(entry.file ?? "") ||
      (entry.scope !== undefined && typeof entry.scope !== "string") ||
      typeof entry.name !== "string" ||
      !entry.name ||
      typeof entry.type !== "string" ||
      !entry.type ||
      !Number.isInteger(entry.count) ||
      entry.count < 1 ||
      !["A", "B", "C", "D"].includes(entry.category) ||
      !["reason", "verification"].every(
        (field) => typeof entry[field] === "string" && entry[field].trim(),
      )
    ) {
      errors.push(`Incomplete ownership record: ${key}`);
    }
    if (newCounts.get(key) !== entry.count)
      errors.push(`Stale ownership record: ${key}`);
  }
  for (const [key, count] of newCounts) {
    if (
      count > (oldCounts.get(key) ?? 0) &&
      reviewed.get(key)?.count !== count
    ) {
      errors.push(
        `New static requires an ownership record: ${key} (count ${count})`,
      );
    }
  }
  for (const entry of previousApprovals) {
    const key = signature(entry);
    if (newCounts.has(key) && !reviewed.has(key)) {
      errors.push(
        `Prior ownership record must remain while its declaration survives: ${key}`,
      );
    }
  }
  return errors;
}

/** Compare tracked working sources with the merge base; missing history is a failure. */
export function checkRepository(root, baseRef) {
  const git = (args) =>
    execFileSync("git", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  if (!baseRef) {
    if (process.env.GITHUB_BASE_REF)
      baseRef = `origin/${process.env.GITHUB_BASE_REF}`;
    else {
      try {
        git(["rev-parse", "--verify", "upstream/main^{commit}"]);
        baseRef = "upstream/main";
      } catch {
        baseRef = "origin/main";
      }
    }
  }
  let base;
  try {
    base = git(["merge-base", baseRef, "HEAD"]);
  } catch {
    throw new Error(
      `Cannot resolve merge base ${baseRef}; fetch the base with full history before running this gate.`,
    );
  }
  const previousFiles = git([
    "ls-tree",
    "-r",
    "-z",
    "--name-only",
    base,
    "--",
    CORE,
  ])
    .split("\0")
    .filter(production);
  const currentFiles = git(["ls-files", "-z", "--", CORE])
    .split("\0")
    .filter(production);
  if (!currentFiles.length)
    throw new Error(
      "No tracked core production sources; refusing an empty scan.",
    );
  const before = previousFiles.flatMap((file) =>
    declarations(git(["show", `${base}:${file}`]), file),
  );
  const after = currentFiles.flatMap((file) =>
    declarations(readFileSync(resolve(root, file), "utf8"), file),
  );
  const approvals = JSON.parse(readFileSync(resolve(root, MANIFEST), "utf8"));
  const hasPreviousManifest = git([
    "ls-tree",
    "--name-only",
    base,
    "--",
    MANIFEST,
  ]);
  const previousApprovals = hasPreviousManifest
    ? JSON.parse(git(["show", `${base}:${MANIFEST}`]))
    : [];
  if (!Array.isArray(previousApprovals))
    throw new Error("Base ownership manifest must contain an array");
  return {
    base,
    files: currentFiles.length,
    errors: reviewChanges(before, after, approvals, previousApprovals),
  };
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  try {
    const report = checkRepository(process.cwd(), process.argv[2]);
    if (report.errors.length) throw new Error(report.errors.join("\n"));
    console.log(
      `Process statics: ${report.files} core files checked against ${report.base}; ownership records valid.`,
    );
  } catch (error) {
    console.error(`Process statics FAILED: ${error.message}`);
    process.exitCode = 1;
  }
}
