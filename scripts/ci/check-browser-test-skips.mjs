#!/usr/bin/env node
// Every executable browser-test skip needs its tracking issue on the same line.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const requireFromApp = createRequire(
  new URL("../../app/package.json", import.meta.url),
);
const ts = requireFromApp("typescript");
const issueUrl = /https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/issues\/\d+/;

function scriptKindFor(fileName) {
  const extension = fileName.toLowerCase().slice(fileName.lastIndexOf("."));
  if (extension === ".tsx") return ts.ScriptKind.TSX;
  if (extension === ".jsx") return ts.ScriptKind.JSX;
  if ([".js", ".mjs", ".cjs"].includes(extension)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

export function untrackedSkipLines(source, fileName = "browser-spec.ts") {
  const sourceFile = ts.createSourceFile(
    fileName,
    source,
    ts.ScriptTarget.Latest,
    true,
    scriptKindFor(fileName),
  );
  if (sourceFile.parseDiagnostics.length > 0) {
    const diagnostic = sourceFile.parseDiagnostics[0];
    const location =
      diagnostic.start === undefined
        ? ""
        : ` at line ${sourceFile.getLineAndCharacterOfPosition(diagnostic.start).line + 1}`;
    throw new Error(
      `Could not parse ${fileName}${location}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, " ")}`,
    );
  }
  const lines = source.split("\n");
  const failures = [];

  function visit(node) {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "skip"
    ) {
      const line =
        sourceFile.getLineAndCharacterOfPosition(
          node.expression.name.getStart(sourceFile),
        ).line + 1;
      if (!issueUrl.test(lines[line - 1] ?? "")) {
        failures.push(`${line}: ${lines[line - 1]?.trim() ?? ""}`);
      }
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return failures;
}

if (
  process.argv[1] &&
  import.meta.url === new URL(`file://${process.argv[1]}`).href
) {
  let files;
  try {
    files = execFileSync(
      "git",
      ["ls-files", "app/test/e2e/specs", "app/test/playwright/specs"],
      { encoding: "utf8" },
    )
      .trim()
      .split("\n")
      .filter(Boolean);
  } catch (error) {
    console.error(`Could not list browser specs: ${error.message}`);
    process.exit(2);
  }
  const failures = [];
  for (const file of files) {
    const badLines = untrackedSkipLines(readFileSync(file, "utf8"), file);
    failures.push(...badLines.map((line) => `${file}:${line}`));
  }
  if (failures.length) {
    console.error(
      "Browser-test skips need a GitHub issue URL on the same line:\n" +
        failures.map((line) => `  ${line}`).join("\n"),
    );
    process.exit(1);
  }
}
