import { test } from "node:test";
import assert from "node:assert/strict";

import {
  validateSpec,
  findOverlaps,
  parseArgs,
  SpecError,
  BRANCH_RE,
} from "../lib.mjs";

function baseSpec(overrides = {}) {
  return {
    batch_id: "pilot-test",
    base_repo: "tinyhumansai/openhuman",
    base_branch: "main",
    tracking_issue: 1480,
    agents: [
      {
        id: "a01",
        issue: 100,
        title: "fix foo",
        branch: "cursor/a01-100-fix-foo",
        owned_paths: ["src/openhuman/foo/"],
      },
      {
        id: "a02",
        issue: 101,
        title: "fix bar",
        branch: "cursor/a02-101-fix-bar",
        owned_paths: ["app/src/components/bar/"],
      },
    ],
    ...overrides,
  };
}

test("validateSpec accepts a well-formed spec", () => {
  const spec = baseSpec();
  assert.strictEqual(validateSpec(spec), spec);
});

test("validateSpec rejects wrong base_repo", () => {
  const spec = baseSpec({ base_repo: "somefork/openhuman" });
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects wrong base_branch", () => {
  const spec = baseSpec({ base_branch: "develop" });
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects non-positive tracking_issue", () => {
  const spec = baseSpec({ tracking_issue: 0 });
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects empty agents array", () => {
  const spec = baseSpec({ agents: [] });
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects batch larger than the hard cap", () => {
  const agents = Array.from({ length: 26 }, (_, i) => {
    const id = `a${String(i + 1).padStart(2, "0")}`;
    return {
      id,
      issue: 1000 + i,
      title: "t",
      branch: `cursor/${id}-${1000 + i}-x`,
      owned_paths: [`src/openhuman/dom${i}/`],
    };
  });
  assert.throws(() => validateSpec(baseSpec({ agents })), SpecError);
});

test("validateSpec rejects malformed agent id", () => {
  const spec = baseSpec();
  spec.agents[0].id = "agent-one";
  spec.agents[0].branch = "cursor/agent-one-100-fix-foo";
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects branch whose id segment does not match agent id", () => {
  const spec = baseSpec();
  spec.agents[0].branch = "cursor/a99-100-fix-foo";
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects branch whose issue segment does not match agent issue", () => {
  const spec = baseSpec();
  spec.agents[0].branch = "cursor/a01-999-fix-foo";
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects duplicate agent ids", () => {
  const spec = baseSpec();
  spec.agents[1].id = "a01";
  spec.agents[1].branch = "cursor/a01-101-fix-bar";
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects duplicate issues", () => {
  const spec = baseSpec();
  spec.agents[1].issue = 100;
  spec.agents[1].branch = "cursor/a02-100-fix-bar";
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects glob characters in owned_paths", () => {
  const spec = baseSpec();
  spec.agents[0].owned_paths = ["src/openhuman/**"];
  assert.throws(() => validateSpec(spec), SpecError);
});

test("validateSpec rejects absolute paths", () => {
  const spec = baseSpec();
  spec.agents[0].owned_paths = ["/etc/passwd"];
  assert.throws(() => validateSpec(spec), SpecError);
});

test("findOverlaps returns empty for disjoint prefixes", () => {
  assert.deepStrictEqual(findOverlaps(baseSpec()), []);
});

test("findOverlaps detects identical paths", () => {
  const spec = baseSpec();
  spec.agents[1].owned_paths = ["src/openhuman/foo/"];
  const collisions = findOverlaps(spec);
  assert.strictEqual(collisions.length, 1);
  assert.strictEqual(collisions[0].reason, "exact");
});

test("findOverlaps detects prefix containment in either direction", () => {
  const spec = baseSpec();
  spec.agents[1].owned_paths = ["src/openhuman/foo/sub/"];
  const collisions = findOverlaps(spec);
  assert.strictEqual(collisions.length, 1);
  assert.strictEqual(collisions[0].reason, "prefix");
});

test("findOverlaps ignores paths listed in the other agent's allowed_shared_paths", () => {
  const spec = baseSpec();
  spec.agents[0].owned_paths = ["docs/TEST-COVERAGE-MATRIX.md"];
  spec.agents[1].owned_paths = ["docs/TEST-COVERAGE-MATRIX.md"];
  spec.agents[0].allowed_shared_paths = ["docs/TEST-COVERAGE-MATRIX.md"];
  spec.agents[1].allowed_shared_paths = ["docs/TEST-COVERAGE-MATRIX.md"];
  assert.deepStrictEqual(findOverlaps(spec), []);
});

test("BRANCH_RE accepts well-formed branches", () => {
  assert.ok(BRANCH_RE.test("cursor/a01-1234-short-title"));
  assert.ok(BRANCH_RE.test("cursor/a100-99-x"));
});

test("BRANCH_RE rejects malformed branches", () => {
  assert.ok(!BRANCH_RE.test("feature/foo"));
  assert.ok(!BRANCH_RE.test("cursor/a01-1234"));
  assert.ok(!BRANCH_RE.test("cursor/a01-abc-foo"));
  assert.ok(!BRANCH_RE.test("cursor/A01-1234-foo"));
});

test("parseArgs splits positional, --flag, --flag=v, --flag v", () => {
  const r = parseArgs([
    "spec.json",
    "--post",
    "--fixture=foo.json",
    "--agent",
    "a02",
  ]);
  assert.deepStrictEqual(r.positional, ["spec.json"]);
  assert.strictEqual(r.flags.post, true);
  assert.strictEqual(r.flags.fixture, "foo.json");
  assert.strictEqual(r.flags.agent, "a02");
});
