/**
 * Unit tests for the mock skill-registry catalog and SKILL.md routes.
 *
 * Run via:
 *   node --test scripts/mock-api/routes/__tests__/skill-registry.test.mjs
 */

import assert from "node:assert/strict";
import test from "node:test";

import { resetMockBehavior, setMockBehavior } from "../../state.mjs";
import { handleIntegrations } from "../integrations.mjs";

function createRes() {
  return {
    statusCode: 0,
    headers: {},
    body: "",
    writeHead(status, headers = {}) {
      this.statusCode = status;
      this.headers = { ...this.headers, ...headers };
    },
    setHeader(name, value) {
      this.headers[name] = value;
    },
    end(chunk = "") {
      this.body += String(chunk);
    },
  };
}

function get(path) {
  const ctx = {
    method: "GET",
    url: path,
    body: "",
    parsedBody: null,
    res: createRes(),
  };
  const handled = handleIntegrations(ctx);
  return { handled, res: ctx.res };
}

test.beforeEach(() => {
  resetMockBehavior();
});

test("the catalog spans more than one 25-entry page", () => {
  const { handled, res } = get("/skills/catalog.json");
  assert.equal(handled, true);
  assert.equal(res.statusCode, 200);
  const entries = JSON.parse(res.body);
  assert.ok(entries.length > 25, `only ${entries.length} entries`);
  assert.ok(entries.some((entry) => entry.name === "git-workflow"));
  assert.ok(entries.some((entry) => entry.name === "docker-management"));
});

test("the catalog answers 503 while the registry is marked unavailable", () => {
  setMockBehavior("skillRegistryUnavailable", "true");
  const { res } = get("/skills/catalog.json");
  assert.equal(res.statusCode, 503);
});

test("each catalog entry has a SKILL.md and unknown names 404", () => {
  const found = get("/skills/git-workflow/SKILL.md");
  assert.equal(found.handled, true);
  assert.equal(found.res.statusCode, 200);
  assert.match(found.res.body, /^---\nname: git-workflow\n/);

  const missing = get("/skills/nope/SKILL.md");
  assert.equal(missing.res.statusCode, 404);
});

test("the skill named by skillRegistryScanBlocked carries an invisible code point", () => {
  setMockBehavior("skillRegistryScanBlocked", "git-workflow");
  const blocked = get("/skills/git-workflow/SKILL.md");
  assert.equal(blocked.res.statusCode, 200);
  assert.ok(blocked.res.body.includes("\u200b"));

  const clean = get("/skills/docker-management/SKILL.md");
  assert.ok(!clean.res.body.includes("\u200b"));
});

test("skillRegistryScanVariant changes the blocked document text", () => {
  setMockBehavior("skillRegistryScanBlocked", "git-workflow");
  const before = get("/skills/git-workflow/SKILL.md").res.body;
  setMockBehavior("skillRegistryScanVariant", "2");
  const after = get("/skills/git-workflow/SKILL.md").res.body;
  assert.notEqual(after, before);
  assert.ok(after.includes("\u200b"));
});
