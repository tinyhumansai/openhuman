#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const MIXED_RUNNER_EXCEPTIONS = [
  {
    basename: "smoke.spec.ts",
    reason:
      "Native smoke proves a live Wry window and embedded core; Playwright smoke proves standalone browser bootstrap and RPC.",
  },
  {
    basename: "login-flow.spec.ts",
    reason:
      "Native login proves desktop window and deep-link token handling; Playwright login proves browser callback entry.",
  },
  {
    basename: "auth-access-control.spec.ts",
    reason:
      "Native auth proves desktop deep-link reauthentication and native auth-expiry event delivery; Playwright owns account, billing, logout and revocation UI.",
  },
  {
    basename: "notifications.spec.ts",
    reason:
      "Native notifications prove OS permission state and real plugin calls; Playwright owns rendered notification content and empty-state UI.",
  },
  {
    basename: "settings-data-management.spec.ts",
    reason:
      "Native reset proves the host reset IPC and embedded-core restart; Playwright proves confirmation and cancellation UI.",
  },
];

const defaultRoots = ["app/test/e2e/specs", "app/test/playwright/specs"];

export function checkOwnership(root, exceptions = MIXED_RUNNER_EXCEPTIONS) {
  const problems = [];
  const byName = new Map();
  for (const relativeDir of defaultRoots) {
    const dir = path.join(root, relativeDir);
    if (!fs.existsSync(dir)) {
      problems.push(`spec directory is missing: ${relativeDir}`);
      continue;
    }
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (
        !entry.isFile() ||
        !/\.(?:spec\.(?:ts|tsx|js|mjs)|browser\.(?:ts|tsx|js|mjs))$/.test(
          entry.name,
        )
      )
        continue;
      const matches = byName.get(entry.name) ?? [];
      matches.push(relativeDir);
      byName.set(entry.name, matches);
    }
  }

  const allowlist = new Map();
  for (const exception of exceptions) {
    if (
      !exception?.basename ||
      typeof exception.reason !== "string" ||
      !exception.reason.trim()
    ) {
      problems.push(
        `allowlist entry must have a basename and nonempty reason: ${JSON.stringify(exception)}`,
      );
      continue;
    }
    if (allowlist.has(exception.basename))
      problems.push(`duplicate allowlist entry: ${exception.basename}`);
    allowlist.set(exception.basename, exception.reason);
    const roots = byName.get(exception.basename) ?? [];
    if (roots.length !== defaultRoots.length) {
      problems.push(
        `allowlisted file must exist in both spec roots: ${exception.basename} (found ${roots.length})`,
      );
    }
  }

  for (const [basename, roots] of byName) {
    if (roots.length > 1 && !allowlist.has(basename)) {
      problems.push(
        `unapproved cross-runner duplicate ${basename}: ${roots.join(", ")}`,
      );
    }
    if (roots.length === 1 && allowlist.has(basename)) {
      problems.push(`stale allowlist entry: ${basename}`);
    }
  }
  return problems;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const root = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../..",
  );
  const problems = checkOwnership(root);
  if (problems.length) {
    console.error(problems.map((problem) => `- ${problem}`).join("\n"));
    process.exitCode = 1;
  } else {
    console.log("E2E runner ownership is valid (five documented mixed files).");
  }
}
