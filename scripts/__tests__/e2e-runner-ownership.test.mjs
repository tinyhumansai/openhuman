import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { checkOwnership } from "../ci/check-e2e-runner-ownership.mjs";

fs.mkdirSync(path.join(process.cwd(), "target"), { recursive: true });
const fixtureRoot = fs.mkdtempSync(path.join(process.cwd(), "target", "e2e-ownership-test-"));
const roots = ["app/test/e2e/specs", "app/test/playwright/specs"];

function fixture(native = [], browser = []) {
  fs.rmSync(fixtureRoot, { recursive: true, force: true });
  for (const root of roots) fs.mkdirSync(path.join(fixtureRoot, root), { recursive: true });
  for (const name of native) fs.writeFileSync(path.join(fixtureRoot, roots[0], name), "native");
  for (const name of browser) fs.writeFileSync(path.join(fixtureRoot, roots[1], name), "browser");
}

test("ownership accepts exactly documented mixed pairs and independent files", () => {
  fixture(["smoke.spec.ts", "native-only.spec.ts"], ["smoke.spec.ts", "browser-only.spec.ts"]);
  assert.deepEqual(checkOwnership(fixtureRoot, [{ basename: "smoke.spec.ts", reason: "Distinct host and browser assertions." }]), []);
});

test("ownership rejects an unapproved duplicate basename", () => {
  fixture(["same.spec.ts"], ["same.spec.ts"]);
  assert.ok(checkOwnership(fixtureRoot, []).some((problem) => problem.includes("unapproved cross-runner duplicate same.spec.ts")));
});

test("ownership rejects stale allowlist entries and missing counterparts", () => {
  fixture(["native-only.spec.ts"], []);
  const stale = checkOwnership(fixtureRoot, [{ basename: "gone.spec.ts", reason: "old split" }]);
  assert.ok(stale.some((problem) => problem.includes("allowlisted file must exist")));
  const missing = checkOwnership(fixtureRoot, [{ basename: "native-only.spec.ts", reason: "split" }]);
  assert.ok(missing.some((problem) => problem.includes("allowlisted file must exist")));
});

test("ownership rejects entries without a reason", () => {
  fixture([], []);
  assert.ok(checkOwnership(fixtureRoot, [{ basename: "missing.spec.ts", reason: " " }]).some((problem) => problem.includes("nonempty reason")));
});

test.after(() => fs.rmSync(fixtureRoot, { recursive: true, force: true }));
