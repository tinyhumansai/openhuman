import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("hosted CI installs its pinned Rust components before parallel lanes", () => {
  const workflow = readFileSync(join(root, ".github/workflows/ci-lanes.yml"), "utf8");
  const marker = "      - name: Preflight pinned Rust toolchain before parallel lanes";
  assert.equal(workflow.split(marker).length, 2, "the hosted matrix primes Rust exactly once");

  const preflight = workflow.indexOf(marker);
  const checkout = workflow.indexOf("      - name: Check out submodules");
  const start = workflow.indexOf("      - name: Start lanes", preflight);
  assert.ok(checkout >= 0 && checkout < preflight && preflight < start);
  const nextStep = workflow.indexOf("      - name:", preflight + marker.length);
  assert.ok(nextStep > preflight, "preflight has a separate fail-closed action");
  const step = workflow.slice(preflight, nextStep);
  assert.match(step, /if: matrix\.container != ''/);
  assert.match(step, /cargo --version/);
  assert.match(step, /rustc --version/);
  assert.match(step, /cargo fmt --version/);
  assert.match(step, /cargo clippy --version/);
  assert.doesNotMatch(step, /continue-on-error|\|\| true/);
});
