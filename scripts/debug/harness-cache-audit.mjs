#!/usr/bin/env node
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_RPC_URL = "http://127.0.0.1:7788/rpc";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));

function usage() {
  return `Usage: node scripts/debug/harness-cache-audit.mjs [options]

Runs live harness turns through JSON-RPC, then audits transcript usage metadata.
No prompt, response, or credential bodies are printed.

Options:
  --core-url <url>        JSON-RPC endpoint (default: OPENHUMAN_CORE_RPC_URL or ${DEFAULT_RPC_URL})
  --token <token>         RPC bearer (default: OPENHUMAN_CORE_TOKEN or <workspace>/core.token)
  --workspace <path>      Workspace whose session_raw transcripts should be audited
  --turns <n>             Number of agent turns to run (default: 3)
  --model <model>         Optional model_override passed to openhuman.agent_chat
  --thread-id <id>        Stable backend thread_id to group audit inference/cache logs
  --rpc-timeout-ms <n>    Per-RPC timeout in milliseconds (default: 600000)
  --prompt <text>         Prompt to send each turn (default: cache-audit delegation prompt)
  --spawn-core            Start openhuman-core serve --jsonrpc-only for the audit
  --isolated-workspace    With --spawn-core, use a temp workspace and custom audit agent definitions
  --keep-workspace        Do not remove an isolated temp workspace after the run
  --min-hit-rate <pct>    Fail if aggregate cached/input ratio is below this percent (default: 1)
  --max-turns-without-cache <n>
                           Fail if more than n completed turns have zero cached input (default: 1)
  --verbose               Stream spawned core logs and print per-turn response chars
  -h, --help              Show this help

Examples:
  pnpm debug harness-cache-audit --turns 4 --min-hit-rate 20
  node scripts/debug/harness-cache-audit.mjs --spawn-core --isolated-workspace --model gpt-4.1-mini
`;
}

function parseArgs(argv) {
  const opts = {
    coreUrl: process.env.OPENHUMAN_CORE_RPC_URL || DEFAULT_RPC_URL,
    token: process.env.OPENHUMAN_CORE_TOKEN || "",
    workspace: process.env.OPENHUMAN_WORKSPACE || "",
    turns: 3,
    model: "",
    threadId: `harness-cache-audit-${Date.now().toString(36)}`,
    rpcTimeoutMs: 600_000,
    prompt: "",
    spawnCore: false,
    isolatedWorkspace: false,
    keepWorkspace: false,
    minHitRate: 1,
    maxTurnsWithoutCache: 1,
    verbose: false,
    coreUrlExplicit: Boolean(process.env.OPENHUMAN_CORE_RPC_URL),
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const value = argv[++i];
      if (!value) throw new Error(`missing value for ${arg}`);
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
      case "--turns":
        opts.turns = parsePositiveInt(next(), "--turns");
        break;
      case "--model":
        opts.model = next();
        break;
      case "--thread-id":
        opts.threadId = next();
        break;
      case "--rpc-timeout-ms":
        opts.rpcTimeoutMs = parsePositiveInt(next(), "--rpc-timeout-ms");
        break;
      case "--prompt":
        opts.prompt = next();
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
      case "--min-hit-rate":
        opts.minHitRate = parseNonNegativeNumber(next(), "--min-hit-rate");
        break;
      case "--max-turns-without-cache":
        opts.maxTurnsWithoutCache = parseNonNegativeInt(
          next(),
          "--max-turns-without-cache",
        );
        break;
      case "--verbose":
        opts.verbose = true;
        break;
      case "-h":
      case "--help":
        console.log(usage());
        process.exit(0);
      default:
        throw new Error(`unknown option: ${arg}`);
    }
  }
  return opts;
}

function parsePositiveInt(raw, label) {
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 1)
    throw new Error(`${label} must be a positive integer`);
  return value;
}

function parseNonNegativeInt(raw, label) {
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 0)
    throw new Error(`${label} must be a non-negative integer`);
  return value;
}

function parseNonNegativeNumber(raw, label) {
  const value = Number(raw);
  if (!Number.isFinite(value) || value < 0)
    throw new Error(`${label} must be a non-negative number`);
  return value;
}

function defaultOpenhumanDir() {
  return process.env.OPENHUMAN_APP_ENV === "staging"
    ? path.join(homedir(), ".openhuman-staging")
    : path.join(homedir(), ".openhuman");
}

async function defaultWorkspace() {
  if (process.env.OPENHUMAN_WORKSPACE) return process.env.OPENHUMAN_WORKSPACE;
  const openhumanDir = defaultOpenhumanDir();
  try {
    const active = await readFile(
      path.join(openhumanDir, "active_user.toml"),
      "utf8",
    );
    const match = active.match(/^\s*user_id\s*=\s*"([^"]+)"\s*$/m);
    if (match?.[1]) {
      return path.join(openhumanDir, "users", match[1], "workspace");
    }
  } catch {
    // Fall back to the legacy root workspace below.
  }
  return openhumanDir;
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
      `RPC token not provided and ${tokenPath} could not be read. Pass --token or set OPENHUMAN_CORE_TOKEN.`,
    );
  }
}

async function rpc(coreUrl, token, method, params, timeoutMs = 600_000) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
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
        id: `audit-${Date.now()}-${Math.random().toString(16).slice(2)}`,
        method,
        params,
      }),
    });
  } catch (err) {
    if (err?.name === "AbortError") {
      throw new Error(`RPC ${method} timed out after ${timeoutMs}ms`);
    }
    throw err;
  } finally {
    clearTimeout(timeout);
  }
  const bodyText = await res.text();
  let body;
  try {
    body = JSON.parse(bodyText);
  } catch {
    throw new Error(`RPC ${method} returned non-JSON HTTP ${res.status}`);
  }
  if (!res.ok) {
    throw new Error(`RPC ${method} HTTP ${res.status}`);
  }
  if (body.error) {
    throw new Error(`RPC ${method} error`);
  }
  return body.result;
}

async function walkJsonl(dir) {
  const out = [];
  async function walk(current) {
    let entries;
    try {
      entries = await readdir(current, { withFileTypes: true });
    } catch {
      return;
    }
    await Promise.all(
      entries.map(async (entry) => {
        const full = path.join(current, entry.name);
        if (entry.isDirectory()) return walk(full);
        if (entry.isFile() && entry.name.endsWith(".jsonl")) out.push(full);
      }),
    );
  }
  await walk(dir);
  return out;
}

function num(value) {
  return Number.isFinite(Number(value)) ? Number(value) : 0;
}

async function readTranscriptMeta(file) {
  const data = await readFile(file, "utf8");
  const firstLine = data.split(/\r?\n/, 1)[0];
  const parsed = JSON.parse(firstLine);
  const meta = parsed._meta || {};
  return {
    file,
    agent: String(meta.agent || "(unknown)"),
    input: num(meta.input_tokens),
    output: num(meta.output_tokens),
    cached: num(meta.cached_input_tokens),
    charged: num(meta.charged_amount_usd),
    isSubagent: path.basename(file).includes("__"),
  };
}

async function snapshotTranscripts(workspace) {
  const files = await walkJsonl(path.join(workspace, "session_raw"));
  const entries = new Map();
  await Promise.all(
    files.map(async (file) => {
      try {
        entries.set(file, await readTranscriptMeta(file));
      } catch {
        // Ignore malformed or partially-written transcripts; the next snapshot can pick them up.
      }
    }),
  );
  return entries;
}

function diffSnapshots(before, after) {
  const rows = [];
  for (const [file, current] of after.entries()) {
    const prior = before.get(file);
    const delta = {
      file,
      agent: current.agent,
      isSubagent: current.isSubagent,
      input: Math.max(0, current.input - (prior?.input || 0)),
      output: Math.max(0, current.output - (prior?.output || 0)),
      cached: Math.max(0, current.cached - (prior?.cached || 0)),
      charged: Math.max(0, current.charged - (prior?.charged || 0)),
    };
    if (delta.input || delta.output || delta.cached || delta.charged || !prior)
      rows.push(delta);
  }
  return rows;
}

function summarize(rows) {
  const totals = rows.reduce(
    (acc, row) => {
      acc.input += row.input;
      acc.output += row.output;
      acc.cached += row.cached;
      acc.charged += row.charged;
      acc.sessions += 1;
      if (row.isSubagent) acc.subagentSessions += 1;
      return acc;
    },
    {
      input: 0,
      output: 0,
      cached: 0,
      charged: 0,
      sessions: 0,
      subagentSessions: 0,
    },
  );
  totals.hitRate = totals.input > 0 ? (totals.cached / totals.input) * 100 : 0;
  return totals;
}

function printReport(opts, rows, turnResults) {
  const totals = summarize(rows);
  const byAgent = new Map();
  for (const row of rows) {
    const item = byAgent.get(row.agent) || {
      agent: row.agent,
      input: 0,
      output: 0,
      cached: 0,
      charged: 0,
      sessions: 0,
      subagentSessions: 0,
    };
    item.input += row.input;
    item.output += row.output;
    item.cached += row.cached;
    item.charged += row.charged;
    item.sessions += 1;
    if (row.isSubagent) item.subagentSessions += 1;
    byAgent.set(row.agent, item);
  }

  console.log("\n[harness-cache-audit] summary");
  console.log(`  turns completed: ${turnResults.length}/${opts.turns}`);
  console.log(`  transcript sessions changed: ${totals.sessions}`);
  console.log(`  subagent transcript sessions: ${totals.subagentSessions}`);
  console.log(`  input tokens: ${totals.input}`);
  console.log(`  output tokens: ${totals.output}`);
  console.log(`  cached input tokens: ${totals.cached}`);
  console.log(`  cache hit rate: ${totals.hitRate.toFixed(2)}%`);
  console.log(`  charged amount: $${totals.charged.toFixed(6)}`);

  if (byAgent.size > 0) {
    console.log("\n[harness-cache-audit] by agent");
    const rowsForTable = [...byAgent.values()].map((row) => ({
      agent: row.agent,
      sessions: row.sessions,
      subagents: row.subagentSessions,
      input: row.input,
      output: row.output,
      cached: row.cached,
      hit_rate:
        row.input > 0
          ? `${((row.cached / row.input) * 100).toFixed(2)}%`
          : "0.00%",
      charged: `$${row.charged.toFixed(6)}`,
    }));
    console.table(rowsForTable);
  }

  if (opts.verbose) {
    console.log("\n[harness-cache-audit] changed transcript files");
    for (const row of rows) {
      console.log(`  ${row.agent}: ${row.file}`);
    }
  }

  return totals;
}

function auditPrompt(turn) {
  return `Harness cache audit turn ${turn}.

You are exercising OpenHuman's live harness and backend cache behavior.
You must delegate exactly one small task to an appropriate subagent if a delegation tool is available.
Ask the subagent to return a one-sentence cache-audit note about stable prompts and repeated turns.
Then reply with only a concise one-sentence summary.`;
}

function responseText(result) {
  if (typeof result === "string") return result;
  if (typeof result?.result === "string") return result.result;
  if (typeof result?.response === "string") return result.response;
  return JSON.stringify(result);
}

async function writeAuditDefinitions(workspace) {
  const agentsDir = path.join(workspace, "agents");
  await mkdir(agentsDir, { recursive: true });
  await writeFile(
    path.join(agentsDir, "orchestrator.toml"),
    `id = "orchestrator"
display_name = "Cache Audit Orchestrator"
when_to_use = "Deterministic live harness cache audit orchestrator."
temperature = 0.0
max_iterations = 4
sandbox_mode = "none"
agent_tier = "chat"
omit_identity = true
omit_memory_context = true
omit_safety_preamble = true
omit_skills_catalog = true
omit_profile = true
omit_memory_md = true

[system_prompt]
inline = """
You are the OpenHuman harness cache audit orchestrator.
For every user message, call delegate_audit_worker exactly once with a concise prompt.
After the worker returns, provide one sentence. Do not call any other tools.
"""

[tools]
named = ["spawn_subagent"]

[subagents]
allowlist = ["audit_worker"]
`,
  );
  await writeFile(
    path.join(agentsDir, "audit_worker.toml"),
    `id = "audit_worker"
display_name = "Cache Audit Worker"
delegate_name = "delegate_audit_worker"
when_to_use = "Tiny worker used only by harness cache audit runs."
temperature = 0.0
max_iterations = 1
sandbox_mode = "none"
agent_tier = "worker"
omit_identity = true
omit_memory_context = true
omit_safety_preamble = true
omit_skills_catalog = true
omit_profile = true
omit_memory_md = true

[system_prompt]
inline = "Return one short sentence confirming the cache-audit worker ran. Do not call tools."

[tools]
named = []
`,
  );
}

async function pickFreePort() {
  return await new Promise((resolve, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close(() => resolve(port));
    });
  });
}

async function startCore(opts) {
  const token = opts.token || `audit-${randomBytes(24).toString("hex")}`;
  const env = { ...process.env, OPENHUMAN_CORE_TOKEN: token };
  if (opts.workspace) {
    env.OPENHUMAN_WORKSPACE = opts.workspace;
  }
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
      stdio: opts.verbose
        ? ["ignore", "inherit", "inherit"]
        : ["ignore", "ignore", "pipe"],
    },
  );
  let stderr = "";
  if (child.stderr) {
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
      if (stderr.length > 8000) stderr = stderr.slice(-8000);
    });
  }
  await waitForCore(opts.coreUrl, token, child, () => stderr);
  return { child, token };
}

async function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;

  child.kill("SIGTERM");
  const exited = await Promise.race([
    once(child, "exit").then(() => true),
    new Promise((resolve) => setTimeout(() => resolve(false), 5_000)),
  ]);
  if (exited || child.exitCode !== null || child.signalCode !== null) return;

  child.kill("SIGKILL");
  await Promise.race([
    once(child, "exit"),
    new Promise((resolve) => setTimeout(resolve, 2_000)),
  ]);
}

async function waitForCore(coreUrl, token, child, stderrFn) {
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `spawned core exited with ${child.exitCode}\n${stderrFn()}`,
      );
    }
    try {
      await rpc(coreUrl, token, "core.ping", {}, 10_000);
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 750));
    }
  }
  throw new Error(
    `timed out waiting for spawned core at ${coreUrl}\n${stderrFn()}`,
  );
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.workspace) opts.workspace = await defaultWorkspace();

  let tempWorkspace = "";
  let spawned;
  if (opts.isolatedWorkspace) {
    if (!opts.spawnCore)
      throw new Error("--isolated-workspace requires --spawn-core");
    tempWorkspace = await mkdtemp(
      path.join(tmpdir(), "openhuman-harness-cache-audit-"),
    );
    opts.workspace = path.join(tempWorkspace, "workspace");
    await mkdir(opts.workspace, { recursive: true });
    await writeAuditDefinitions(opts.workspace);
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

  console.log("[harness-cache-audit] starting live audit");
  console.log(`  rpc: ${opts.coreUrl}`);
  console.log(`  workspace: ${opts.workspace}`);
  console.log(`  turns: ${opts.turns}`);
  console.log(`  mode: ${opts.spawnCore ? "spawned-core" : "attached-core"}`);
  if (opts.isolatedWorkspace) {
    console.log("  definitions: isolated audit overrides enabled");
  }

  const before = await snapshotTranscripts(opts.workspace);
  const turnResults = [];
  let zeroCacheTurns = 0;
  try {
    for (let i = 1; i <= opts.turns; i += 1) {
      const turnBefore = await snapshotTranscripts(opts.workspace);
      const params = {
        message: opts.prompt || auditPrompt(i),
      };
      if (opts.model) params.model_override = opts.model;
      if (opts.threadId) params.thread_id = opts.threadId;
      const started = Date.now();
      const result = await rpc(
        opts.coreUrl,
        opts.token,
        "openhuman.agent_chat",
        params,
        opts.rpcTimeoutMs,
      );
      const response = responseText(result);
      const turnAfter = await snapshotTranscripts(opts.workspace);
      const rootDelta = summarize(
        diffSnapshots(turnBefore, turnAfter).filter((row) => !row.isSubagent),
      );
      if (rootDelta.input > 0 && rootDelta.cached === 0) zeroCacheTurns += 1;
      turnResults.push({ ms: Date.now() - started, chars: response.length });
      console.log(
        `[harness-cache-audit] turn ${i}/${opts.turns} ok (${turnResults.at(-1).ms}ms${
          opts.verbose ? `, response_chars=${response.length}` : ""
        })`,
      );
    }
  } finally {
    if (spawned?.child) {
      await stopChild(spawned.child);
    }
  }

  const after = await snapshotTranscripts(opts.workspace);
  const rows = diffSnapshots(before, after);
  const totals = printReport(opts, rows, turnResults);

  const failures = [];
  if (totals.input === 0) {
    failures.push(
      "no transcript usage metadata changed; the provider may not have emitted usage",
    );
  }
  if (totals.hitRate < opts.minHitRate) {
    failures.push(
      `cache hit rate ${totals.hitRate.toFixed(2)}% is below ${opts.minHitRate}%`,
    );
  }
  if (zeroCacheTurns > opts.maxTurnsWithoutCache) {
    failures.push(
      `${zeroCacheTurns} root turn(s) had zero cached input, above limit ${opts.maxTurnsWithoutCache}`,
    );
  }
  if (totals.subagentSessions === 0) {
    failures.push("no subagent transcript deltas were observed");
  }

  if (tempWorkspace && !opts.keepWorkspace) {
    await rm(tempWorkspace, { recursive: true, force: true });
  } else if (tempWorkspace) {
    console.log(
      `[harness-cache-audit] kept isolated workspace: ${opts.workspace}`,
    );
  }

  if (failures.length > 0) {
    console.error("\n[harness-cache-audit] FAIL");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log("\n[harness-cache-audit] PASS");
}

main().catch((err) => {
  console.error(`[harness-cache-audit] ERROR: ${err.message}`);
  process.exit(1);
});
