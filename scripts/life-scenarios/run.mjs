#!/usr/bin/env node
/**
 * Life-scenario harness benchmark — headless, but shaped like the desktop app.
 *
 * The point is to measure the harness the product actually ships, so the
 * default path here is the one the desktop composer takes, with the UI removed
 * and nothing else changed:
 *
 *   | desktop app                              | here                          |
 *   | ---------------------------------------- | ----------------------------- |
 *   | Tauri spawns the core as a tokio task     | spawn `openhuman-core serve`  |
 *   | composer calls `openhuman.channel_web_chat`| same RPC, same params        |
 *   | reply streams over Socket.IO              | same events over `GET /events`|
 *   | orchestrator agent, packs withheld        | same — no custom definition   |
 *   | approval gate ON, user clicks Approve     | gate ON, responder approves   |
 *   | BYOK set in Settings → Models             | `config.update_model_settings`|
 *   | signed-in session                         | offline local session         |
 *
 * What is deliberately NOT the app: it runs under a `HOME` of its own, so it
 * never reads or writes the operator's `~/.openhuman` (installing a credential
 * there would sign a running desktop app out), inference goes to OpenRouter on
 * the caller's key rather than the hosted backend, and Composio is a local
 * mock over the same fixtures. Those three make a run reproducible and cost
 * what it says on the tin; everything else is the shipping path.
 *
 * `--driver rpc` switches to `openhuman.inference_agent_chat`, which is the
 * only path that can scope a per-turn `cwd` and name an `agent_id` — used for
 * the comparison arm against a custom agent definition.
 *
 * Usage:
 *   node scripts/life-scenarios/run.mjs                       # all scenarios
 *   node scripts/life-scenarios/run.mjs --only calendar-buffer
 *   node scripts/life-scenarios/run.mjs --driver rpc --agent life_scenarios
 *   node scripts/life-scenarios/run.mjs --grade-only <run-dir>
 */

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import fsp from "node:fs/promises";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { SCENARIOS, scenarioById } from "./scenarios.mjs";
import { startMockComposio } from "./mock-composio.mjs";
import { startMockSearch, DEFAULT_INDEX_PATH } from "./mock-search.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..");
const FIXTURES = path.join(HERE, "fixtures");
const SUPPORTED_AGENT_IDS = new Set(["life_scenarios", "orchestrator"]);

// ---------------------------------------------------------------------------
// args
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const o = {
    only: [],
    // `desktop` = channel_web_chat + SSE, exactly what the composer does.
    // `rpc`     = inference_agent_chat, the only path with `cwd`/`agent_id`.
    driver: "desktop",
    // The suite's benchmark agent (scripts/life-scenarios/agent-life-scenarios.toml):
    // 40 iterations and the named tool belt these multi-step scenarios need.
    // `--agent orchestrator` runs the unmodified shipping agent for comparison,
    // capped at the 15 iterations its own definition declares.
    //
    // This now takes effect on BOTH drivers: the rpc path passes it per call,
    // the desktop path gets it through `[agent] chat_agent_id` in the generated
    // config.
    agentId: "life_scenarios",
    model: process.env.LIFE_SCENARIO_MODEL || "deepseek/deepseek-v4.1-flash",
    inferenceUrl:
      process.env.LIFE_SCENARIO_INFERENCE_URL || "https://openrouter.ai/api/v1",
    apiKey: process.env.OPENROUTER_API_KEY || "",
    managed: false,
    mockComposio: true,
    composioPort: 0,
    mockSearch: true,
    searchPort: 0,
    repeat: 1,
    turnTimeoutMs: 900_000,
    coreBin:
      process.env.OPENHUMAN_CORE_BIN ||
      path.join(REPO, "target", "debug", "openhuman-core"),
    runRoot: path.join(REPO, "target", "life-scenarios"),
    gradeOnly: "",
    // The desktop app ships the approval gate ON and a human answers it. The
    // headless equivalent is a responder, not a disabled gate — see
    // `ApprovalResponder`. `--no-approvals` disables the gate instead, which
    // is what most headless harnesses do and is worth being able to compare.
    approvals: true,
    verbose: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) throw new Error(`missing value for ${a}`);
      return v;
    };
    if (a === "--only")
      o.only = next().split(",").map((s) => s.trim()).filter(Boolean);
    else if (a === "--driver") o.driver = next();
    else if (a === "--agent") {
      const agentId = next();
      if (!/^[A-Za-z0-9_-]+$/.test(agentId)) {
        throw new Error(
          "--agent must contain only ASCII letters, digits, '_' or '-'",
        );
      }
      if (!SUPPORTED_AGENT_IDS.has(agentId)) {
        throw new Error(
          `--agent must be one of: ${[...SUPPORTED_AGENT_IDS].join(", ")}`,
        );
      }
      o.agentId = agentId;
    }
    else if (a === "--model") o.model = next();
    else if (a === "--inference-url") o.inferenceUrl = next();
    else if (a === "--api-key") o.apiKey = next();
    else if (a === "--managed") o.managed = true;
    else if (a === "--no-mock-composio") o.mockComposio = false;
    else if (a === "--no-mock-search") o.mockSearch = false;
    else if (a === "--no-approvals") o.approvals = false;
    else if (a === "--repeat") o.repeat = Number(next());
    else if (a === "--turn-timeout-ms") o.turnTimeoutMs = Number(next());
    else if (a === "--core-bin") o.coreBin = next();
    else if (a === "--run-root") o.runRoot = next();
    else if (a === "--grade-only") o.gradeOnly = next();
    else if (a === "--verbose" || a === "-v") o.verbose = true;
    else if (a === "-h" || a === "--help") {
      console.log(fs.readFileSync(path.join(HERE, "README.md"), "utf8"));
      process.exit(0);
    } else throw new Error(`unknown flag ${a}`);
  }
  if (!["desktop", "rpc"].includes(o.driver))
    throw new Error(`--driver must be 'desktop' or 'rpc', got '${o.driver}'`);
  return o;
}

// ---------------------------------------------------------------------------
// small utilities
// ---------------------------------------------------------------------------

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

async function copyDir(from, to) {
  await fsp.mkdir(to, { recursive: true });
  for (const entry of await fsp.readdir(from, { withFileTypes: true })) {
    const s = path.join(from, entry.name);
    const d = path.join(to, entry.name);
    if (entry.isDirectory()) await copyDir(s, d);
    else await fsp.copyFile(s, d);
  }
}

/** Walk a directory and return [{ rel, bytes, mtimeMs }] for every file. */
async function snapshotTree(root) {
  const out = [];
  async function walk(dir) {
    let entries;
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) await walk(p);
      else {
        const st = await fsp.stat(p).catch(() => null);
        if (st)
          out.push({ rel: path.relative(root, p), bytes: st.size, mtimeMs: st.mtimeMs });
      }
    }
  }
  await walk(root);
  return out;
}

/**
 * Mint the core's offline local session token.
 *
 * The desktop app installs a real session JWT after login. A headless run has
 * no login flow, but the core ships a third credential kind for exactly this:
 * a JWT-shaped token whose signature segment is the literal `local`
 * (`security::credentials::session_support::is_local_session_token`). It buys
 * no access to the hosted backend — it only lets a local host say whose turn
 * this is, which is all that is needed when inference is BYOK and Composio is
 * mocked.
 */
function mintLocalSessionToken(userId) {
  const b64 = (o) =>
    Buffer.from(JSON.stringify(o))
      .toString("base64")
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
  const now = Math.floor(Date.now() / 1000);
  return [
    b64({ alg: "none", typ: "JWT" }),
    b64({
      sub: userId,
      iat: now,
      exp: now + 24 * 60 * 60,
      email: "life-scenarios@local.invalid",
    }),
    "local",
  ].join(".");
}

// ---------------------------------------------------------------------------
// the throwaway HOME
// ---------------------------------------------------------------------------

/**
 * Write the config a freshly-onboarded desktop install would have.
 *
 * Written from scratch rather than copied: a copied `config.toml` drags along
 * whatever `api_url`, provider pins and autonomy settings that machine happens
 * to have, and a benchmark that silently inherits those measures the machine
 * rather than the harness.
 */
async function prepareHome(runDir, opts, { searchBase } = {}) {
  const home = path.join(runDir, "home");
  const oh = path.join(home, ".openhuman");
  await fsp.mkdir(path.join(oh, "agents"), { recursive: true });
  await fsp.mkdir(path.join(oh, "users", "local"), { recursive: true });

  const config = [
    "schema_version = 13",
    // The backend base every non-inference call resolves through
    // (`openhuman_tinyhumans::backend::url::effective_backend_api_url`). Pointed at the local mock so
    // `web_search_tool` has something to talk to: it posts to
    // `/agent-integrations/parallel/search` on this base, and against the
    // hosted backend this run's offline token is rejected 401 every time.
    // See also BACKEND_URL in `Core.start` — this file alone is not enough.
    searchBase
      ? `api_url = "${searchBase}"`
      : 'api_url = "https://api.tinyhumans.ai"',
    "default_temperature = 0.7",
    "onboarding_completed = true",
    "chat_onboarding_completed = true",
    "",
    "[autonomy]",
    // The shipped desktop default is `enabled = false`: the policy is opt-in,
    // because the product's agents run in containers and jails that already
    // bound them. Written explicitly rather than left to the default so a
    // reader of this file can see which product is being measured, and so
    // flipping it to `true` is a one-line comparison arm.
    //
    // With it off, `gate_decision` answers Allow for every class, so nothing
    // parks and the ApprovalResponder below has little to answer. That is the
    // measurement, not a shortcut: see README.md.
    "enabled = false",
    'level = "supervised"',
    "workspace_only = false",
    "",
    // The web-chat path (`channel_web_chat`, the desktop driver below) has no
    // per-call `agent_id` the way `inference_agent_chat` does, so this is how
    // it is pointed at the suite's benchmark agent. Without it that path runs
    // `orchestrator`. A definition cap replaces `[agent] max_tool_iterations`,
    // so only `max_tool_iterations_override` changes the budget without
    // changing which agent (prompt, tools) answers.
    "[agent]",
    `chat_agent_id = "${opts.agentId}"`,
    "",
    "[observability]",
    "analytics_enabled = false",
    "share_usage_data = false",
    "",
    // `create_composio_client` dispatches on this field alone; the
    // `OPENHUMAN_COMPOSIO_DIRECT_BASE_V*` env pair is consulted only in
    // `direct` mode, and only in a debug build. Without it the core ignores
    // the mock and dials the hosted proxy.
    "[composio]",
    'mode = "direct"',
    'api_key = "ck_life_scenarios_mock"',
    'entity_id = "default"',
    "",
  ].join("\n");
  await fsp.writeFile(path.join(oh, "config.toml"), config);
  // Once a user is active the per-user config is read in preference to the
  // root one, so the composio block has to exist in both.
  await fsp.writeFile(path.join(oh, "users", "local", "config.toml"), config);

  // Read by both drivers now: the rpc path names it per call, the desktop path
  // selects it with `[agent] chat_agent_id` above. `--agent orchestrator` opts
  // back into the unmodified shipping agent.
  await fsp.copyFile(
    path.join(HERE, "agent-life-scenarios.toml"),
    path.join(oh, "agents", "life_scenarios.toml"),
  );

  return home;
}

/** Retry `fn` until it stops throwing, then give up with the last error. */
async function withRetries(fn, { attempts, delayMs, what }) {
  let last;
  for (let i = 0; i < attempts; i += 1) {
    try {
      await fn();
      return;
    } catch (e) {
      last = e;
      await new Promise((r) => setTimeout(r, delayMs));
    }
  }
  throw new Error(`${what} never settled after ${attempts} attempts: ${last?.message ?? last}`);
}

// ---------------------------------------------------------------------------
// core lifecycle
// ---------------------------------------------------------------------------

class Core {
  constructor(opts) {
    this.opts = opts;
    this.proc = null;
    this.port = 0;
    this.token = randomBytes(24).toString("hex");
    this.exited = null;
  }

  get url() {
    return `http://127.0.0.1:${this.port}`;
  }

  async start({ actionDir, logPath, home, composioBase, searchBase, approvals }) {
    this.port = await freePort();
    const log = fs.createWriteStream(logPath, { flags: "a" });

    const env = {
      ...process.env,
      // A private HOME is the isolation boundary: `default_root_openhuman_dir`
      // resolves `<home>/.openhuman`, so config, keyring, auth profiles,
      // workspace and session db all land inside the run directory.
      HOME: home,
      OPENHUMAN_HOME: path.join(home, ".openhuman"),
      OPENHUMAN_CORE_TOKEN: this.token,
      OPENHUMAN_CORE_PORT: String(this.port),
      OPENHUMAN_CORE_HOST: "127.0.0.1",
      // Just this one now. It used to need OPENHUMAN_PROJECTS_DIR beside it:
      // `action_dir` was only the base that relative tool paths are joined
      // onto, and the *permission* to write came from a trusted root that
      // `security/policy/enforcement.rs` granted for `default_projects_dir()`
      // alone — so setting ACTION_DIR by itself got every file-tool write
      // refused "Resolved path escapes workspace" (FINDINGS.md #1).
      // `from_config` now grants the configured action dir itself, which is
      // what this single variable proves end to end.
      OPENHUMAN_ACTION_DIR: actionDir,
      RUST_LOG: process.env.RUST_LOG || "info",
    };
    if (!approvals) env.OPENHUMAN_APPROVAL_GATE = "0";
    if (process.env.BACKEND_URL) env.BACKEND_URL = process.env.BACKEND_URL;
    // The same backend base as `api_url` above, set again as an env var
    // because the config file loses a race that is easy to miss:
    // `auth.set_credential` activates a per-user config dir
    // (`users/<id>/config.toml`) whose id the core derives at runtime, and a
    // config there takes precedence over the root one. `prepareHome` cannot
    // know that id, so it writes `users/local/`; the core activates
    // `users/local-dragonfly/`, finds no `api_url` and falls back to the
    // hosted backend. `api_base_from_env` reads BACKEND_URL ahead of the
    // compile-time default whichever config wins, so this is the override
    // that actually holds.
    if (searchBase) env.BACKEND_URL = searchBase;
    if (composioBase) {
      // Both are read by `integrations/composio/client/factory.rs`; the match
      // arm is `(Some, Some)`, so setting only one silently falls through to
      // the production Composio URLs.
      env.OPENHUMAN_COMPOSIO_DIRECT_BASE_V3 = composioBase;
      env.OPENHUMAN_COMPOSIO_DIRECT_BASE_V2 = composioBase;
    }

    // Plain `serve`, not `--jsonrpc-only`: the app boots the full service set,
    // and the background services are part of what a turn costs.
    this.proc = spawn(this.opts.coreBin, ["serve"], {
      env,
      stdio: ["ignore", "pipe", "pipe"],
      cwd: actionDir,
    });
    this.proc.stdout.pipe(log);
    this.proc.stderr.pipe(log);
    this.proc.on("exit", (code, sig) => {
      this.exited = { code, sig };
    });

    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline) {
      if (this.exited)
        throw new Error(
          `core exited during boot (code=${this.exited.code} sig=${this.exited.sig}); see ${logPath}`,
        );
      try {
        const r = await fetch(`${this.url}/health`, {
          signal: AbortSignal.timeout(3000),
        });
        if (r.ok) return await r.json();
      } catch {
        /* not up yet */
      }
      await sleep(500);
    }
    throw new Error(`core did not become healthy on ${this.url}; see ${logPath}`);
  }

  async rpc(method, params, timeoutMs = 120_000) {
    const res = await fetch(`${this.url}/rpc`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${this.token}`,
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: `ls-${Date.now()}`, method, params }),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const text = await res.text();
    if (!res.ok)
      throw new Error(`RPC ${method} HTTP ${res.status}: ${text.slice(0, 400)}`);
    let body;
    try {
      body = JSON.parse(text);
    } catch {
      throw new Error(`RPC ${method} non-JSON: ${text.slice(0, 300)}`);
    }
    if (body.error)
      throw new Error(`RPC ${method} error: ${JSON.stringify(body.error).slice(0, 500)}`);
    // `apply_log_envelope` wraps a result that carried log lines.
    const r = body.result;
    if (r && typeof r === "object" && "result" in r && "logs" in r) return r.result;
    return r;
  }

  async stop() {
    if (!this.proc || this.exited) return;
    this.proc.kill("SIGTERM");
    const deadline = Date.now() + 15_000;
    while (!this.exited && Date.now() < deadline) await sleep(100);
    if (!this.exited) this.proc.kill("SIGKILL");
  }
}

// ---------------------------------------------------------------------------
// SSE — the composer's event stream
// ---------------------------------------------------------------------------

/**
 * Subscribe to `GET /events?client_id=…`, the same stream the desktop
 * frontend consumes. Events are delivered to `onEvent` as parsed objects.
 */
class EventStream {
  constructor(core, clientId) {
    this.core = core;
    this.clientId = clientId;
    this.handlers = new Set();
    this.controller = new AbortController();
    this.closed = false;
  }

  onEvent(fn) {
    this.handlers.add(fn);
    return () => this.handlers.delete(fn);
  }

  async connect() {
    const url = `${this.core.url}/events?client_id=${encodeURIComponent(this.clientId)}`;
    const res = await fetch(url, {
      headers: { authorization: `Bearer ${this.core.token}`, accept: "text/event-stream" },
      signal: this.controller.signal,
    });
    if (!res.ok || !res.body)
      throw new Error(`events subscribe failed: HTTP ${res.status}`);
    // Pump in the background; the caller awaits chat_done, not this.
    this.pump = (async () => {
      const decoder = new TextDecoder();
      let buffer = "";
      try {
        for await (const chunk of res.body) {
          buffer += decoder.decode(chunk, { stream: true });
          let idx;
          while ((idx = buffer.indexOf("\n\n")) >= 0) {
            const frame = buffer.slice(0, idx);
            buffer = buffer.slice(idx + 2);
            for (const line of frame.split("\n")) {
              if (!line.startsWith("data:")) continue;
              const payload = line.slice(5).trim();
              if (!payload) continue;
              let event;
              try {
                event = JSON.parse(payload);
              } catch {
                continue;
              }
              for (const h of this.handlers) h(event);
            }
          }
        }
      } catch {
        /* aborted or core stopped */
      }
      this.closed = true;
    })();
  }

  close() {
    this.controller.abort();
  }
}

// ---------------------------------------------------------------------------
// approvals — what the human in front of the app does
// ---------------------------------------------------------------------------

/**
 * Stand in for the person clicking "Approve" on the approval card.
 *
 * The gate stays installed exactly as it ships; this only supplies the
 * decision it is waiting for. Without a responder, a headless turn parks on
 * the first gated tool call and the request expires as DENIED after ten
 * minutes — so a harness that simply disables the gate is not measuring the
 * product, and one that leaves it unanswered is measuring the timeout.
 *
 * Every decision is recorded, because "how many approvals did this task need"
 * is itself a result: a task that needs fourteen is not one a supervised user
 * would enjoy.
 */
class ApprovalResponder {
  constructor(core, { intervalMs = 400 } = {}) {
    this.core = core;
    this.intervalMs = intervalMs;
    this.decisions = [];
    this.seen = new Set();
    this.running = false;
    this.errors = [];
  }

  start() {
    this.running = true;
    this.loop = (async () => {
      while (this.running) {
        try {
          const pending = await this.core.rpc("openhuman.approval_list_pending", {}, 15_000);
          const rows = Array.isArray(pending)
            ? pending
            : (pending && (pending.requests || pending.pending || pending.items)) || [];
          for (const row of rows) {
            // `PendingApproval.request_id` — the decide RPC takes `request_id`,
            // not `id`, and a wrong key comes back as a redacted
            // param-validation error that names neither the field nor the
            // method's expectation.
            const requestId = row.request_id || row.requestId;
            if (!requestId || this.seen.has(requestId)) continue;
            this.seen.add(requestId);
            await this.core.rpc(
              "openhuman.approval_decide",
              { request_id: requestId, decision: "approve_once" },
              15_000,
            );
            this.decisions.push({
              at: new Date().toISOString(),
              request_id: requestId,
              tool: row.tool_name || "",
              summary: (row.action_summary || "").slice(0, 200),
            });
          }
        } catch (e) {
          // A transient failure here must not kill the run; record it so a
          // silent approval stall is still visible afterwards.
          this.errors.push(String(e.message).slice(0, 200));
        }
        await sleep(this.intervalMs);
      }
    })();
  }

  async stop() {
    this.running = false;
    await this.loop?.catch(() => {});
  }

  /** Decisions made while `fn` ran. */
  countSince(mark) {
    return this.decisions.length - mark;
  }
}

// ---------------------------------------------------------------------------
// usage accounting
// ---------------------------------------------------------------------------

/**
 * `openhuman.cost_get_usage_log` returns one row per provider call. Diffing
 * row ids across a turn is exact where a timestamp window would not be: the
 * background services (memory, learning) bill against the same log
 * while a turn runs, and a window would sweep them in.
 */
async function usageIds(core) {
  const log = await core.rpc("openhuman.cost_get_usage_log", { days: 1, limit: 1000 });
  const records = (log && log.records) || [];
  return new Map(records.map((r) => [r.id, r]));
}

function foldUsage(rows) {
  const t = {
    calls: rows.length,
    input_tokens: 0,
    output_tokens: 0,
    cached_input_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    cost_usd: 0,
  };
  const models = new Set();
  const sources = new Set();
  for (const r of rows) {
    t.input_tokens += Number(r.input_tokens || 0);
    t.output_tokens += Number(r.output_tokens || 0);
    t.cached_input_tokens += Number(r.cached_input_tokens || 0);
    t.cache_creation_tokens += Number(r.cache_creation_tokens || 0);
    t.reasoning_tokens += Number(r.reasoning_tokens || 0);
    t.cost_usd += Number(r.cost_usd || 0);
    if (r.model) models.add(r.model);
    if (r.cost_source) sources.add(r.cost_source);
  }
  return {
    ...t,
    models: [...models],
    cost_sources: [...sources],
    cache_hit_pct: t.input_tokens > 0 ? (t.cached_input_tokens / t.input_tokens) * 100 : 0,
  };
}

/** Prefer the turn's own `chat_done.usage`; fall back to the cost-log diff. */
function reconcileUsage(chatDoneUsage, loggedUsage) {
  if (!chatDoneUsage) return { ...loggedUsage, source: "cost_log" };
  const input = Number(chatDoneUsage.input_tokens || 0);
  const cached = Number(chatDoneUsage.cached_input_tokens || 0);
  return {
    calls: loggedUsage.calls,
    input_tokens: input,
    output_tokens: Number(chatDoneUsage.output_tokens || 0),
    cached_input_tokens: cached,
    cache_creation_tokens: loggedUsage.cache_creation_tokens,
    reasoning_tokens: loggedUsage.reasoning_tokens,
    cost_usd: Number(chatDoneUsage.cost_usd || 0) || loggedUsage.cost_usd,
    models: loggedUsage.models,
    cost_sources: loggedUsage.cost_sources,
    cache_hit_pct: input > 0 ? (cached / input) * 100 : 0,
    context_window: Number(chatDoneUsage.context_window || 0),
    subagents: chatDoneUsage.subagents || [],
    source: "chat_done",
  };
}

// ---------------------------------------------------------------------------
// grading
// ---------------------------------------------------------------------------

function gradeScenario(scenario, sandbox, transcript) {
  const ctx = {
    read(rel) {
      try {
        return fs.readFileSync(path.join(sandbox, rel), "utf8");
      } catch {
        return null;
      }
    },
    exists(rel) {
      return fs.existsSync(path.join(sandbox, rel));
    },
    transcript: transcript || "",
    sandbox,
  };
  let checks;
  try {
    ({ checks } = scenario.grade(ctx));
  } catch (e) {
    checks = [
      { id: "grader_crashed", ok: false, detail: String(e.message).slice(0, 200) },
    ];
  }
  const passed = checks.filter((c) => c.ok).length;
  return {
    checks,
    passed,
    total: checks.length,
    score: checks.length ? passed / checks.length : 0,
  };
}

// ---------------------------------------------------------------------------
// the two drivers
// ---------------------------------------------------------------------------

/**
 * The desktop path: `channel_web_chat` acks immediately and the answer arrives
 * on the event stream as `chat_done`, carrying the turn's own usage payload.
 */
async function sendDesktopTurn({ core, events, clientId, threadId, message, opts }) {
  const toolCalls = [];
  let done = null;
  let requestId = null;

  const finished = new Promise((resolve) => {
    const off = events.onEvent((ev) => {
      if (ev.client_id && ev.client_id !== clientId) return;
      if (ev.event === "tool_call")
        toolCalls.push({
          name: ev.tool_name || ev.tool || "",
          label: ev.tool_display_label || "",
        });
      // A failed turn ends in `chat_error`, not `chat_done` — a driver that
      // waits only for `chat_done` hangs until its own timeout on every
      // provider misconfiguration, and reports it as a timeout.
      if (ev.event === "chat_done" || ev.event === "chat_error") {
        // The ack may not have landed yet, so match loosely on thread when the
        // request id is not yet known.
        if (requestId && ev.request_id && ev.request_id !== requestId) return;
        if (!requestId && ev.thread_id && ev.thread_id !== threadId) return;
        done = ev;
        off();
        resolve(ev);
      }
    });
  });

  const ack = await core.rpc(
    "openhuman.channel_web_chat",
    {
      client_id: clientId,
      thread_id: threadId,
      message,
      source: "type",
      queue_mode: "interrupt",
      ...(opts.model ? { model_override: opts.model } : {}),
    },
    120_000,
  );
  requestId = (ack && (ack.request_id || ack.requestId)) || null;

  const timeout = sleep(opts.turnTimeoutMs).then(() => "timeout");
  const outcome = await Promise.race([finished, timeout]);
  if (outcome === "timeout")
    return { error: `turn did not emit chat_done within ${opts.turnTimeoutMs}ms`, requestId, toolCalls };

  if (done && done.event === "chat_error")
    return {
      error: `chat_error (${done.error_type || "unknown"}): ${String(done.message || "").slice(0, 300)}`,
      requestId: requestId || done.request_id || null,
      toolCalls,
    };

  return {
    reply: (done && (done.full_response || done.response)) || "",
    usage: done && done.usage,
    requestId: requestId || (done && done.request_id) || null,
    toolCalls,
  };
}

/** The RPC path: synchronous, and the only one that can scope `cwd`. */
async function sendRpcTurn({ core, threadId, message, sandbox, opts }) {
  const reply = await core.rpc(
    "openhuman.inference_agent_chat",
    {
      message,
      thread_id: threadId,
      cwd: sandbox,
      ...(opts.agentId ? { agent_id: opts.agentId } : {}),
      ...(opts.managed
        ? opts.model
          ? { model_override: opts.model }
          : {}
        : {
            model_override: opts.model,
            inference_url: opts.inferenceUrl,
            api_key: opts.apiKey,
          }),
    },
    opts.turnTimeoutMs,
  );
  return { reply: typeof reply === "string" ? reply : "", usage: null, requestId: null, toolCalls: [] };
}

// ---------------------------------------------------------------------------
// run one scenario
// ---------------------------------------------------------------------------

async function runScenario({ core, events, clientId, approvals, scenario, runDir, opts, attempt }) {
  const sandboxRoot = path.join(runDir, "sandbox");
  const sandbox = path.join(sandboxRoot, scenario.id);
  await fsp.rm(sandbox, { recursive: true, force: true });
  await copyDir(FIXTURES, sandbox);
  await fsp.mkdir(path.join(sandbox, "out"), { recursive: true });
  const before = await snapshotTree(sandbox);

  const usageBefore = await usageIds(core);
  const approvalsMark = approvals ? approvals.decisions.length : 0;
  const threadId = `ls-${scenario.id}-${attempt}-${randomBytes(3).toString("hex")}`;

  // The desktop driver has no per-turn `cwd`: the action dir is the whole
  // sandbox root, so the task has to say which folder inside it is its own.
  // The RPC driver scopes `cwd` to the folder itself and needs no preamble.
  const message =
    opts.driver === "desktop"
      ? `All paths in this task are relative to the folder \`${scenario.id}/\` inside your action directory. Work only inside that folder.\n\n${scenario.prompt}`
      : scenario.prompt;

  const t0 = Date.now();
  let turn;
  let error = null;
  try {
    turn =
      opts.driver === "desktop"
        ? await sendDesktopTurn({ core, events, clientId, threadId, message, opts })
        : await sendRpcTurn({ core, threadId, message, sandbox, opts });
    if (turn.error) error = turn.error;
  } catch (e) {
    error = String(e.message).slice(0, 600);
    turn = { reply: "", usage: null, requestId: null, toolCalls: [] };
  }
  const latencyMs = Date.now() - t0;

  // Cost rows can land a beat after the turn reports done.
  await sleep(1500);
  const usageAfter = await usageIds(core);
  const newRows = [...usageAfter.entries()]
    .filter(([id]) => !usageBefore.has(id))
    .map(([, r]) => r);
  const usage = reconcileUsage(turn.usage, foldUsage(newRows));

  // The run ledger is written only by the web-chat bridge, so it exists on the
  // desktop driver and not the RPC one.
  let ledger = null;
  if (turn.requestId) {
    ledger = await core
      .rpc("openhuman.run_ledger_get", { id: turn.requestId })
      .then((r) => (r && r.run) || null)
      .catch(() => null);
  }

  const after = await snapshotTree(sandbox);
  const beforeSet = new Map(before.map((f) => [f.rel, f]));
  const written = after.filter(
    (f) => !beforeSet.has(f.rel) || beforeSet.get(f.rel).mtimeMs !== f.mtimeMs,
  );

  const grade = gradeScenario(scenario, sandbox, turn.reply);

  return {
    scenario: scenario.id,
    title: scenario.title,
    attempt,
    driver: opts.driver,
    thread_id: threadId,
    request_id: turn.requestId,
    ok: !error,
    error,
    latency_ms: latencyMs,
    ledger_elapsed_ms: ledger && ledger.telemetry ? ledger.telemetry.elapsedMs : null,
    tool_calls: turn.toolCalls,
    tool_call_count:
      ledger && ledger.telemetry && ledger.telemetry.toolCount != null
        ? ledger.telemetry.toolCount
        : turn.toolCalls.length,
    approvals_requested: approvals ? approvals.countSince(approvalsMark) : 0,
    reply_chars: (turn.reply || "").length,
    reply: (turn.reply || "").slice(0, 4000),
    files_written: written.map((f) => ({ rel: f.rel, bytes: f.bytes })),
    usage,
    grade,
    sandbox,
  };
}

// ---------------------------------------------------------------------------
// reporting
// ---------------------------------------------------------------------------

const fmtUsd = (n) => `$${n.toFixed(4)}`;
const fmtTok = (n) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));

function printReport(results) {
  console.log("");
  console.table(
    results.map((r) => ({
      scenario: r.scenario,
      done: `${r.grade.passed}/${r.grade.total}`,
      pct: `${Math.round(r.grade.score * 100)}%`,
      tools: r.tool_call_count,
      appr: r.approvals_requested,
      in: fmtTok(r.usage.input_tokens),
      cached: `${r.usage.cache_hit_pct.toFixed(0)}%`,
      out: fmtTok(r.usage.output_tokens),
      cost: fmtUsd(r.usage.cost_usd),
      latency: `${(r.latency_ms / 1000).toFixed(1)}s`,
      status: r.ok ? "ok" : "ERROR",
    })),
  );

  const tot = results.reduce(
    (a, r) => {
      a.input += r.usage.input_tokens;
      a.cached += r.usage.cached_input_tokens;
      a.output += r.usage.output_tokens;
      a.cost += r.usage.cost_usd;
      a.latency += r.latency_ms;
      a.tools += r.tool_call_count;
      a.approvals += r.approvals_requested;
      a.passed += r.grade.passed;
      a.total += r.grade.total;
      return a;
    },
    { input: 0, cached: 0, output: 0, cost: 0, latency: 0, tools: 0, approvals: 0, passed: 0, total: 0 },
  );
  console.log(
    `TOTAL  completion ${tot.passed}/${tot.total} (${Math.round(
      (tot.passed / Math.max(1, tot.total)) * 100,
    )}%)  tools ${tot.tools}  approvals ${tot.approvals}  in ${fmtTok(
      tot.input,
    )}  cached ${((tot.cached / Math.max(1, tot.input)) * 100).toFixed(
      1,
    )}%  out ${fmtTok(tot.output)}  cost ${fmtUsd(tot.cost)}  wall ${(
      tot.latency / 1000
    ).toFixed(1)}s`,
  );

  for (const r of results) {
    const failed = r.grade.checks.filter((c) => !c.ok);
    if (!failed.length && r.ok) continue;
    console.log(`\n  ${r.scenario}:`);
    if (r.error) console.log(`    ! turn error: ${r.error}`);
    for (const f of failed)
      console.log(`    x ${f.id}${f.detail ? ` — ${f.detail}` : ""}`);
  }
  console.log("");
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

async function main() {
  const opts = parseArgs(process.argv.slice(2));

  if (opts.gradeOnly) {
    const results = [];
    for (const s of SCENARIOS) {
      const sandbox = path.join(opts.gradeOnly, "sandbox", s.id);
      if (!fs.existsSync(sandbox)) continue;
      results.push({
        scenario: s.id,
        ok: true,
        error: null,
        latency_ms: 0,
        tool_call_count: 0,
        approvals_requested: 0,
        usage: foldUsage([]),
        grade: gradeScenario(s, sandbox, ""),
      });
    }
    printReport(results);
    return;
  }

  const selected = opts.only.length ? opts.only.map(scenarioById) : SCENARIOS;
  const runId = new Date().toISOString().replace(/[:.]/g, "-");
  const runDir = path.join(opts.runRoot, runId);
  const actionRoot = path.join(runDir, "sandbox");
  await fsp.mkdir(actionRoot, { recursive: true });

  if (!fs.existsSync(opts.coreBin))
    throw new Error(
      `core binary not found at ${opts.coreBin}\n` +
        `build it: cargo build --manifest-path Cargo.toml -p openhuman-cli --bin openhuman-core`,
    );
  if (!opts.managed && !opts.apiKey)
    throw new Error(
      "no inference key: set OPENROUTER_API_KEY, pass --api-key, or use --managed",
    );

  console.log(`run dir : ${runDir}`);
  console.log(`driver  : ${opts.driver}${opts.agentId ? ` agent=${opts.agentId}` : " agent=orchestrator"}`);

  let search = null;
  if (opts.mockSearch) {
    search = await startMockSearch({
      indexPath: DEFAULT_INDEX_PATH,
      requestsPath: path.join(runDir, "search-requests.json"),
      port: opts.searchPort,
    });
    console.log(
      `search  : mock at ${search.url} (${search.ctx.documents.length} documents)`,
    );
  }

  const home = await prepareHome(runDir, opts, {
    searchBase: search ? search.url : "",
  });

  let composio = null;
  if (opts.mockComposio) {
    composio = await startMockComposio({
      fixtureRoot: FIXTURES,
      outboxPath: path.join(runDir, "composio-outbox.json"),
      port: opts.composioPort,
    });
    console.log(
      `composio: mock at ${composio.url} (${composio.ctx.mailbox.length} messages, ${composio.ctx.calendar.length} events)`,
    );
  }

  const core = new Core(opts);
  const health = await core.start({
    actionDir: actionRoot,
    logPath: path.join(runDir, "core.log"),
    home,
    composioBase: composio ? composio.url : "",
    searchBase: search ? search.url : "",
    approvals: opts.approvals,
  });
  console.log(`core    : ${core.url} (pid ${health.pid}, healthy=${health.healthy})`);

  // Stand in for login.
  const localUserId = "life-scenarios-local";
  await core.rpc("openhuman.auth_set_credential", {
    token: mintLocalSessionToken(localUserId),
    kind: "local",
    userId: localUserId,
    user: {
      _id: localUserId,
      email: "life-scenarios@local.invalid",
      name: "Life Scenarios",
    },
  });

  // Point inference at OpenRouter the way the Settings → Models screen does.
  // `channel_web_chat` carries no per-turn route, so on the desktop path this
  // is the only way to run BYOK — which is also how a BYOK desktop user runs.
  if (!opts.managed) {
    // The three documented BYOK fields, and nothing else. Until
    // `complete_byok_route` (crates/openhuman-core/src/config/ops/model.rs)
    // this was accepted and then every turn died `BYOK_INCOMPLETE`, because
    // role resolution goes through `cloud_providers` and nothing had put the
    // endpoint there; the caller had to hand-build a provider entry and pin
    // four roles. That this short form now routes is the end-to-end check on
    // that fix.
    //
    // Written in a loop, and read back, because of a startup race: the write
    // lands in whichever config is active *now*, and `auth_set_credential`
    // above activates a per-user dir (`users/<id>/config.toml`) a moment
    // later, whose config then takes precedence and carries no BYOK route.
    // Lose that race and every scenario dies in under a second with
    // `provider=openhuman ... 401 Invalid token` — which reads like a broken
    // harness and is really a config that arrived too early. Observed doing
    // exactly that: one run green, the next 0/9 on the same binary.
    await withRetries(
      async () => {
        await core.rpc("openhuman.config_update_model_settings", {
          inference_url: opts.inferenceUrl,
          api_key: opts.apiKey,
          default_model: opts.model,
        });
        // `config.get` wraps the config under `config` (see
        // `snapshot_config_json`), and the RPC envelope may wrap that again.
        const snap = await core.rpc("openhuman.config_get", {});
        const cfg = snap?.config ?? snap?.snapshot?.config ?? snap?.snapshot ?? snap ?? {};
        const providers = cfg.cloud_providers ?? [];
        const routed =
          cfg.inference_url === opts.inferenceUrl &&
          providers.some((p) => p?.endpoint === opts.inferenceUrl);
        if (!routed)
          throw new Error(
            `BYOK route not in the active config yet (inference_url=${cfg.inference_url ?? "unset"}, ` +
              `${providers.length} cloud_providers)`,
          );
      },
      { attempts: 10, delayMs: 500, what: "BYOK route" },
    );
  }

  // The web-chat driver has no per-call `agent_id`, so the agent is chosen by
  // `[agent] chat_agent_id`. Set it through the running core rather than by
  // pre-writing the file, for exactly the reason the BYOK block above gives:
  // `prepareHome` writes `users/local/config.toml`, but the active user dir is
  // minted at boot (`users/local-dragonfly/...`) and its config wins. The
  // pre-written value is read by nothing, and the turn silently runs the
  // orchestrator at its own 15-iteration cap — which looks like the benchmark
  // agent failing when it never ran at all.
  const chatAgentId = opts.agentId.trim() || null;
  await withRetries(
    async () => {
      await core.rpc("openhuman.config_update_agent_settings", {
        chat_agent_id: opts.agentId,
      });
      const snap = await core.rpc("openhuman.config_get", {});
      const cfg = snap?.config ?? snap?.snapshot?.config ?? snap?.snapshot ?? snap ?? {};
      const got = cfg.agent?.chat_agent_id ?? null;
      if (got !== chatAgentId)
        throw new Error(
          `chat_agent_id not in the active config yet (want ${chatAgentId ?? "unset"}, got ${got ?? "unset"})`,
        );
    },
    { attempts: 10, delayMs: 500, what: "chat_agent_id" },
  );

  console.log(
    `route   : ${opts.managed ? "managed backend" : opts.inferenceUrl} model=${opts.model}`,
  );

  const clientId = `life-scenarios-${runId.slice(0, 12)}`;
  const events = new EventStream(core, clientId);
  if (opts.driver === "desktop") await events.connect();

  const approvals = opts.approvals ? new ApprovalResponder(core) : null;
  approvals?.start();
  console.log(
    `approve : ${opts.approvals ? "gate ON, headless responder approving" : "gate DISABLED"}`,
  );

  const results = [];
  try {
    for (let attempt = 1; attempt <= opts.repeat; attempt += 1) {
      for (const scenario of selected) {
        process.stdout.write(`running ${scenario.id} (attempt ${attempt}) ... `);
        const r = await runScenario({
          core,
          events,
          clientId,
          approvals,
          scenario,
          runDir,
          opts,
          attempt,
        });
        results.push(r);
        console.log(
          `${r.ok ? "ok" : "ERROR"} ${(r.latency_ms / 1000).toFixed(1)}s ` +
            `${r.grade.passed}/${r.grade.total} tools=${r.tool_call_count} ${fmtUsd(
              r.usage.cost_usd,
            )}`,
        );
        await fsp.writeFile(
          path.join(runDir, "results.json"),
          JSON.stringify(results, null, 2),
        );
      }
    }
  } finally {
    await approvals?.stop();
    events.close();
    await core.stop();
    if (approvals)
      await fsp.writeFile(
        path.join(runDir, "approvals.json"),
        JSON.stringify({ decisions: approvals.decisions, errors: approvals.errors }, null, 2),
      );
    if (composio) {
      await fsp.writeFile(
        path.join(runDir, "composio-requests.json"),
        JSON.stringify(
          { requests: composio.ctx.requests, outbox: composio.ctx.outbox },
          null,
          2,
        ),
      );
      await composio.close();
    }
    // `close` flushes the search log itself, so the record survives a run that
    // failed partway: it is the only evidence of what discovery returned, and
    // a post-mortem needs it most on the runs that went wrong.
    if (search) await search.close();
  }

  printReport(results);
  console.log(`results : ${path.join(runDir, "results.json")}`);
  console.log(`core log: ${path.join(runDir, "core.log")}`);
}

main().catch((e) => {
  console.error(`\nfatal: ${e.stack || e.message}`);
  process.exit(1);
});
