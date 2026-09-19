import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

const workflow = fs.readFileSync(
  path.join(repoRoot, ".github", "workflows", "ci-lite.yml"),
  "utf8",
);
const rustCoverage = fs.readFileSync(
  path.join(repoRoot, "scripts", "ci", "rust-coverage.sh"),
  "utf8",
);

test("CI Lite runs the complete frontend suite for frontend changes", () => {
  assert.match(workflow, /pnpm --filter openhuman-app test:coverage/);
  assert.doesNotMatch(workflow, /vitest related|CHANGED_FILES|frontend-src/);
});

test("CI Lite runs the complete Rust suite for Rust-core changes", () => {
  assert.match(workflow, /bash scripts\/ci\/rust-coverage\.sh/);
  assert.doesNotMatch(workflow, /rust-core-src|rust-core-full/);
  assert.doesNotMatch(rustCoverage, /CHANGED_FILES|MAX_CHANGED_FILES/);

  for (const crate of [
    "openhuman",
    "openhuman-embed",
    "openhuman-rpc",
    "openhuman-session",
    "openhuman-tui",
  ]) {
    assert.match(
      rustCoverage,
      new RegExp(`-p ${crate}(?: |\\n)`),
      `${crate} must remain in the complete Rust suite`,
    );
  }
});
