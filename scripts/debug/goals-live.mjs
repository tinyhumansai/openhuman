#!/usr/bin/env node
// Live test harness for the memory_goals domain.
//
// Exercises the goal list lifecycle (list / add / edit / delete) and the
// turn-based enrichment agent (reflect) against a running core over JSON-RPC.
// Surfaces the goals_agent's thoughts + tool calls and per-session token
// usage (input / output / cached) and cost so you can iterate on the
// goals_agent prompt and watch what it actually does.
//
// Usage examples:
//   pnpm debug goals-live --spawn-core
//   node scripts/debug/goals-live.mjs                       # attach to running core
//   node scripts/debug/goals-live.mjs --reset --show-thoughts
//   node scripts/debug/goals-live.mjs --case reflect \
//       --context "User keeps asking about shipping the desktop app and wants daily standups."
//
// No credential bodies are printed. Goal text and agent thoughts ARE printed
// (this is a debug tool) — point it at a scratch workspace if that matters.

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, mkdir, readFile, readdir, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_RPC_URL = "http://127.0.0.1:7788/rpc";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ALL_CASES = ["list", "add", "edit", "delete", "reflect", "list-final"];

function usage() {
  return `Usage: node scripts/debug/goals-live.mjs [options]

Live-tests the memory_goals flow (list/add/edit/delete + reflect enrichment)
and prints the goals_agent's thoughts, tool calls, token usage and cost.

Options:
  --core-url <url>     JSON-RPC endpoint (default: OPENHUMAN_CORE_RPC_URL or ${DEFAULT_RPC_URL})
  --token <token>      RPC bearer (default: OPENHUMAN_CORE_TOKEN or <workspace>/core.token)
  --workspace <path>   Workspace whose MEMORY_GOALS.md + transcripts are used
  --spawn-core         Start openhuman-core for the run (uses the real workspace unless --isolated-workspace)
  --isolated-workspace With --spawn-core, use a throwaway temp workspace (needs provider creds in env to enrich)
  --keep-workspace     Do not delete the temp workspace afterwards
  --case <name>        Run only one case: ${ALL_CASES.join(", ")} (repeatable)
  --reset              Delete all existing goals before running
  --context <text>     Context/prompt handed to the reflect enrichment agent
  --model <model>      model_override for the reflect agent (forwarded if supported)
  --show-thoughts      Print the goals_agent transcript thread (thoughts + tool calls + results)
  --rpc-timeout-ms <n> Per-RPC timeout in ms (default: 600000)
  --verbose            Stream spawned core logs
  -h, --help           Show this help
`;
}

function parseArgs(argv) {
  const opts = {
    coreUrl: process.env.OPENHUMAN_CORE_RPC_URL || DEFAULT_RPC_URL,
    token: process.env.OPENHUMAN_CORE_TOKEN || "",
    workspace: process.env.OPENHUMAN_WORKSPACE || "",
    spawnCore: false,
    isolatedWorkspace: false,
    keepWorkspace: false,
    cases: [],
    reset: false,
    context: "",
    model: "",
    showThoughts: false,
    rpcTimeoutMs: 600_000,
    verbose: false,
    coreUrlExplicit: Boolean(process.env.OPENHUMAN_CORE_RPC_URL),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const value = argv[++i];
      if (value === undefined) throw new Error(`missing value for ${arg}`);
      return value;
    };
    switch (arg) {
      case "--core-url":
        opts.coreUrl = next();
        opts.coreUrlExplicit = true;
        break;
      case "--token":
        opts.token = next();
        break;
      case "--workspace":
        opts.workspace = next();
        break;
      case "--spawn-core":
        opts.spawnCore = true;
        break;
      case "--isolated-workspace":
        opts.isolatedWorkspace = true;
        break;
      case "--keep-workspace":
        opts.keepWorkspace = true;
        break;
      case "--case": {
        const name = next();
        if (!ALL_CASES.includes(name))
          throw new Error(`unknown case '${name}' (valid: ${ALL_CASES.join(", ")})`);
        opts.cases.push(name);
        break;
      }
      case "--reset":
        opts.reset = true;
        break;
      case "--context":
        opts.context = next();
        break;
      case "--model":
        opts.model = next();
        break;
      case "--show-thoughts":
        opts.showThoughts = true;
        break;
      case "--rpc-timeout-ms":
        opts.rpcTimeoutMs = Number(next());
        break;
      case "--verbose":
        opts.verbose = true;
        break;
      case "-h":
      case "--help":
        console.log(usage());
        process.exit(0);
      // eslint-disable-next-line no-fallthrough
      default:
        throw new Error(`unknown option: ${arg}`);
    }
  }
  if (opts.cases.length === 0) opts.cases = [...ALL_CASES];
  return opts;
}

// ── workspace / token resolution (mirrors harness-cache-audit) ──────────────

function defaultOpenhumanDir() {
  return process.env.OPENHUMAN_APP_ENV === "staging"
    ? path.join(homedir(), ".openhuman-staging")
    : path.join(homedir(), ".openhuman");
}

async function defaultWorkspace() {
  if (process.env.OPENHUMAN_WORKSPACE) return process.env.OPENHUMAN_WORKSPACE;
  const dir = defaultOpenhumanDir();
  try {
    const active = await readFile(path.join(dir, "active_user.toml"), "utf8");
    const match = active.match(/^\s*user_id\s*=\s*"([^"]+)"\s*$/m);
    if (match?.[1]) return path.join(dir, "users", match[1], "workspace");
  } catch {
    // fall through
  }
  return dir;
}

async function readToken(opts) {
  if (opts.token.trim()) return opts.token.trim();
  const tokenPath = path.join(
    opts.workspace || (await defaultWorkspace()),
    "core.token",
  );
  try {
    return (await readFile(tokenPath, "utf8")).trim();
  } catch {
    throw new Error(
      `RPC token not found at ${tokenPath}. Pass --token or set OPENHUMAN_CORE_TOKEN.`,
    );
  }
}

async function rpc(coreUrl, token, method, params, timeoutMs = 600_000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  let res;
  try {
    res = await fetch(coreUrl, {
      method: "POST",
      signal: controller.signal,
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: `goals-${Date.now()}-${Math.random().toString(16).slice(2)}`,
        method,
        params,
      }),
    });
  } catch (err) {
    if (err?.name === "AbortError")
      throw new Error(`RPC ${method} timed out after ${timeoutMs}ms`);
    throw err;
  } finally {
    clearTimeout(timer);
  }
  const text = await res.text();
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Error(`RPC ${method} returned non-JSON HTTP ${res.status}: ${text.slice(0, 200)}`);
  }
  if (!res.ok) throw new Error(`RPC ${method} HTTP ${res.status}`);
  if (body.error)
    throw new Error(`RPC ${method} error: ${body.error.message || JSON.stringify(body.error)}`);
  return body.result;
}

// RpcOutcome serializes either as the bare value (no logs) or { result, logs }.
function unwrap(result) {
  if (result && typeof result === "object" && "result" in result && "logs" in result) {
    return { value: result.result, logs: result.logs || [] };
  }
  return { value: result, logs: [] };
}

function renderGoals(doc) {
  const items = doc?.items || [];
  if (items.length === 0) return "    (no goals)";
  return items.map((g) => `    - [${g.id}] ${g.text}`).join("\n");
}

// ── transcript auditing ─────────────────────────────────────────────────────

function num(v) {
  return Number.isFinite(Number(v)) ? Number(v) : 0;
}

async function walkJsonl(dir) {
  const out = [];
  async function walk(cur) {
    let entries;
    try {
      entries = await readdir(cur, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const full = path.join(cur, e.name);
      if (e.isDirectory()) await walk(full);
      else if (e.isFile() && e.name.endsWith(".jsonl")) out.push(full);
    }
  }
  await walk(dir);
  return out;
}

async function readTranscript(file) {
  const data = await readFile(file, "utf8");
  const lines = data.split(/\r?\n/).filter((l) => l.trim().length > 0);
  const meta = JSON.parse(lines[0])._meta || {};
  const messages = [];
  for (const line of lines.slice(1)) {
    try {
      const obj = JSON.parse(line);
      if (obj.role) messages.push(obj);
    } catch {
      // skip malformed line
    }
  }
  return {
    file,
    agent: String(meta.agent || "(unknown)"),
    input: num(meta.input_tokens),
    output: num(meta.output_tokens),
    cached: num(meta.cached_input_tokens),
    charged: num(meta.charged_amount_usd),
    messages,
  };
}

async function snapshot(workspace) {
  const files = await walkJsonl(path.join(workspace, "session_raw"));
  const map = new Map();
  await Promise.all(
    files.map(async (f) => {
      try {
        map.set(f, await readTranscript(f));
      } catch {
        // ignore
      }
    }),
  );
  return map;
}

function changedTranscripts(before, after) {
  const rows = [];
  for (const [file, cur] of after.entries()) {
    const prev = before.get(file);
    if (!prev) {
      rows.push(cur);
      continue;
    }
    if (
      cur.input !== prev.input ||
      cur.output !== prev.output ||
      cur.messages.length !== prev.messages.length
    ) {
      rows.push(cur);
    }
  }
  return rows;
}

function printThoughts(transcript) {
  console.log(`\n  ── goals_agent thread (${path.basename(transcript.file)}) ──`);
  for (const msg of transcript.messages) {
    const role = (msg.role || "?").toUpperCase();
    const content = String(msg.content || "").trim();
    if (role === "SYSTEM") {
      console.log(`  [system] (${content.length} chars of system prompt — hidden)`);
      continue;
    }
    if (!content) continue;
    const indented = content
      .split("\n")
      .map((l) => `      ${l}`)
      .join("\n");
    console.log(`  [${role.toLowerCase()}]\n${indented}`);
  }
}

function printUsageTable(rows) {
  if (rows.length === 0) {
    console.log("  (no transcript usage recorded — provider may not have emitted usage)");
    return;
  }
  console.table(
    rows.map((r) => ({
      agent: r.agent,
      input: r.input,
      output: r.output,
      cached: r.cached,
      hit_rate: r.input > 0 ? `${((r.cached / r.input) * 100).toFixed(1)}%` : "0.0%",
      cost_usd: `$${r.charged.toFixed(6)}`,
    })),
  );
}

// ── spawn-core (mirrors harness-cache-audit) ────────────────────────────────

async function pickFreePort() {
  return await new Promise((resolve, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      server.close(() => resolve(port));
    });
  });
}

async function startCore(opts) {
  const token = opts.token || `goals-${randomBytes(24).toString("hex")}`;
  const env = { ...process.env, OPENHUMAN_CORE_TOKEN: token };
  if (opts.workspace) env.OPENHUMAN_WORKSPACE = opts.workspace;
  const port = new URL(opts.coreUrl).port || "7788";
  env.OPENHUMAN_CORE_PORT = port;
  env.OPENHUMAN_CORE_RPC_URL = opts.coreUrl;
  const args = ["run", "--host", "127.0.0.1", "--port", port, "--jsonrpc-only"];
  const child = spawn(
    "cargo",
    ["run", "--quiet", "--bin", "openhuman-core", "--", ...args],
    {
      cwd: path.resolve(SCRIPT_DIR, "../.."),
      env,
      stdio: opts.verbose ? ["ignore", "inherit", "inherit"] : ["ignore", "ignore", "pipe"],
    },
  );
  let stderr = "";
  if (child.stderr) {
    child.stderr.on("data", (c) => {
      stderr += c.toString();
      if (stderr.length > 8000) stderr = stderr.slice(-8000);
    });
  }
  await waitForCore(opts.coreUrl, token, child, () => stderr);
  return { child, token };
}

async function waitForCore(coreUrl, token, child, stderrFn) {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null)
      throw new Error(`spawned core exited with ${child.exitCode}\n${stderrFn()}`);
    try {
      await rpc(coreUrl, token, "core.ping", {}, 10_000);
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 750));
    }
  }
  throw new Error(`timed out waiting for core at ${coreUrl}\n${stderrFn()}`);
}

async function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  const exited = await Promise.race([
    once(child, "exit").then(() => true),
    new Promise((r) => setTimeout(() => r(false), 5000)),
  ]);
  if (exited) return;
  child.kill("SIGKILL");
  await Promise.race([once(child, "exit"), new Promise((r) => setTimeout(r, 2000))]);
}

// ── cases ───────────────────────────────────────────────────────────────────

function call(opts, method, params = {}) {
  return rpc(opts.coreUrl, opts.token, method, params, opts.rpcTimeoutMs);
}

async function resetGoals(opts) {
  const { value } = unwrap(await call(opts, "openhuman.memory_goals_list"));
  const items = value?.items || [];
  for (const g of items) {
    await call(opts, "openhuman.memory_goals_delete", { id: g.id });
  }
  if (items.length > 0) console.log(`  reset: deleted ${items.length} existing goal(s)`);
}

async function caseList(opts, label = "list") {
  console.log(`\n=== case: ${label} ===`);
  const { value } = unwrap(await call(opts, "openhuman.memory_goals_list"));
  console.log(`  current goals:\n${renderGoals(value)}`);
  return value;
}

async function caseAdd(opts) {
  console.log("\n=== case: add ===");
  const seeds = [
    "Help the user ship the OpenHuman desktop app across macOS, Windows and Linux",
    "Keep the Rust core the single source of truth for business logic",
  ];
  for (const text of seeds) {
    const { value, logs } = unwrap(
      await call(opts, "openhuman.memory_goals_add", { text }),
    );
    console.log(`  + ${logs[0] || "added"} -> ${value.id}`);
  }
  const { value } = unwrap(await call(opts, "openhuman.memory_goals_list"));
  console.log(`  goals now:\n${renderGoals(value)}`);
  return value;
}

async function caseEdit(opts) {
  console.log("\n=== case: edit ===");
  const { value: before } = unwrap(await call(opts, "openhuman.memory_goals_list"));
  const target = before?.items?.[0];
  if (!target) {
    console.log("  (no goal to edit — run the add case first)");
    return before;
  }
  const { value, logs } = unwrap(
    await call(opts, "openhuman.memory_goals_edit", {
      id: target.id,
      text: `${target.text} (edited live at ${new Date().toISOString()})`,
    }),
  );
  console.log(`  ~ ${logs[0] || "edited"}`);
  console.log(`  goals now:\n${renderGoals(value)}`);
  return value;
}

async function caseDelete(opts) {
  console.log("\n=== case: delete ===");
  const { value: before } = unwrap(await call(opts, "openhuman.memory_goals_list"));
  const target = before?.items?.at(-1);
  if (!target) {
    console.log("  (no goal to delete)");
    return before;
  }
  const { value, logs } = unwrap(
    await call(opts, "openhuman.memory_goals_delete", { id: target.id }),
  );
  console.log(`  - ${logs[0] || "deleted"} (${target.id})`);
  console.log(`  goals now:\n${renderGoals(value)}`);
  return value;
}

async function caseReflect(opts) {
  console.log("\n=== case: reflect (turn-based enrichment agent) ===");
  if (opts.context) console.log(`  context: ${opts.context}`);
  else console.log("  context: (default review nudge)");

  const before = await snapshot(opts.workspace);
  const params = {};
  if (opts.context) params.context = opts.context;
  if (opts.model) params.model_override = opts.model;

  const started = Date.now();
  let result;
  try {
    result = unwrap(await call(opts, "openhuman.memory_goals_reflect", params)).value;
  } catch (err) {
    console.error(`  reflect RPC failed: ${err.message}`);
    console.error(
      "  (enrichment needs a configured provider/model + the goals_agent definition)",
    );
    return;
  }
  const ms = Date.now() - started;

  console.log(`  ran: ${result.ran}  (${ms}ms)`);
  console.log(`  agent summary: ${result.summary}`);
  console.log(`  goals after enrichment:\n${renderGoals(result.goals)}`);

  const after = await snapshot(opts.workspace);
  const changed = changedTranscripts(before, after);
  const goalsRuns = changed.filter((r) => r.agent.includes("goals"));

  console.log("\n  token usage / cost (changed sessions):");
  printUsageTable(changed.length ? changed : goalsRuns);

  if (opts.showThoughts) {
    const toShow = goalsRuns.length ? goalsRuns : changed;
    if (toShow.length === 0)
      console.log("\n  (no goals_agent transcript found — run may not persist transcripts)");
    for (const t of toShow) printThoughts(t);
  } else if (goalsRuns.length) {
    console.log("\n  (re-run with --show-thoughts to see the agent's reasoning + tool calls)");
  }
}

// ── main ────────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.workspace) opts.workspace = await defaultWorkspace();

  let tempWorkspace = "";
  let spawned;
  if (opts.isolatedWorkspace) {
    if (!opts.spawnCore) throw new Error("--isolated-workspace requires --spawn-core");
    tempWorkspace = await mkdtemp(path.join(tmpdir(), "openhuman-goals-live-"));
    opts.workspace = path.join(tempWorkspace, "workspace");
    await mkdir(opts.workspace, { recursive: true });
  }

  if (opts.spawnCore) {
    if (!opts.coreUrlExplicit) {
      const port = await pickFreePort();
      opts.coreUrl = `http://127.0.0.1:${port}/rpc`;
    }
    spawned = await startCore(opts);
    opts.token = spawned.token;
  } else {
    opts.token = await readToken(opts);
  }

  console.log("[goals-live] starting");
  console.log(`  rpc:        ${opts.coreUrl}`);
  console.log(`  workspace:  ${opts.workspace}`);
  console.log(`  goals file: ${path.join(opts.workspace, "MEMORY_GOALS.md")}`);
  console.log(`  mode:       ${opts.spawnCore ? "spawned-core" : "attached-core"}`);
  console.log(`  cases:      ${opts.cases.join(", ")}`);

  try {
    if (opts.reset) {
      console.log("\n=== reset ===");
      await resetGoals(opts);
    }
    for (const name of opts.cases) {
      if (name === "list") await caseList(opts, "list (initial)");
      else if (name === "add") await caseAdd(opts);
      else if (name === "edit") await caseEdit(opts);
      else if (name === "delete") await caseDelete(opts);
      else if (name === "reflect") await caseReflect(opts);
      else if (name === "list-final") await caseList(opts, "list (final)");
    }
  } finally {
    if (spawned?.child) await stopChild(spawned.child);
    if (tempWorkspace && !opts.keepWorkspace) {
      await rm(tempWorkspace, { recursive: true, force: true });
    } else if (tempWorkspace) {
      console.log(`\n[goals-live] kept temp workspace: ${opts.workspace}`);
    }
  }

  console.log("\n[goals-live] done");
}

main().catch((err) => {
  console.error(`[goals-live] ERROR: ${err.message}`);
  process.exit(1);
});
