import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { rootRustTargetNames } from "../lib/root-rust-targets.mjs";

test("absent root target directories are empty and existing targets remain inventoried", () => {
  const fixture = mkdtempSync(join(tmpdir(), "root-rust-targets-"));
  try {
    const directory = join(fixture, "examples");
    assert.deepEqual([...rootRustTargetNames(directory)], []);
    mkdirSync(directory);
    writeFileSync(join(directory, "review.rs"), "");
    writeFileSync(join(directory, "README.md"), "");
    assert.deepEqual([...rootRustTargetNames(directory)], ["review"]);
    assert.throws(() => rootRustTargetNames(join(directory, "review.rs")), {
      code: "ENOTDIR",
    });
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});
