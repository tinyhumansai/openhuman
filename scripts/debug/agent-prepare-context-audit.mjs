#!/usr/bin/env node
// Live audit for the `agent_prepare_context` tool + `context_scout` subagent.
//
// Drives real agent turns through JSON-RPC against an authenticated core, forces
// the orchestrator to call `agent_prepare_context`, then reads the resulting
// session transcripts to surface — per query — the returned [context_bundle],
// the scout's step-by-step turns ("thoughts"), and tokens in/out/cached + cost.
//
// Unlike harness-cache-audit.mjs (which deliberately hides bodies), this script
// PRINTS bundle + thought content so you can iterate on the context_scout
// prompt. Run it against a core built from this branch (so the tool exists) and
// signed into your account (so the LLM calls bill to you).
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import { statSync } from "node:fs";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_RPC_URL = "http://127.0.0.1:7788/rpc";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const SCOUT_AGENT = "context_scout";

// Default audit queries, each aimed at a different context source.
const DEFAULT_CASES = [
  { name: "memory/projects", query: "What do you know about my current projects and what should I work on next?" },
  { name: "goals/profile", query: "Based on my stated goals, what should I prioritise this week?" },
  { name: "web/fresh-fact", query: "What is the latest stable Rust release and one notable change in it?" },
  { name: "integrations/email", query: "Summarise my most important unread emails from the last day." },
  { name: "mixed/plan", query: "Plan my day: combine my goals, recent activity, and anything time-sensitive." },
];

function usage() {
  return `Usage: node scripts/debug/agent-prepare-context-audit.mjs [options]

Audits the agent_prepare_context tool live: forces it per query, then prints the
returned context bundle, the scout's turns, and tokens/cache/cost.

Options:
  --core-url <url>        JSON-RPC endpoint (default: OPENHUMAN_CORE_RPC_URL or ${DEFAULT_RPC_URL})
  --token <token>         RPC bearer (default: OPENHUMAN_CORE_TOKEN or <workspace>/core.token)
  --workspace <path>      Workspace whose session_raw transcripts are read
  --model <model>         Optional model_override passed to openhuman.inference_agent_chat
  --query <text>          Add a custom query (repeatable). Replaces the defaults.
  --raw                   Send the query unwrapped (let the orchestrator decide
                          whether to call the tool) instead of forcing the call.
  --scout-prompt-file <f> Override the context_scout system prompt with this file
                          (writes a temporary workspace agent override; restored
                          after the run unless --keep-workspace). Test your prompt.
  --thread-prefix <s>     Thread id prefix to isolate transcripts (default: random)
  --max-print-chars <n>   Truncate printed bundle/thought blocks (default: 4000)
  --rpc-timeout-ms <n>    Per-RPC timeout (default: 600000)
  --spawn-core            Start \`cargo run --bin openhuman-core\` for the audit
  --keep-workspace        Keep any temp override files written for --scout-prompt-file
  --json                  Print a machine-readable JSON summary at the end
  --verbose               Stream spawned core logs
  -h, --help              Show this help

Examples:
  node scripts/debug/agent-prepare-context-audit.mjs --spawn-core
  node scripts/debug/agent-prepare-context-audit.mjs --query "what are my goals?" --model claude-sonnet-4-6
  node scripts/debug/agent-prepare-context-audit.mjs --scout-prompt-file /tmp/my-scout.md
`;
}

function parseArgs(argv) {
  const opts = {
    coreUrl: process.env.OPENHUMAN_CORE_RPC_URL || DEFAULT_RPC_URL,
    token: process.env.OPENHUMAN_CORE_TOKEN || "",
    workspace: process.env.OPENHUMAN_WORKSPACE || "",
    model: "",
    queries: [],
    raw: false,
    scoutPromptFile: "",
    threadPrefix: `apc-audit-${randomBytes(3).toString("hex")}`,
    maxPrintChars: 4000,
    rpcTimeoutMs: 600_000,
    spawnCore: false,
    keepWorkspace: false,
    json: false,
    verbose: false,
    coreUrlExplicit: Boolean(process.env.OPENHUMAN_CORE_RPC_URL),
    workspaceExplicit: Boolean(process.env.OPENHUMAN_WORKSPACE),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const value = argv[++i];
      if (value === undefined) throw new Error(`missing value for ${arg}`);
      return value;
    };
    switch (arg) {
      case "--core-url": opts.coreUrl = next(); opts.coreUrlExplicit = true; break;
      case "--token": opts.token = next(); break;
      case "--workspace": opts.workspace = next(); opts.workspaceExplicit = true; break;
      case "--model": opts.model = next(); break;
      case "--query": opts.queries.push(next()); break;
      case "--raw": opts.raw = true; break;
      case "--scout-prompt-file": opts.scoutPromptFile = next(); break;
      case "--thread-prefix": opts.threadPrefix = next(); break;
      case "--max-print-chars": opts.maxPrintChars = parsePositiveInt(next(), "--max-print-chars"); break;
      case "--rpc-timeout-ms": opts.rpcTimeoutMs = parsePositiveInt(next(), "--rpc-timeout-ms"); break;
      case "--spawn-core": opts.spawnCore = true; break;
      case "--keep-workspace": opts.keepWorkspace = true; break;
      case "--json": opts.json = true; break;
      case "--verbose": opts.verbose = true; break;
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
  if (!Number.isInteger(value) || value < 1) throw new Error(`${label} must be a positive integer`);
  return value;
}

function defaultOpenhumanDir() {
  if (process.env.OPENHUMAN_APP_ENV === "staging") {
    return path.join(homedir(), ".openhuman-staging");
  }
  if (process.env.OPENHUMAN_APP_ENV) {
    return path.join(homedir(), ".openhuman");
  }
  // APP_ENV unset: the core (launched from a shell that may export
  // OPENHUMAN_APP_ENV=staging) and this script can disagree. Auto-pick the
  // dir whose active_user.toml was touched most recently so transcript reads
  // land in the same env the core actually uses. Falls back to prod.
  const prod = path.join(homedir(), ".openhuman");
  const staging = path.join(homedir(), ".openhuman-staging");
  const mtime = (p) => {
    try {
      return statSync(path.join(p, "active_user.toml")).mtimeMs;
    } catch {
      return -1;
    }
  };
  return mtime(staging) > mtime(prod) ? staging : prod;
}

async function defaultWorkspace() {
  if (process.env.OPENHUMAN_WORKSPACE) return process.env.OPENHUMAN_WORKSPACE;
  const openhumanDir = defaultOpenhumanDir();
  try {
    const active = await readFile(path.join(openhumanDir, "active_user.toml"), "utf8");
    const match = active.match(/^\s*user_id\s*=\s*"([^"]+)"\s*$/m);
    if (match?.[1]) return path.join(openhumanDir, "users", match[1], "workspace");
  } catch {
    // fall through to legacy root
  }
  return openhumanDir;
}

async function readToken(opts) {
  if (opts.token.trim()) return opts.token.trim();
  const tokenPath = path.join(opts.workspace || (await defaultWorkspace()), "core.token");
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
      headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: `apc-${Date.now()}-${Math.random().toString(16).slice(2)}`,
        method,
        params,
      }),
    });
  } catch (err) {
    if (err?.name === "AbortError") throw new Error(`RPC ${method} timed out after ${timeoutMs}ms`);
    throw err;
  } finally {
    clearTimeout(timeout);
  }
  const bodyText = await res.text();
  let body;
  try {
    body = JSON.parse(bodyText);
  } catch {
    throw new Error(`RPC ${method} returned non-JSON HTTP ${res.status}: ${bodyText.slice(0, 200)}`);
  }
  if (!res.ok) throw new Error(`RPC ${method} HTTP ${res.status}`);
  if (body.error) throw new Error(`RPC ${method} error: ${JSON.stringify(body.error).slice(0, 300)}`);
  return body.result;
}

// ── Transcript reading ──────────────────────────────────────────────────────

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

async function readTranscript(file) {
  const data = await readFile(file, "utf8");
  const lines = data.split(/\r?\n/).filter((l) => l.trim());
  if (lines.length === 0) throw new Error("empty transcript");
  const meta = (JSON.parse(lines[0])._meta) || {};
  const messages = [];
  for (const line of lines.slice(1)) {
    try {
      const m = JSON.parse(line);
      if (typeof m.role === "string") messages.push(m);
    } catch {
      // skip malformed line
    }
  }
  return {
    file,
    agent: String(meta.agent || "(unknown)"),
    threadId: meta.thread_id || null,
    isSubagent: path.basename(file).includes("__"),
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
    files.map(async (file) => {
      try {
        map.set(file, await readTranscript(file));
      } catch {
        // ignore partial writes
      }
    }),
  );
  return map;
}

// Transcripts created or grown since `before`, scoped to a thread id.
function changedForThread(before, after, threadId) {
  const rows = [];
  for (const [file, cur] of after.entries()) {
    if (threadId && cur.threadId && cur.threadId !== threadId) continue;
    const prior = before.get(file);
    const grew = !prior || cur.input !== prior.input || cur.output !== prior.output || cur.messages.length !== prior.messages.length;
    if (grew) rows.push(cur);
  }
  return rows;
}

// ── Bundle parsing ──────────────────────────────────────────────────────────

function extractBundle(text) {
  if (typeof text !== "string") return null;
  const open = text.indexOf("[context_bundle]");
  if (open === -1) return null;
  const close = text.indexOf("[/context_bundle]", open);
  const inner = text.slice(open + "[context_bundle]".length, close === -1 ? undefined : close).trim();
  const hasEnough = /has_enough_context\s*:\s*(true|false)/i.exec(inner)?.[1] ?? "(unspecified)";
  // summary: everything between `summary:` and `recommended_tool_calls:`
  const sumMatch = /summary\s*:\s*([\s\S]*?)(?:\n\s*recommended_tool_calls\s*:|$)/i.exec(inner);
  const summary = (sumMatch?.[1] || "").trim();
  const recIdx = inner.search(/recommended_tool_calls\s*:/i);
  const recBlock = recIdx === -1 ? "" : inner.slice(recIdx).replace(/^[^\n]*\n?/, "");
  const tools = [];
  for (const m of recBlock.matchAll(/(?:^|\n)\s*-?\s*tool\s*:\s*([^\n]+)/gi)) {
    tools.push(m[1].trim());
  }
  return { raw: text.slice(open, close === -1 ? undefined : close + "[/context_bundle]".length), hasEnough, summary, tools, full: inner };
}

function clip(text, max) {
  if (typeof text !== "string") return "";
  if (text.length <= max) return text;
  return `${text.slice(0, max)}\n…[truncated ${text.length - max} chars]`;
}

// ── Prompt shaping ──────────────────────────────────────────────────────────

function forcedPrompt(query) {
  return `Call the \`agent_prepare_context\` tool now, passing \`question\` set to the user request below. \
Do not answer the request yourself first — scout context first. After the tool returns, reply with the \
returned context bundle verbatim, then one short line on how you'd proceed.

User request: ${query}`;
}

// ── Scout prompt override (optional) ─────────────────────────────────────────

function scoutOverrideToml(inlinePrompt) {
  // A full context_scout definition mirroring the builtin, with the system
  // prompt swapped for the user's file. Workspace overrides replace the builtin
  // on id collision, so every field the runtime relies on must be present.
  const esc = inlinePrompt.replace(/\\/g, "\\\\").replace(/"""/g, '\\"\\"\\"');
  return `id = "context_scout"
display_name = "Context Scout (override)"
when_to_use = "Pre-flight context collector (prompt override)."
temperature = 0.3
max_iterations = 6
iteration_policy = "extended"
max_result_chars = 4000
sandbox_mode = "read_only"
agent_tier = "worker"
omit_identity = true
omit_memory_context = true
omit_safety_preamble = true
omit_skills_catalog = true
omit_profile = false
omit_memory_md = false

[model]
hint = "agentic"

[system_prompt]
inline = """
${esc}
"""

[tools]
# Retrieval-only surface, matching the builtin context_scout. memory_tree is
# intentionally excluded: it bundles a write mode (ingest_document) under a
# ReadOnly wrapper, so it must not be reachable by the read-only scout — even
# via a prompt-override used in a "read-only" audit.
named = ["memory_recall", "web_search_tool", "web_fetch"]
`;
}

// ── Core spawn helpers (mirrors harness-cache-audit.mjs) ─────────────────────

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
  const token = opts.token || `apc-${randomBytes(24).toString("hex")}`;
  const env = { ...process.env, OPENHUMAN_CORE_TOKEN: token };
  // Only pin OPENHUMAN_WORKSPACE when the user explicitly asked for one.
  // Setting it to the auto-resolved active-user workspace makes the core
  // create a *nested* `.openhuman/` config dir without the signed-in
  // session (→ SESSION_EXPIRED). Leaving it unset lets the core resolve the
  // active user from ~/.openhuman/active_user.toml and load its live session;
  // transcripts then land in that same workspace the script reads.
  if (opts.workspaceExplicit && opts.workspace) env.OPENHUMAN_WORKSPACE = opts.workspace;
  const port = new URL(opts.coreUrl).port || "7788";
  env.OPENHUMAN_CORE_PORT = port;
  env.OPENHUMAN_CORE_RPC_URL = opts.coreUrl;
  const args = ["run", "--host", "127.0.0.1", "--port", port, "--jsonrpc-only"];
  const child = spawn("cargo", ["run", "--quiet", "--bin", "openhuman-core", "--", ...args], {
    cwd: path.resolve(SCRIPT_DIR, "../.."),
    env,
    stdio: opts.verbose ? ["ignore", "inherit", "inherit"] : ["ignore", "ignore", "pipe"],
  });
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
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`spawned core exited with ${child.exitCode}\n${stderrFn()}`);
    try {
      await rpc(coreUrl, token, "core.ping", {}, 10_000);
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 750));
    }
  }
  throw new Error(`timed out waiting for spawned core at ${coreUrl}\n${stderrFn()}`);
}

async function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  const exited = await Promise.race([
    once(child, "exit").then(() => true),
    new Promise((r) => setTimeout(() => r(false), 5_000)),
  ]);
  if (exited || child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGKILL");
  await Promise.race([once(child, "exit"), new Promise((r) => setTimeout(r, 2_000))]);
}

// ── Reporting ────────────────────────────────────────────────────────────────

function printCase(opts, caseInfo, scout, root, ms) {
  console.log(`\n${"═".repeat(78)}`);
  console.log(`▶ case: ${caseInfo.name}  (${ms}ms)`);
  console.log(`  query: ${caseInfo.query}`);

  if (!scout) {
    console.log("  ⚠ no context_scout transcript found — tool was not invoked.");
    console.log("    (Is the core built from this branch? Is agent_prepare_context allowlisted?)");
    return;
  }

  // Scout turns ("thoughts" — the model's step-by-step text + tool results).
  const assistantTurns = scout.messages.filter((m) => m.role === "assistant");
  const toolTurns = scout.messages.filter((m) => m.role === "tool");
  console.log(`\n  ── scout thoughts (${assistantTurns.length} assistant turn(s), ${toolTurns.length} tool result(s)) ──`);
  scout.messages
    .filter((m) => m.role === "assistant" || m.role === "tool")
    .forEach((m, i) => {
      const label = m.role === "assistant" ? "think" : "tool ←";
      console.log(`  [${i}] ${label}: ${clip(m.content, opts.maxPrintChars).replace(/\n/g, "\n        ")}`);
    });

  // Parsed bundle from the scout's final assistant message.
  const finalText = assistantTurns.at(-1)?.content || "";
  const bundle = extractBundle(finalText) || extractBundle(scout.messages.at(-1)?.content || "");
  console.log("\n  ── parsed context bundle ──");
  if (bundle) {
    console.log(`  has_enough_context: ${bundle.hasEnough}`);
    console.log(`  summary: ${clip(bundle.summary, opts.maxPrintChars).replace(/\n/g, "\n           ")}`);
    console.log(`  recommended_tool_calls (${bundle.tools.length}): ${bundle.tools.join(", ") || "(none)"}`);
  } else {
    console.log("  ⚠ scout output did not contain a [context_bundle] envelope. Raw final text:");
    console.log(`  ${clip(finalText, opts.maxPrintChars).replace(/\n/g, "\n  ")}`);
  }

  // Token / cost breakdown.
  const rows = [];
  rows.push(usageRow("context_scout", scout));
  if (root) rows.push(usageRow("orchestrator", root));
  rows.push({
    session: "TOTAL",
    in: (scout.input + (root?.input || 0)),
    out: (scout.output + (root?.output || 0)),
    cached: (scout.cached + (root?.cached || 0)),
    "cache%": cachePct(scout.input + (root?.input || 0), scout.cached + (root?.cached || 0)),
    cost_usd: round6(scout.charged + (root?.charged || 0)),
  });
  console.log("\n  ── tokens & cost ──");
  console.table(rows);
}

function usageRow(label, t) {
  return {
    session: label,
    in: t.input,
    out: t.output,
    cached: t.cached,
    "cache%": cachePct(t.input, t.cached),
    cost_usd: round6(t.charged),
  };
}

function cachePct(input, cached) {
  return input > 0 ? `${((cached / input) * 100).toFixed(1)}%` : "0.0%";
}

function round6(n) {
  return Number(n.toFixed(6));
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.workspace) opts.workspace = await defaultWorkspace();

  const cases = (opts.queries.length > 0 ? opts.queries.map((q, i) => ({ name: `custom-${i + 1}`, query: q })) : DEFAULT_CASES);

  // Write the optional context_scout prompt override BEFORE the core starts,
  // so a spawned core loads it (the override is read at boot). For attached
  // mode the caller must have already started the core after writing it — but
  // writing first here still beats writing after, and we warn below.
  let overridePath = "";
  if (opts.scoutPromptFile) {
    const promptBody = await readFile(opts.scoutPromptFile, "utf8");
    const agentsDir = path.join(opts.workspace, "agents");
    await mkdir(agentsDir, { recursive: true });
    overridePath = path.join(agentsDir, "context_scout.toml");
    await writeFile(overridePath, scoutOverrideToml(promptBody.trim()));
    console.log(`[apc-audit] wrote scout prompt override → ${overridePath}`);
    if (!opts.spawnCore) {
      console.log(
        "[apc-audit] NOTE: attached mode — restart your core now so it loads the override before audits run.",
      );
    }
  }

  let spawned;
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

  console.log("[apc-audit] starting live agent_prepare_context audit");
  console.log(`  rpc:        ${opts.coreUrl}`);
  console.log(`  workspace:  ${opts.workspace}`);
  console.log(`  mode:       ${opts.spawnCore ? "spawned-core (this branch)" : "attached-core"}`);
  console.log(`  model:      ${opts.model || "(account default)"}`);
  console.log(`  cases:      ${cases.length}${opts.raw ? " (raw — not forcing the tool)" : " (forcing the tool)"}`);

  const caseResults = [];
  try {
    for (let i = 0; i < cases.length; i += 1) {
      const c = cases[i];
      const threadId = `${opts.threadPrefix}-${i}`;
      const before = await snapshot(opts.workspace);
      const params = { message: opts.raw ? c.query : forcedPrompt(c.query), thread_id: threadId };
      if (opts.model) params.model_override = opts.model;
      const started = Date.now();
      let rpcError = "";
      try {
        await rpc(opts.coreUrl, opts.token, "openhuman.inference_agent_chat", params, opts.rpcTimeoutMs);
      } catch (err) {
        rpcError = err.message;
      }
      const ms = Date.now() - started;
      const after = await snapshot(opts.workspace);
      const changed = changedForThread(before, after, threadId);
      const scout = changed.find((t) => t.isSubagent && t.agent === SCOUT_AGENT) || changed.find((t) => t.agent === SCOUT_AGENT);
      const root = changed.find((t) => !t.isSubagent);

      if (rpcError) console.log(`\n▶ case: ${c.name} — RPC error: ${rpcError}`);
      printCase(opts, c, scout, root, ms);

      const finalText = (scout?.messages.filter((m) => m.role === "assistant").at(-1)?.content) || "";
      const bundle = scout ? extractBundle(finalText) : null;
      caseResults.push({
        name: c.name,
        query: c.query,
        ms,
        invoked: Boolean(scout),
        hasEnough: bundle?.hasEnough ?? null,
        recommended: bundle?.tools ?? [],
        scout: scout ? pick(scout) : null,
        orchestrator: root ? pick(root) : null,
        rpcError: rpcError || null,
      });
    }
  } finally {
    if (spawned?.child) await stopChild(spawned.child);
    if (overridePath && !opts.keepWorkspace) {
      await rm(overridePath, { force: true });
      console.log(`\n[apc-audit] removed scout prompt override ${overridePath}`);
    }
  }

  // Aggregate.
  const agg = caseResults.reduce(
    (a, r) => {
      a.invoked += r.invoked ? 1 : 0;
      a.in += (r.scout?.input || 0) + (r.orchestrator?.input || 0);
      a.out += (r.scout?.output || 0) + (r.orchestrator?.output || 0);
      a.cached += (r.scout?.cached || 0) + (r.orchestrator?.cached || 0);
      a.cost += (r.scout?.charged || 0) + (r.orchestrator?.charged || 0);
      return a;
    },
    { invoked: 0, in: 0, out: 0, cached: 0, cost: 0 },
  );

  console.log(`\n${"═".repeat(78)}`);
  console.log("[apc-audit] summary");
  console.table(
    caseResults.map((r) => ({
      case: r.name,
      invoked: r.invoked ? "yes" : "NO",
      enough: r.hasEnough ?? "-",
      rec_tools: r.recommended.length,
      ms: r.ms,
      cost_usd: round6((r.scout?.charged || 0) + (r.orchestrator?.charged || 0)),
    })),
  );
  console.log(`  tool invoked: ${agg.invoked}/${caseResults.length}`);
  console.log(`  total tokens: ${agg.in} in / ${agg.out} out / ${agg.cached} cached (${cachePct(agg.in, agg.cached)} hit)`);
  console.log(`  total cost:   $${round6(agg.cost)}`);

  if (opts.json) {
    console.log(`\n[apc-audit] json\n${JSON.stringify({ cases: caseResults, totals: { ...agg, cost: round6(agg.cost) } }, null, 2)}`);
  }

  const everInvoked = agg.invoked > 0;
  if (!everInvoked) {
    console.error("\n[apc-audit] FAIL: agent_prepare_context was never invoked (no context_scout transcript).");
    process.exit(1);
  }
  console.log("\n[apc-audit] done");
}

function pick(t) {
  return { input: t.input, output: t.output, cached: t.cached, charged: t.charged, file: t.file };
}

main().catch((err) => {
  console.error(`[apc-audit] ERROR: ${err.message}`);
  process.exit(1);
});
