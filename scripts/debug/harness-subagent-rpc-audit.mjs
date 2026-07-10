#!/usr/bin/env node
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_RPC_URL = "http://127.0.0.1:7788/rpc";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));

function usage() {
  return `Usage: node scripts/debug/harness-subagent-rpc-audit.mjs [options]

Runs a live JSON-RPC harness turn, waits for the async subagent to register,
then steers it through openhuman.subagent_steer while the parent/core process is live.
No prompt, response, credential, or transcript bodies are printed.

Options:
  --scenario <name>        async-steer, parallel-research-code, reuse-parent-comm, or all (default: async-steer)
  --core-url <url>          JSON-RPC endpoint (default: OPENHUMAN_CORE_RPC_URL or ${DEFAULT_RPC_URL})
  --token <token>           RPC bearer (default: OPENHUMAN_CORE_TOKEN or <workspace>/core.token)
  --workspace <path>        Workspace containing .openhuman/subagent_sessions.json
  --task-key <key>          Durable task key (default: audit-subagent-rpc-<timestamp>)
  --agent-id <id>           Subagent id to request (default: researcher)
  --model <model>           Optional model_override for openhuman.agent_chat
  --provider-mode <mode>    Isolated provider config: openhuman-backend or direct-openai (default: openhuman-backend)
  --rpc-timeout-ms <n>      Parent agent_chat timeout (default: 600000)
  --spawn-wait-ms <n>       Time to wait for a running durable session (default: 120000)
  --settle-wait-ms <n>      Time to wait for final session status after parent returns (default: 60000)
  --spawn-core              Start openhuman-core run --jsonrpc-only for the audit
  --isolated-workspace      With --spawn-core, use a temp workspace and custom audit agent definitions
  --keep-workspace          Do not remove an isolated temp workspace after the run; this can leave a temp config with a live API key
  --verbose                 Print response char counts and spawned core logs
  -h, --help                Show this help

Examples:
  node scripts/debug/harness-subagent-rpc-audit.mjs
  node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --model agentic-v1
  node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --scenario parallel-research-code --model agentic-v1
  node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --scenario reuse-parent-comm --model agentic-v1
  node scripts/debug/harness-subagent-rpc-audit.mjs --spawn-core --isolated-workspace --provider-mode direct-openai --scenario reuse-parent-comm --model gpt-4.1-mini
`;
}

function parseArgs(argv) {
  const opts = {
    scenario: "async-steer",
    coreUrl: process.env.OPENHUMAN_CORE_RPC_URL || DEFAULT_RPC_URL,
    token: process.env.OPENHUMAN_CORE_TOKEN || "",
    workspace: process.env.OPENHUMAN_WORKSPACE || "",
    taskKey: `audit-subagent-rpc-${Date.now().toString(36)}`,
    agentId: "researcher",
    model: "",
    providerMode: "openhuman-backend",
    rpcTimeoutMs: 600_000,
    spawnWaitMs: 120_000,
    settleWaitMs: 60_000,
    spawnCore: false,
    isolatedWorkspace: false,
    keepWorkspace: false,
    verbose: false,
    coreUrlExplicit: Boolean(process.env.OPENHUMAN_CORE_RPC_URL),
    agentIdExplicit: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const value = argv[++i];
      if (!value) throw new Error(`missing value for ${arg}`);
      return value;
    };
    switch (arg) {
      case "--scenario":
        opts.scenario = next();
        break;
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
      case "--task-key":
        opts.taskKey = next();
        break;
      case "--agent-id":
        opts.agentId = next();
        opts.agentIdExplicit = true;
        break;
      case "--model":
        opts.model = next();
        break;
      case "--provider-mode":
        opts.providerMode = next();
        break;
      case "--rpc-timeout-ms":
        opts.rpcTimeoutMs = parsePositiveInt(next(), "--rpc-timeout-ms");
        break;
      case "--spawn-wait-ms":
        opts.spawnWaitMs = parsePositiveInt(next(), "--spawn-wait-ms");
        break;
      case "--settle-wait-ms":
        opts.settleWaitMs = parsePositiveInt(next(), "--settle-wait-ms");
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
  const scenarios = new Set([
    "async-steer",
    "parallel-research-code",
    "reuse-parent-comm",
    "all",
  ]);
  if (!scenarios.has(opts.scenario)) {
    throw new Error(
      `--scenario must be one of ${Array.from(scenarios).join(", ")}`,
    );
  }
  const providerModes = new Set(["direct-openai", "openhuman-backend"]);
  if (!providerModes.has(opts.providerMode)) {
    throw new Error(
      `--provider-mode must be one of ${Array.from(providerModes).join(", ")}`,
    );
  }
  return opts;
}

function parsePositiveInt(raw, label) {
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 1) {
    throw new Error(`${label} must be a positive integer`);
  }
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
    // Fall through to legacy root workspace.
  }
  return openhumanDir;
}

async function readToken(opts) {
  if (opts.token.trim()) return opts.token.trim();
  const tokenPath = path.join(opts.workspace, "core.token");
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
        id: `subagent-audit-${Date.now()}-${Math.random().toString(16).slice(2)}`,
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
  if (!res.ok) throw new Error(`RPC ${method} HTTP ${res.status}`);
  if (body.error) {
    throw new Error(
      `RPC ${method} error: ${body.error.message || body.error.code || "unknown"}`,
    );
  }
  return body.result;
}

function sessionStorePath(workspace) {
  return path.join(workspace, ".openhuman", "subagent_sessions.json");
}

async function readSessions(workspace, taskKey) {
  let raw;
  try {
    raw = await readFile(sessionStorePath(workspace), "utf8");
  } catch {
    return [];
  }
  let sessions;
  try {
    sessions = JSON.parse(raw);
  } catch {
    return [];
  }
  return sessions
    .filter((session) => session?.taskKey === taskKey)
    .map((session) => ({
      subagentSessionId: String(session.subagentSessionId || ""),
      parentSession: String(session.parentSession || ""),
      workerThreadId: session.workerThreadId || null,
      agentId: String(session.agentId || ""),
      taskKey: String(session.taskKey || ""),
      currentTaskId: session.currentTaskId || null,
      status: String(session.status || ""),
      reusable: Boolean(session.reusable),
      updatedAt: String(session.updatedAt || ""),
      lastUsedAt: String(session.lastUsedAt || ""),
    }));
}

async function waitForRunningSession(
  workspace,
  taskKey,
  waitMs,
  parentPromise,
) {
  const deadline = Date.now() + waitMs;
  let last = [];
  while (Date.now() < deadline) {
    const parentState = await Promise.race([
      parentPromise.then(
        () => ({ done: true }),
        (err) => ({ error: err }),
      ),
      sleep(0).then(() => ({ pending: true })),
    ]);
    if (parentState.error) throw parentState.error;
    if (parentState.done) {
      throw new Error(
        `parent agent_chat completed before a running subagent session appeared; last_count=${last.length}`,
      );
    }

    last = await readSessions(workspace, taskKey);
    const running = last.find(
      (session) => session.currentTaskId && session.status === "running",
    );
    if (running) return running;
    await sleep(200);
  }
  throw new Error(
    `timed out waiting for running subagent session; last_count=${last.length}`,
  );
}

async function waitForSettledSessions(workspace, taskKey, waitMs) {
  const deadline = Date.now() + waitMs;
  let last = [];
  while (Date.now() < deadline) {
    last = await readSessions(workspace, taskKey);
    if (
      last.length > 0 &&
      last.some((session) => session.status !== "running")
    ) {
      return last;
    }
    await sleep(500);
  }
  return last;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function listJsonlFiles(dir) {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  const files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listJsonlFiles(full)));
    } else if (entry.isFile() && full.endsWith(".jsonl")) {
      files.push(full);
    }
  }
  return files;
}

async function transcriptSnapshot(workspace) {
  const transcriptDir = path.join(workspace, "session_raw");
  const files = await listJsonlFiles(transcriptDir);
  const snapshot = new Map();
  for (const file of files) {
    try {
      const metadata = await stat(file);
      snapshot.set(file, {
        size: metadata.size,
        mtimeMs: metadata.mtimeMs,
      });
    } catch {
      // Ignore racing transcript writes during live audit sampling.
    }
  }
  return snapshot;
}

function num(value) {
  return Number.isFinite(Number(value)) ? Number(value) : 0;
}

async function usageSnapshot(workspace) {
  const transcriptDir = path.join(workspace, "session_raw");
  const files = await listJsonlFiles(transcriptDir);
  const snapshot = new Map();
  await Promise.all(
    files.map(async (file) => {
      try {
        const data = await readFile(file, "utf8");
        const firstLine = data.split(/\r?\n/, 1)[0];
        const parsed = JSON.parse(firstLine);
        const meta = parsed._meta || {};
        snapshot.set(file, {
          file,
          agent: String(meta.agent || "(unknown)"),
          input: num(meta.input_tokens),
          output: num(meta.output_tokens),
          cached: num(meta.cached_input_tokens),
          charged: num(meta.charged_amount_usd),
          isSubagent: path.basename(file).includes("__"),
        });
      } catch {
        // Ignore malformed or partially-written transcripts during live audit sampling.
      }
    }),
  );
  return snapshot;
}

function diffUsageSnapshots(before, after) {
  const rows = [];
  for (const [file, current] of after.entries()) {
    const prior = before.get(file);
    const row = {
      file,
      agent: current.agent,
      isSubagent: current.isSubagent,
      input: Math.max(0, current.input - (prior?.input || 0)),
      output: Math.max(0, current.output - (prior?.output || 0)),
      cached: Math.max(0, current.cached - (prior?.cached || 0)),
      charged: Math.max(0, current.charged - (prior?.charged || 0)),
    };
    if (row.input || row.output || row.cached || row.charged || !prior) {
      rows.push(row);
    }
  }
  return rows;
}

function summarizeUsage(rows) {
  return rows.reduce(
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
}

function printUsageReport(label, rows) {
  const totals = summarizeUsage(rows);
  const cacheRate = totals.input > 0 ? (totals.cached / totals.input) * 100 : 0;
  console.log(
    `[harness-subagent-rpc-audit] usage ${label} total in=${totals.input} cache=${totals.cached} out=${totals.output} cost=$${totals.charged.toFixed(6)} cache_rate=${cacheRate.toFixed(2)}% sessions=${totals.sessions} subagents=${totals.subagentSessions}`,
  );
  for (const row of rows.sort((a, b) => {
    if (a.isSubagent !== b.isSubagent) return a.isSubagent ? 1 : -1;
    return a.agent.localeCompare(b.agent);
  })) {
    console.log(
      `  ${row.isSubagent ? "subagent" : "parent"} agent=${row.agent} in=${row.input} cache=${row.cached} out=${row.output} cost=$${row.charged.toFixed(6)}`,
    );
  }
  return totals;
}

function changedTranscriptFiles(before, after) {
  return [...after.entries()]
    .filter(([file, current]) => {
      const previous = before.get(file);
      return (
        !previous ||
        previous.size !== current.size ||
        previous.mtimeMs !== current.mtimeMs
      );
    })
    .map(([file]) => file);
}

function subagentTranscriptFiles(files) {
  return files.filter((file) => path.basename(file).includes("__"));
}

function spawnPrompt(opts) {
  return `Harness async subagent RPC audit.
Call spawn_subagent exactly once with agent_id \`${opts.agentId}\`, task_key \`${opts.taskKey}\`, blocking false, and fresh false.
Ask the sub-agent to produce a concise confirmation for audit marker \`${opts.taskKey}\`.
After the tool returns, reply with one short sentence saying the async worker was started.
Do not call wait_subagent.`;
}

function parallelPrompt(opts) {
  return `Harness parallel subagent audit.
Call spawn_parallel_agents exactly once with these two tasks:
1. agent_id "researcher", ownership "website research", prompt "Research https://example.com and return a concise factual note with the page title or domain purpose. Include one short evidence phrase. Do not browse unrelated sites."
2. agent_id "code_executor", ownership "code draft", prompt "Write a small Python function normalize_title(title: str) -> str that trims whitespace, collapses internal whitespace, and title-cases the result. Include one tiny assert-style example. Return only the code block; do not modify files."
After spawn_parallel_agents returns, reply with one concise sentence summarizing that both parallel workers completed.
Audit marker: ${opts.taskKey}.`;
}

function reusePrompt(opts, turn) {
  const base = `${opts.taskKey}-reuse`;
  const agentId = opts.agentId;
  if (turn === 1) {
    return `Harness reusable subagent parent communication audit, turn 1.
Call spawn_subagent exactly twice, both with blocking false and fresh false:
1. agent_id "${agentId}", task_key "${base}-alpha", prompt "You are alpha. Send a concise parent-facing status update with marker ${base}-alpha and remember that the topic is cache-aware reuse."
2. agent_id "${agentId}", task_key "${base}-beta", prompt "You are beta. Send a concise parent-facing status update with marker ${base}-beta and remember that the topic is durable worker reuse."
Then call wait_subagent for each returned task_id and collect both final updates.
After both workers are collected, reply with one concise sentence saying both worker updates were collected.`;
  }
  return `Harness reusable subagent parent communication audit, turn 2.
Call spawn_subagent exactly twice again with the same agent_id, same task_key values, blocking false, fresh false:
1. agent_id "${agentId}", task_key "${base}-alpha", prompt "Continue alpha's prior work. Mention the remembered cache-aware reuse topic and send a new concise parent-facing update."
2. agent_id "${agentId}", task_key "${base}-beta", prompt "Continue beta's prior work. Mention the remembered durable worker reuse topic and send a new concise parent-facing update."
Then call wait_subagent for each returned task_id and collect both final updates.
After both workers are collected, reply with one concise sentence saying both reusable worker updates were collected.`;
}

function steerMessage(opts) {
  return `Mid-run RPC steering audit for marker \`${opts.taskKey}\`: acknowledge that this instruction arrived through the async steering queue, then keep the final answer concise.`;
}

function responseText(result) {
  if (typeof result === "string") return result;
  if (typeof result?.result === "string") return result.result;
  if (typeof result?.response === "string") return result.response;
  if (typeof result?.data === "string") return result.data;
  return JSON.stringify(result);
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

async function writeAuditDefinitions(workspace) {
  const agentsDir = path.join(workspace, "agents");
  await mkdir(agentsDir, { recursive: true });
  await writeFile(
    path.join(agentsDir, "orchestrator.toml"),
    `id = "orchestrator"
display_name = "Subagent RPC Audit Orchestrator"
when_to_use = "Deterministic live harness async subagent RPC steering audit orchestrator."
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
You are the OpenHuman async subagent RPC audit orchestrator.
For async steering audit messages, call spawn_subagent exactly once with agent_id "async_audit_worker", blocking false, fresh false, and the task_key provided by the user.
For parallel audit messages, call spawn_parallel_agents exactly once with the task list provided by the user.
For reusable subagent parent communication audit messages, call spawn_subagent exactly as many times as requested, preserving each requested task_key, blocking setting, and fresh setting. When asked to collect workers, call wait_subagent for the returned task_id values.
After the requested tool call or calls return, provide one concise sentence. Do not call wait_subagent for async steering audits. Do not call any tools other than the requested audit tools.
"""

[tools]
named = ["spawn_subagent", "spawn_parallel_agents", "wait_subagent"]

[subagents]
allowlist = ["async_audit_worker", "researcher", "code_executor"]
`,
  );
  await writeFile(
    path.join(agentsDir, "async_audit_worker.toml"),
    `id = "async_audit_worker"
display_name = "Async Audit Worker"
delegate_name = "delegate_async_audit_worker"
when_to_use = "Tiny worker used only by harness async subagent RPC steering audit runs."
temperature = 0.0
max_iterations = 2
sandbox_mode = "none"
agent_tier = "worker"
omit_identity = true
omit_memory_context = true
omit_safety_preamble = true
omit_skills_catalog = true
omit_profile = true
omit_memory_md = true

[system_prompt]
inline = "Return one short sentence confirming the async audit worker ran and mention whether a steering instruction was received. Do not call tools."

[tools]
named = []
`,
  );
}

async function writeIsolatedDirectProviderConfig(workspace, model) {
  const apiKey =
    process.env.OPENAI_API_KEY?.trim() || process.env.OPENAI_KEY?.trim() || "";
  if (!apiKey) {
    throw new Error(
      "--isolated-workspace requires OPENAI_API_KEY or OPENAI_KEY for direct OpenAI provider routing",
    );
  }
  const providerModel = model?.trim() || "gpt-4.1-mini";
  const providerRoute = `openai:${providerModel}`;
  await writeFile(
    path.join(workspace, "config.toml"),
    `api_key = ${JSON.stringify(apiKey)}
inference_url = "https://api.openai.com/v1"
default_model = ${JSON.stringify(providerModel)}
chat_provider = ${JSON.stringify(providerRoute)}
reasoning_provider = ${JSON.stringify(providerRoute)}
agentic_provider = ${JSON.stringify(providerRoute)}
coding_provider = ${JSON.stringify(providerRoute)}
memory_provider = "openhuman"
embedding_provider = "none"

[[cloud_providers]]
id = "audit_openai"
slug = "openai"
label = "OpenAI"
endpoint = "https://api.openai.com/v1"
auth_style = "bearer"
default_model = ${JSON.stringify(providerModel)}
`,
    { mode: 0o600 },
  );
}

function backendApiUrl() {
  const explicit =
    process.env.BACKEND_URL?.trim() ||
    process.env.VITE_BACKEND_URL?.trim() ||
    "";
  if (explicit) return explicit.replace(/\/+$/, "");
  return process.env.OPENHUMAN_APP_ENV === "staging"
    ? "https://staging-api.tinyhumans.ai"
    : "https://api.tinyhumans.ai";
}

async function writeIsolatedOpenHumanBackendConfig(workspace, model) {
  const providerModel = model?.trim() || "agentic-v1";
  await writeFile(
    path.join(workspace, "config.toml"),
    `api_url = ${JSON.stringify(backendApiUrl())}
default_model = ${JSON.stringify(providerModel)}
chat_provider = "openhuman"
reasoning_provider = "openhuman"
agentic_provider = "openhuman"
coding_provider = "openhuman"
memory_provider = "openhuman"
embedding_provider = "none"

[secrets]
encrypt = false
`,
    { mode: 0o600 },
  );
}

async function writeIsolatedAppSessionAuth(workspace) {
  const token = process.env.JWT_TOKEN?.trim() || "";
  if (!token) {
    throw new Error(
      "--provider-mode openhuman-backend with --isolated-workspace requires JWT_TOKEN from scripts/load-dotenv.sh or your shell",
    );
  }
  const now = new Date().toISOString();
  await writeFile(
    path.join(workspace, "auth-profiles.json"),
    JSON.stringify(
      {
        schema_version: 1,
        updated_at: now,
        active_profiles: {
          "app-session": "app-session:default",
        },
        profiles: {
          "app-session:default": {
            provider: "app-session",
            profile_name: "default",
            kind: "token",
            token,
            created_at: now,
            updated_at: now,
            metadata: {},
          },
        },
      },
      null,
      2,
    ),
    { mode: 0o600 },
  );
}

async function startCore(opts) {
  const token = opts.token || `audit-${randomBytes(24).toString("hex")}`;
  const env = { ...process.env, OPENHUMAN_CORE_TOKEN: token };
  if (opts.workspace) env.OPENHUMAN_WORKSPACE = opts.workspace;
  if (opts.isolatedWorkspace) env.OPENHUMAN_AGENTBOX_MODE = "1";
  const port = new URL(opts.coreUrl).port || "7788";
  env.OPENHUMAN_CORE_PORT = port;
  env.OPENHUMAN_CORE_RPC_URL = opts.coreUrl;
  const child = spawn(
    "cargo",
    [
      "run",
      "--quiet",
      "--bin",
      "openhuman-core",
      "--",
      "run",
      "--host",
      "127.0.0.1",
      "--port",
      port,
      "--jsonrpc-only",
    ],
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
      await sleep(750);
    }
  }
  throw new Error(`timed out waiting for spawned core\n${stderrFn()}`);
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  const exited = await Promise.race([
    once(child, "exit").then(() => true),
    sleep(5_000).then(() => false),
  ]);
  if (exited || child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGKILL");
  await Promise.race([once(child, "exit"), sleep(2_000)]);
}

function unwrapData(result) {
  return result?.data && typeof result.data === "object" ? result.data : result;
}

async function runAsyncSteerScenario(opts) {
  const params = { message: spawnPrompt(opts) };
  if (opts.model) params.model_override = opts.model;

  const parentStarted = Date.now();
  const parentPromise = rpc(
    opts.coreUrl,
    opts.token,
    "openhuman.agent_chat",
    params,
    opts.rpcTimeoutMs,
  );

  const runningSession = await waitForRunningSession(
    opts.sessionWorkspace || opts.workspace,
    opts.taskKey,
    opts.spawnWaitMs,
    parentPromise,
  );
  console.log(
    `[harness-subagent-rpc-audit] running session task_id=${runningSession.currentTaskId} subagent_session_id=${runningSession.subagentSessionId}`,
  );

  const steerResult = unwrapData(
    await rpc(
      opts.coreUrl,
      opts.token,
      "openhuman.subagent_steer",
      {
        taskId: runningSession.currentTaskId,
        message: steerMessage(opts),
        mode: "steer",
      },
      30_000,
    ),
  );
  console.log(
    `[harness-subagent-rpc-audit] steer result steered=${Boolean(steerResult.steered)} reason=${steerResult.reason || "none"}`,
  );

  const parentResult = await parentPromise;
  const response = responseText(parentResult);
  console.log(
    `[harness-subagent-rpc-audit] parent turn completed in ${Date.now() - parentStarted}ms${
      opts.verbose ? ` response_chars=${response.length}` : ""
    }`,
  );

  const sessions = await waitForSettledSessions(
    opts.sessionWorkspace || opts.workspace,
    opts.taskKey,
    opts.settleWaitMs,
  );

  console.log("[harness-subagent-rpc-audit] sessions");
  if (sessions.length === 0) {
    console.log("  none");
  } else {
    for (const session of sessions) {
      console.log(
        `  subagent_session_id=${session.subagentSessionId} task_id=${session.currentTaskId || "none"} status=${session.status} reusable=${session.reusable} updated_at=${session.updatedAt}`,
      );
    }
  }

  const failures = [];
  if (!runningSession?.currentTaskId)
    failures.push("no running subagent task observed");
  if (!steerResult?.steered) {
    failures.push(
      `subagent steer was not accepted (${steerResult?.reason || "unknown"})`,
    );
  }
  const uniqueSessions = new Set(
    sessions.map((session) => session.subagentSessionId),
  );
  if (uniqueSessions.size !== 1) {
    failures.push(
      `expected one durable session for task key, observed ${uniqueSessions.size}`,
    );
  }
  if (sessions.length === 0)
    failures.push("no durable session remained after audit");
  return failures;
}

async function runParallelResearchCodeScenario(opts) {
  const transcriptWorkspace = opts.sessionWorkspace || opts.workspace;
  const before = await transcriptSnapshot(transcriptWorkspace);
  const params = { message: parallelPrompt(opts) };
  if (opts.model) params.model_override = opts.model;

  const started = Date.now();
  const result = await rpc(
    opts.coreUrl,
    opts.token,
    "openhuman.agent_chat",
    params,
    opts.rpcTimeoutMs,
  );
  const response = responseText(result);
  const after = await transcriptSnapshot(transcriptWorkspace);
  const changed = changedTranscriptFiles(before, after);
  const changedSubagents = subagentTranscriptFiles(changed);

  console.log(
    `[harness-subagent-rpc-audit] parallel turn completed in ${Date.now() - started}ms${
      opts.verbose ? ` response_chars=${response.length}` : ""
    }`,
  );
  console.log(
    `[harness-subagent-rpc-audit] transcript changes changed=${changed.length} subagent_changed=${changedSubagents.length}`,
  );

  const failures = [];
  if (response.length === 0)
    failures.push("parallel parent response was empty");
  if (changedSubagents.length < 2) {
    failures.push(
      `expected at least two changed subagent transcripts, observed ${changedSubagents.length}`,
    );
  }
  return failures;
}

async function runReuseParentCommScenario(opts) {
  const transcriptWorkspace = opts.sessionWorkspace || opts.workspace;
  const sessionWorkspace = opts.sessionWorkspace || opts.workspace;
  const failures = [];
  const turnSummaries = [];
  const sessionSets = [];

  for (let turn = 1; turn <= 2; turn += 1) {
    const before = await usageSnapshot(transcriptWorkspace);
    const params = { message: reusePrompt(opts, turn) };
    if (opts.model) params.model_override = opts.model;

    const started = Date.now();
    const result = await rpc(
      opts.coreUrl,
      opts.token,
      "openhuman.agent_chat",
      params,
      opts.rpcTimeoutMs,
    );
    const response = responseText(result);
    const after = await usageSnapshot(transcriptWorkspace);
    const rows = diffUsageSnapshots(before, after);
    const totals = printUsageReport(`reuse-parent-comm turn=${turn}`, rows);
    const sessions = [
      ...(await readSessions(sessionWorkspace, `${opts.taskKey}-reuse-alpha`)),
      ...(await readSessions(sessionWorkspace, `${opts.taskKey}-reuse-beta`)),
    ];
    const durableIds = sessions
      .map((session) => session.subagentSessionId)
      .filter(Boolean)
      .sort();
    sessionSets.push(durableIds);
    turnSummaries.push({
      ms: Date.now() - started,
      responseChars: response.length,
      totals,
      durableIds,
    });
    console.log(
      `[harness-subagent-rpc-audit] reuse turn ${turn} completed in ${turnSummaries.at(-1).ms}ms sessions=${durableIds.join(",") || "none"}${
        opts.verbose ? ` response_chars=${response.length}` : ""
      }`,
    );
    if (response.length === 0) {
      failures.push(`reuse turn ${turn} parent response was empty`);
    }
    if (totals.input === 0) {
      failures.push(`reuse turn ${turn} had no transcript usage metadata`);
    }
    if (totals.subagentSessions < 2) {
      failures.push(
        `reuse turn ${turn} expected at least two subagent usage deltas, observed ${totals.subagentSessions}`,
      );
    }
    if (durableIds.length !== 2) {
      failures.push(
        `reuse turn ${turn} expected two durable subagent sessions, observed ${durableIds.length}`,
      );
    }
  }

  if (
    sessionSets.length === 2 &&
    JSON.stringify(sessionSets[0]) !== JSON.stringify(sessionSets[1])
  ) {
    failures.push(
      `durable subagent sessions were not reused across turns: first=${sessionSets[0].join(",") || "none"} second=${sessionSets[1].join(",") || "none"}`,
    );
  }
  return failures;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.workspace) opts.workspace = await defaultWorkspace();

  let tempWorkspace = "";
  let spawned;
  if (opts.isolatedWorkspace) {
    if (!opts.spawnCore) {
      throw new Error("--isolated-workspace requires --spawn-core");
    }
    tempWorkspace = await mkdtemp(
      path.join(tmpdir(), "openhuman-harness-subagent-rpc-audit-"),
    );
    opts.workspace = path.join(tempWorkspace, "workspace");
    await mkdir(opts.workspace, { recursive: true });
    await writeAuditDefinitions(opts.workspace);
    await writeAuditDefinitions(path.join(opts.workspace, "workspace"));
    if (opts.providerMode === "openhuman-backend") {
      await writeIsolatedOpenHumanBackendConfig(opts.workspace, opts.model);
      await writeIsolatedAppSessionAuth(opts.workspace);
    } else {
      await writeIsolatedDirectProviderConfig(opts.workspace, opts.model);
    }
    opts.sessionWorkspace = path.join(opts.workspace, "workspace");
    if (!opts.agentIdExplicit) opts.agentId = "async_audit_worker";
    if (!opts.model) {
      opts.model =
        opts.providerMode === "openhuman-backend"
          ? "agentic-v1"
          : "gpt-4.1-mini";
    }
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

  console.log("[harness-subagent-rpc-audit] starting live audit");
  console.log(`  rpc: ${opts.coreUrl}`);
  console.log(`  workspace: ${opts.workspace}`);
  if (opts.sessionWorkspace) {
    console.log(`  session_workspace: ${opts.sessionWorkspace}`);
  }
  console.log(`  task_key: ${opts.taskKey}`);
  console.log(`  scenario: ${opts.scenario}`);
  if (opts.scenario !== "parallel-research-code") {
    console.log(`  agent_id: ${opts.agentId}`);
  }
  console.log(`  mode: ${opts.spawnCore ? "spawned-core" : "attached-core"}`);
  if (opts.isolatedWorkspace) {
    console.log("  definitions: isolated audit overrides enabled");
    console.log(`  provider_mode: ${opts.providerMode}`);
  }

  const failures = [];
  try {
    const scenarios =
      opts.scenario === "all"
        ? ["async-steer", "parallel-research-code", "reuse-parent-comm"]
        : [opts.scenario];
    for (const scenario of scenarios) {
      console.log(`[harness-subagent-rpc-audit] scenario ${scenario}`);
      const scenarioOpts = {
        ...opts,
        taskKey:
          scenarios.length > 1 ? `${opts.taskKey}-${scenario}` : opts.taskKey,
      };
      if (scenario === "async-steer") {
        failures.push(...(await runAsyncSteerScenario(scenarioOpts)));
      } else if (scenario === "parallel-research-code") {
        failures.push(...(await runParallelResearchCodeScenario(scenarioOpts)));
      } else if (scenario === "reuse-parent-comm") {
        failures.push(...(await runReuseParentCommScenario(scenarioOpts)));
      }
    }
  } finally {
    if (spawned?.child) await stopChild(spawned.child);
    if (tempWorkspace && !opts.keepWorkspace) {
      await rm(tempWorkspace, { recursive: true, force: true });
    }
  }

  if (failures.length > 0) {
    console.error("\n[harness-subagent-rpc-audit] FAIL");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  if (tempWorkspace && opts.keepWorkspace) {
    console.log(
      `[harness-subagent-rpc-audit] kept isolated workspace: ${opts.workspace}`,
    );
  }
  console.log("\n[harness-subagent-rpc-audit] PASS");
}

main().catch((err) => {
  console.error(`[harness-subagent-rpc-audit] ERROR: ${err.message}`);
  process.exit(1);
});
