// Shared helpers for the agent-batch tooling.
// Zero dependencies — Node 20+ stdlib only.

import { readFileSync } from "node:fs";

export const BRANCH_RE = /^cursor\/(a\d{2,3})-(\d+)-[a-z0-9][a-z0-9-]*$/;

const REQUIRED_TOP = [
  "batch_id",
  "base_repo",
  "base_branch",
  "tracking_issue",
  "agents",
];
const REQUIRED_AGENT = ["id", "issue", "title", "branch", "owned_paths"];

export class SpecError extends Error {
  constructor(message, path) {
    super(path ? `${path}: ${message}` : message);
    this.path = path;
  }
}

export function loadSpec(filePath) {
  let text;
  try {
    text = readFileSync(filePath, "utf8");
  } catch (e) {
    throw new SpecError(`cannot read ${filePath}: ${e.message}`);
  }
  let json;
  try {
    json = JSON.parse(text);
  } catch (e) {
    throw new SpecError(`invalid JSON in ${filePath}: ${e.message}`);
  }
  return json;
}

// Validate a parsed spec. Returns the spec on success, throws SpecError on
// any policy violation. The caller is responsible for printing.
export function validateSpec(spec) {
  if (!spec || typeof spec !== "object" || Array.isArray(spec)) {
    throw new SpecError("spec must be a JSON object");
  }
  for (const key of REQUIRED_TOP) {
    if (!(key in spec))
      throw new SpecError(`missing required top-level field "${key}"`);
  }
  if (
    typeof spec.batch_id !== "string" ||
    !/^[a-z0-9][a-z0-9-]*$/.test(spec.batch_id)
  ) {
    throw new SpecError("batch_id must be a kebab-case slug", "batch_id");
  }
  if (spec.base_repo !== "tinyhumansai/openhuman") {
    throw new SpecError(
      `base_repo must be "tinyhumansai/openhuman" (got "${spec.base_repo}")`,
      "base_repo",
    );
  }
  if (spec.base_branch !== "main") {
    throw new SpecError(
      `base_branch must be "main" (got "${spec.base_branch}")`,
      "base_branch",
    );
  }
  if (!Number.isInteger(spec.tracking_issue) || spec.tracking_issue <= 0) {
    throw new SpecError(
      "tracking_issue must be a positive integer",
      "tracking_issue",
    );
  }
  if (!Array.isArray(spec.agents) || spec.agents.length === 0) {
    throw new SpecError("agents must be a non-empty array", "agents");
  }
  if (spec.agents.length > 25) {
    throw new SpecError(
      `batch size ${spec.agents.length} exceeds hard cap of 25`,
      "agents",
    );
  }

  const seenId = new Set();
  const seenIssue = new Set();
  const seenBranch = new Set();
  for (let i = 0; i < spec.agents.length; i++) {
    const agent = spec.agents[i];
    const at = `agents[${i}]`;
    if (!agent || typeof agent !== "object" || Array.isArray(agent)) {
      throw new SpecError("must be an object", at);
    }
    for (const key of REQUIRED_AGENT) {
      if (!(key in agent))
        throw new SpecError(`missing required field "${key}"`, at);
    }
    if (typeof agent.id !== "string" || !/^a\d{2,3}$/.test(agent.id)) {
      throw new SpecError(
        `id must match /^a\\d{2,3}$/ (got "${agent.id}")`,
        `${at}.id`,
      );
    }
    if (seenId.has(agent.id))
      throw new SpecError(`duplicate id "${agent.id}"`, `${at}.id`);
    seenId.add(agent.id);
    if (!Number.isInteger(agent.issue) || agent.issue <= 0) {
      throw new SpecError("issue must be a positive integer", `${at}.issue`);
    }
    if (seenIssue.has(agent.issue)) {
      throw new SpecError(`duplicate issue #${agent.issue}`, `${at}.issue`);
    }
    seenIssue.add(agent.issue);
    if (typeof agent.title !== "string" || agent.title.trim().length === 0) {
      throw new SpecError("title must be a non-empty string", `${at}.title`);
    }
    const m = BRANCH_RE.exec(agent.branch);
    if (!m) {
      throw new SpecError(
        `branch must match cursor/<id>-<issue>-<slug> (got "${agent.branch}")`,
        `${at}.branch`,
      );
    }
    if (m[1] !== agent.id) {
      throw new SpecError(
        `branch id segment "${m[1]}" does not match agent id "${agent.id}"`,
        `${at}.branch`,
      );
    }
    if (Number(m[2]) !== agent.issue) {
      throw new SpecError(
        `branch issue segment "${m[2]}" does not match agent issue ${agent.issue}`,
        `${at}.branch`,
      );
    }
    if (seenBranch.has(agent.branch)) {
      throw new SpecError(`duplicate branch "${agent.branch}"`, `${at}.branch`);
    }
    seenBranch.add(agent.branch);
    if (!Array.isArray(agent.owned_paths) || agent.owned_paths.length === 0) {
      throw new SpecError(
        "owned_paths must be a non-empty array",
        `${at}.owned_paths`,
      );
    }
    for (let j = 0; j < agent.owned_paths.length; j++) {
      const p = agent.owned_paths[j];
      if (typeof p !== "string" || p.length === 0) {
        throw new SpecError(
          "must be a non-empty string",
          `${at}.owned_paths[${j}]`,
        );
      }
      if (p.includes("*") || p.includes("?")) {
        throw new SpecError(
          `globs not allowed — use directory prefixes (got "${p}")`,
          `${at}.owned_paths[${j}]`,
        );
      }
      if (p.startsWith("/")) {
        throw new SpecError(
          `paths must be repo-relative, not absolute (got "${p}")`,
          `${at}.owned_paths[${j}]`,
        );
      }
    }
    if ("allowed_shared_paths" in agent) {
      if (!Array.isArray(agent.allowed_shared_paths)) {
        throw new SpecError("must be an array", `${at}.allowed_shared_paths`);
      }
    }
    if ("labels" in agent && !Array.isArray(agent.labels)) {
      throw new SpecError("must be an array", `${at}.labels`);
    }
  }
  return spec;
}

// Pure overlap detection: returns an array of collisions. Each entry is
// { a, b, pathA, pathB, reason } where reason is "prefix" (one contains the
// other) or "exact" (identical). Empty array = disjoint.
//
// Shared paths in allowed_shared_paths are NOT considered collisions — they
// are escape hatches for unavoidably-shared files like the coverage matrix.
export function findOverlaps(spec) {
  const collisions = [];
  for (let i = 0; i < spec.agents.length; i++) {
    for (let j = i + 1; j < spec.agents.length; j++) {
      const a = spec.agents[i];
      const b = spec.agents[j];
      const sharedA = new Set(a.allowed_shared_paths ?? []);
      const sharedB = new Set(b.allowed_shared_paths ?? []);
      for (const pa of a.owned_paths) {
        if (sharedB.has(pa)) continue;
        for (const pb of b.owned_paths) {
          if (sharedA.has(pb)) continue;
          const c = comparePaths(pa, pb);
          if (c) {
            collisions.push({
              a: a.id,
              b: b.id,
              pathA: pa,
              pathB: pb,
              reason: c,
            });
          }
        }
      }
    }
  }
  return collisions;
}

function comparePaths(p1, p2) {
  if (p1 === p2) return "exact";
  const n1 = p1.endsWith("/") ? p1 : p1 + "/";
  const n2 = p2.endsWith("/") ? p2 : p2 + "/";
  if (n1.startsWith(n2) || n2.startsWith(n1)) return "prefix";
  return null;
}

// CLI helper: parse argv after the script name and return { positional, flags }.
// Recognizes `--flag` and `--flag=value` and `--flag value`.
export function parseArgs(argv) {
  const positional = [];
  const flags = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const eq = a.indexOf("=");
      if (eq !== -1) {
        flags[a.slice(2, eq)] = a.slice(eq + 1);
      } else {
        const key = a.slice(2);
        const next = argv[i + 1];
        if (next === undefined || next.startsWith("--")) {
          flags[key] = true;
        } else {
          flags[key] = next;
          i++;
        }
      }
    } else {
      positional.push(a);
    }
  }
  return { positional, flags };
}
