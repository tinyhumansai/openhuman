#!/usr/bin/env node
// Memory scenarios: realistic memory flows against a real headless core, on a
// throwaway local CortexDB and on Built-in (the user's real account), checked
// by the script itself. Every failed check is a finding for the user; nothing
// is fixed during a run. See README.md.

import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  Core,
  EventStream,
  JsonlLog,
  Findings,
  Ledger,
  sendTurn,
  waitFor,
  withRetries,
  pick,
  randomUUID,
  sleep,
  scrub,
} from "./lib.mjs";
import {
  startLocalCortex,
  startCortexWireLog,
  OLLAMA_URL,
  OLLAMA_CHAT_MODEL,
  activeWorkspace,
  placeMigrationGuard,
  verifyBuiltinUntouched,
  forgetLedger,
  builtinCredential,
} from "./engines.mjs";
import { SCENARIOS, registerLocalOnly } from "./scenarios.mjs";
import { migration } from "./migration.mjs";

registerLocalOnly(migration);

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..");
const FIXTURES = path.join(HERE, "fixtures");
// Tools a builtin turn may call: memory and the harness's own bookkeeping.
// Anything else (shell, web, connectors, sub-agents) reaches past the test.
const BUILTIN_TURN_TOOLS = new Set([
  "memory",
  "resolve_time",
  "todo",
  "goal_complete",
  "tool_search",
  "juice_retrieve",
  "juice_find",
  "juice_extract",
  "juice_summarize",
]);

function parseArgs(argv) {
  const opts = {
    only: [],
    engine: "both",
    keep: false,
    gradeOnly: "",
    cleanup: "",
    coreBin: path.join(REPO, "target", "debug", "openhuman-core"),
    runRoot: path.join(REPO, "target", "memory-scenarios"),
    settleMs: 4000,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => argv[++i];
    if (a === "--only") opts.only.push(...next().split(","));
    else if (a === "--engine") opts.engine = next();
    else if (a === "--keep") opts.keep = true;
    else if (a === "--grade-only") opts.gradeOnly = next();
    else if (a === "--cleanup") {
      opts.cleanup = next();
      opts.engine = "builtin";
    } else if (a === "--core-bin") opts.coreBin = next();
    else if (a === "--settle-ms") opts.settleMs = Number(next());
    else if (a === "--help" || a === "-h") {
      console.log(
        fs
          .readFileSync(path.join(HERE, "README.md"), "utf8")
          .split("\n")
          .slice(0, 30)
          .join("\n"),
      );
      process.exit(0);
    } else throw new Error(`unknown argument ${a}`);
  }
  if (!["local", "builtin", "both"].includes(opts.engine))
    throw new Error("--engine local|builtin|both");
  return opts;
}

/** The offline local session token (as life-scenarios mints it). */
function mintLocalSessionToken(userId) {
  const b64 = (o) => Buffer.from(JSON.stringify(o)).toString("base64url");
  const now = Math.floor(Date.now() / 1000);
  return [
    b64({ alg: "none", typ: "JWT" }),
    b64({
      sub: userId,
      iat: now,
      exp: now + 86_400,
      email: "memscen@local.invalid",
    }),
    "local",
  ].join(".");
}

/** Set `key = value` lines in a TOML section of `file` (a small, line-based edit). */
async function editToml(file, section, kv) {
  let text = await fsp.readFile(file, "utf8").catch(() => "");
  const lines = text.split("\n");
  const header = `[${section}]`;
  let start = lines.findIndex((l) => l.trim() === header);
  if (start < 0) {
    lines.push("", header);
    start = lines.length - 1;
  }
  let end = lines.findIndex((l, i) => i > start && /^\s*\[/.test(l));
  if (end < 0) end = lines.length;
  for (const [k, v] of Object.entries(kv)) {
    const line = `${k} = ${typeof v === "string" ? JSON.stringify(v) : v}`;
    const at = lines
      .slice(start + 1, end)
      .findIndex(
        (l) => l.trim().startsWith(`${k} =`) || l.trim().startsWith(`${k}=`),
      );
    if (at >= 0) lines[start + 1 + at] = line;
    else {
      lines.splice(end, 0, line);
      end += 1;
    }
  }
  await fsp.writeFile(file, lines.join("\n"));
}

const BASE_CONFIG = (extra = []) =>
  [
    "schema_version = 13",
    "onboarding_completed = true",
    "chat_onboarding_completed = true",
    "",
    "[observability]",
    "analytics_enabled = false",
    "share_usage_data = false",
    "",
    ...extra,
  ].join("\n");

// ---------------------------------------------------------------------------
// one engine
// ---------------------------------------------------------------------------

/** Teardowns to run if the process is interrupted (a container must not outlive the run). */
const CLEANUPS = new Set();
for (const sig of ["SIGINT", "SIGTERM"])
  process.once(sig, async () => {
    for (const fn of CLEANUPS) await fn().catch(() => {});
    process.exit(130);
  });

async function runEngine(engine, { opts, runDir, runId, findings, results }) {
  const dir = path.join(runDir, engine);
  let home = path.join(dir, "home");
  const oh = path.join(home, ".openhuman");
  await fsp.mkdir(oh, { recursive: true });
  const prior = opts.cleanup
    ? JSON.parse(
        await fsp.readFile(
          path.join(opts.cleanup, "builtin", "cleanup.json"),
          "utf8",
        ),
      )
    : null;
  const marker = prior?.marker ?? `memscen:${runId}`;
  const secrets = [];
  const extraEnv = {};
  const engineResults = { engine, marker, scenarios: {} };
  results.engines[engine] = engineResults;
  const log = (m) => console.log(m);

  let cortex = null;
  let wire = null;
  let guardPlaced = false;
  let builtinSubject = null;
  let core = null;
  let events = null;
  const ledger = new Ledger();
  const threads = [...(prior?.threads ?? [])];
  ledger.addAll(prior?.ids);
  const strayTools = [];

  // --- engine setup -------------------------------------------------------
  if (engine === "local") {
    cortex = await startLocalCortex({
      runId,
      scriptDir: HERE,
      tinymemoryDir: path.join(REPO, "vendor", "tinymemory"),
      keep: opts.keep,
      log,
    });
    secrets.push(cortex.apiKey);
    // The core talks to CortexDB through a logging pass-through, so checks
    // can read what was sent (attribution is wire-only).
    wire = await startCortexWireLog({
      target: cortex.endpoint,
      logFile: path.join(dir, "cortex-wire.jsonl"),
      secrets,
    });
    const stopCortex = () => cortex.stop();
    CLEANUPS.add(stopCortex);
    engineResults.inference = cortex.inference;
    // Chat runs on the account's managed route (user decision); memory stays on
    // the container. The key reaches the core only through the environment.
    try {
      const cred = await builtinCredential({ only: "key" });
      secrets.push(cred.secret);
      Object.assign(extraEnv, cred.env);
    } catch (e) {
      log(
        `local   : no account key (${e.message}); chat will fall back to Ollama`,
      );
    }
    await fsp.writeFile(
      path.join(oh, "config.toml"),
      BASE_CONFIG([
        "[scheduler_gate]",
        'mode = "off"',
        "",
      ]),
    );
  } else {
    const cred = await builtinCredential();
    secrets.push(cred.secret);
    Object.assign(extraEnv, cred.env);
    // Guard 0: the scheduler gate is off before the core first boots, so no
    // background job (the migration's tick, import resume, belief builds)
    // runs on its own. An API key is seeded as a plain profile write with no
    // user-dir activation (boot_env.rs), so the root config is the active
    // one. Verified after boot; the run aborts if it is not in effect.
    // Guard 0 goes in every config the core may treat as active: the root,
    // the pre-login user dir (a key activates none) and, for a session, the
    // account's own user dir (`users/<subject>`). Verified after boot.
    // Guard 2: Composio is off, so no turn can reach the account's connected
    // mail, calendar or repos (a memory test must never read real data).
    const gated = BASE_CONFIG([
      "[scheduler_gate]",
      'mode = "off"',
      "",
      "[composio]",
      'mode = "disabled"',
      "",
    ]);
    await fsp.writeFile(path.join(oh, "config.toml"), gated);
    for (const id of ["local", ...(cred.subject ? [cred.subject] : [])]) {
      await fsp.mkdir(path.join(oh, "users", id), { recursive: true });
      await fsp.writeFile(path.join(oh, "users", id, "config.toml"), gated);
    }
    builtinSubject = cred.subject;
    engineResults.credential = cred.kind;
  }

  const rpcLog = new JsonlLog(path.join(dir, "rpc.jsonl"), secrets);
  const logFile = path.join(dir, "core.log");
  const startCore = async () => {
    core = new Core({ coreBin: opts.coreBin, rpcLog, secrets });
    const health = await core.start({
      home,
      actionDir: path.join(dir, "action"),
      logPath: logFile,
      extraEnv,
    });
    events = new EventStream(core, `memscen-${runId.slice(-8)}`);
    await events.connect();
    return health;
  };

  /**
   * Set up a local-engine core: a local session, memory on
   * this run's container (abort otherwise) and a chat route. With the account
   * key (`withKey`), the scheduler gate is forced off and chat is managed.
   * Without it (a fresh HOME that never saw the key), the gate keeps its
   * default and chat runs on the local Ollama.
   */
  const setupLocal = async ({ withKey }) => {
    // A local session: E needs a per-user root for layout v3.
    const uid = "memscen-local";
    await core.rpc("openhuman.auth_set_credential", {
      token: mintLocalSessionToken(uid),
      kind: "local",
      userId: uid,
      user: { _id: uid, email: "memscen@local.invalid", name: "Jordan Lee" },
    });
    // The session activates a user dir whose config wins: put the
    // scheduler-gate guard there, restart, read back.
    // With the account key present, no background job may run on its own.
    const { config_path: cfgPath } = await core.rpc("openhuman.config_get", {});
    if (withKey) await editToml(cfgPath, "scheduler_gate", { mode: "off" });
    await core.stop();
    await startCore();
    const gate = pick(
      await core.rpc("openhuman.config_get", {}),
      "config.scheduler_gate.mode",
    );
    if (withKey && gate !== "off")
      throw new Error(`ABORT local: scheduler gate is "${gate}", not "off"`);

    // Memory stays on this run's CortexDB container; abort otherwise.
    await withRetries(
      async () => {
        await core.rpc("openhuman.memory_engine_set", {
          engine: "cortexdb",
          endpoint: wire.url,
          api_key: cortex.apiKey,
        });
        const got = await core.rpc("openhuman.memory_engine_get", {});
        if (
          got?.engine !== "cortexdb" ||
          got?.status !== "ok" ||
          got?.endpoint !== wire.url
        )
          throw new Error(
            `engine ${got?.engine} ${got?.status} at ${got?.endpoint} ${got?.reason ?? ""}`,
          );
      },
      { attempts: 20, delayMs: 1000, what: "local CortexDB engine" },
    ).catch((e) => {
      throw new Error(
        `ABORT local: memory is not on the local CortexDB (${e.message})`,
      );
    });

    // Chat: the account's managed route (the key is in the env). If it
    // cannot run here, fall back to the local Ollama and mark the checks
    // that depend on reply quality INCONCLUSIVE instead of failed.
    const probe = !withKey
      ? { error: "no account key on this core (by design)" }
      : await sendTurn({
          core,
          events,
          clientId: events.clientId,
          threadId: `thread-${randomUUID()}`,
          message: "Reply with the single word OK.",
          timeoutMs: 120_000,
        });
    if (!probe.error) {
      engineResults.inference.chat_route = "managed (account API key)";
    } else {
      engineResults.inference.chat_route_managed_error = scrub(
        probe.error,
        secrets,
      );
      const inferenceUrl = `${OLLAMA_URL}/v1`;
      await withRetries(
        async () => {
          await core.rpc("openhuman.config_update_model_settings", {
            inference_url: inferenceUrl,
            api_key: "ollama",
            default_model: OLLAMA_CHAT_MODEL,
          });
          const cfg =
            (await core.rpc("openhuman.config_get", {}))?.config ?? {};
          if (
            cfg.inference_url !== inferenceUrl ||
            !(cfg.cloud_providers ?? []).some(
              (x) => x?.endpoint === inferenceUrl,
            )
          )
            throw new Error(
              `BYOK route not active yet (inference_url=${cfg.inference_url ?? "unset"})`,
            );
        },
        { attempts: 10, delayMs: 500, what: "local chat route (Ollama)" },
      );
      engineResults.inference.chat_route = `BYOK ${inferenceUrl} ${OLLAMA_CHAT_MODEL} (fallback)`;
      if (withKey) engineResults.qualityInconclusive = true;
    }
    // The memory engine must still be the local container after all that.
    const final = await core.rpc("openhuman.memory_engine_get", {});
    if (final?.engine !== "cortexdb" || final?.endpoint !== wire.url)
      throw new Error(
        `ABORT local: memory engine moved to ${final?.engine} at ${final?.endpoint}`,
      );
    log(
      `local   : memory on ${cortex.endpoint} via ${wire.url}, scheduler gate ${gate}, chat ${engineResults.inference.chat_route}`,
    );
  };

  try {
    const health = await startCore();
    log(`${engine.padEnd(8)}: core ${core.url} pid ${health.pid}`);

    if (engine === "local") {
      await setupLocal({ withKey: true });
    } else {
      // Guard 0 verified: the gate the account's config carries is off.
      // A session activates the account's user dir asynchronously at boot:
      // wait for it, then read the guard back from THAT config.
      let snap = await core.rpc("openhuman.config_get", {});
      if (builtinSubject) {
        const want = path.join("users", builtinSubject, "config.toml");
        snap = await waitFor(
          async () => {
            const s2 = await core.rpc("openhuman.config_get", {});
            return String(s2?.config_path ?? "").endsWith(want) ? s2 : null;
          },
          {
            timeoutMs: 60_000,
            intervalMs: 500,
            what: "the account's user dir active",
          },
        ).catch(() => {
          throw new Error(
            "ABORT builtin: the session did not activate the account's user dir",
          );
        });
      }
      const mode = pick(snap, "config.scheduler_gate.mode");
      if (mode !== "off")
        throw new Error(
          `ABORT builtin: scheduler gate is "${mode}", not "off" (user dir not the one pre-written)`,
        );
      const composioMode = pick(snap, "config.composio.mode");
      if (composioMode !== "disabled")
        throw new Error(
          `ABORT builtin: Composio is "${composioMode}", not "disabled"`,
        );
      const eng = await core.rpc("openhuman.memory_engine_get", {});
      if (eng?.engine !== "tinyhumans" || eng?.status === "off")
        throw new Error(
          `ABORT builtin: memory engine is ${eng?.engine}/${eng?.status} (${eng?.reason ?? ""}); is the session valid?`,
        );
      // Guard 1: the migration of this workspace counts as done.
      const { workspace } = await activeWorkspace(core);
      await placeMigrationGuard(
        core,
        path.join(workspace, "memory", "layout_migration.json"),
      );
      guardPlaced = true;
      const imp = await core.rpc("openhuman.memory_import_scan", {});
      if (imp?.found)
        throw new Error(
          "ABORT builtin: a v1 store is present in the throwaway workspace",
        );
      engineResults.inference = { kind: "managed (account route)" };
      log(
        `builtin : guards in place (scheduler gate off, Composio off, migration state cleaned, no v1 store)`,
      );
    }

    // --- scenarios ----------------------------------------------------------
    const selected = SCENARIOS.filter((s) => s.engines.includes(engine))
      .filter(
        (s) =>
          !opts.only.length ||
          opts.only.some((o) => s.id.toLowerCase().startsWith(o.toLowerCase())),
      )
      .sort((a, b) => Number(!!a.last) - Number(!!b.last));

    for (const scenario of opts.cleanup ? [] : selected) {
      if (strayTools.length) break;
      const transcript = new JsonlLog(
        path.join(dir, "scenarios", scenario.id, "transcript.jsonl"),
        secrets,
      );
      const sres = { checks: 0, failed: 0, notes: 0, error: null, results: {} };
      engineResults.scenarios[scenario.id] = sres;
      const ctx = {
        engine,
        persona: JSON.parse(
          await fsp.readFile(path.join(FIXTURES, "persona.json"), "utf8"),
        ),
        fixtureCopy: path.join(dir, "action", "fixtures"),
        marker,
        ledger,
        threads,
        logFile,
        settleMs: opts.settleMs,
        recallQualityChecked:
          engine === "builtin" ||
          !!engineResults.inference?.recall_quality_checked,
        cortex,
        wire,
        results: sres.results,
        get core() {
          return core;
        },
        rpc: (m, p, t) => core.rpc(m, p, t),
        tryRpc: (m, p, t) => core.tryRpc(m, p, t),
        markMeta: (extra = {}) => ({ tags: [marker], ...extra }),
        newThread(label) {
          const id = `thread-${randomUUID()}`;
          threads.push(id);
          transcript.write({ thread: id, label });
          return id;
        },
        async turn(threadId, message) {
          const r = await sendTurn({
            core,
            events,
            clientId: events.clientId,
            threadId,
            message,
          });
          await transcript.write({
            thread: threadId,
            user: message,
            reply: r.reply,
            error: r.error,
            ms: r.ms,
            tools: r.toolCalls,
          });
          // Guard 3 (builtin): a turn may only use memory-side tools. There is
          // no config switch for the shell tool, so this detects rather than
          // prevents: the first stray call stops every remaining scenario.
          const stray =
            engine === "builtin"
              ? (r.toolCalls ?? []).filter((n) => !BUILTIN_TURN_TOOLS.has(n))
              : [];
          if (stray.length) {
            strayTools.push(...stray);
            throw new Error(
              `ABORT builtin: a turn called ${[...new Set(stray)].join(", ")} on the real account`,
            );
          }
          return r;
        },
        async learn(text, kind = "fact", meta = {}) {
          const r = await core.rpc("openhuman.memory_learn", {
            text,
            kind,
            meta: { tags: [marker], ...meta },
          });
          ledger.add(r?.id);
          return r?.id;
        },
        async listAll(filter = {}, max = 2000) {
          const out = [];
          let cursor;
          do {
            const page = await core.rpc("openhuman.memory_items_list", {
              filter,
              limit: 100,
              ...(cursor ? { cursor } : {}),
            });
            out.push(...(page?.items ?? []));
            cursor = page?.next_cursor;
          } while (cursor && out.length < max);
          return out;
        },
        async waitItems(filter, atLeast, timeoutMs) {
          return waitFor(
            async () => {
              const items = await ctx.listAll(filter);
              return items.length >= atLeast ? items : null;
            },
            {
              timeoutMs,
              intervalMs: 1500,
              what: `${atLeast} item(s) for ${JSON.stringify(filter)}`,
            },
          ).catch(() => ctx.listAll(filter));
        },
        async setChatAgent(id) {
          await withRetries(
            async () => {
              await core.rpc("openhuman.config_update_agent_settings", {
                chat_agent_id: id,
              });
              const snap = await core.rpc("openhuman.config_get", {});
              if ((pick(snap, "config.agent.chat_agent_id") ?? "") !== id)
                throw new Error("chat_agent_id not applied");
            },
            { what: "chat_agent_id" },
          );
        },
        activeWorkspace: () => activeWorkspace(core),
        async setConfigToml(section, kv) {
          if (engine !== "local")
            throw new Error("config edits are local-engine only");
          const snap = await core.rpc("openhuman.config_get", {});
          await editToml(snap.config_path, section, kv);
        },
        async restartCore() {
          events?.close();
          await core.stop();
          await startCore();
        },
        check(id, ok, expected, actual, severity = "medium", basis = "READ") {
          sres.checks += 1;
          if (!ok) sres.failed += 1;
          return findings.check({
            engine,
            scenario: scenario.id,
            id,
            ok,
            expected,
            actual,
            evidence: path.relative(
              runDir,
              path.join(dir, "scenarios", scenario.id),
            ),
            severity,
            basis,
          });
        },
        /** A check that depends on reply quality: INCONCLUSIVE on a fallback model. */
        qcheck(
          id,
          ok,
          expected,
          actual,
          severity = "medium",
          basis = "INFERRED",
        ) {
          if (engineResults.qualityInconclusive && !ok) {
            sres.checks += 1;
            sres.inconclusive = (sres.inconclusive ?? 0) + 1;
            findings.note({
              engine,
              scenario: scenario.id,
              id: `${id}-INCONCLUSIVE`,
              text: `INCONCLUSIVE on the fallback chat model: expected ${expected}; got ${typeof actual === "string" ? actual.slice(0, 200) : JSON.stringify(actual).slice(0, 200)}`,
              evidence: path.relative(
                runDir,
                path.join(dir, "scenarios", scenario.id),
              ),
              basis,
            });
            return false;
          }
          return ctx.check(id, ok, expected, actual, severity, basis);
        },
        note(id, text, basis = "READ") {
          sres.notes += 1;
          findings.note({
            engine,
            scenario: scenario.id,
            id,
            text,
            evidence: path.relative(
              runDir,
              path.join(dir, "scenarios", scenario.id),
            ),
            basis,
          });
        },
      };
      await fsp.cp(FIXTURES, ctx.fixtureCopy, { recursive: true });
      if (scenario.noAccount && engine === "local") {
        // A core that never saw the account key, so its background work
        // (which a user-started move is gated on) may run without any
        // chance of touching the hosted account.
        events?.close();
        await core.stop();
        delete extraEnv.OPENHUMAN_BACKEND_API_KEY;
        home = path.join(dir, `home-${scenario.id}`);
        await fsp.mkdir(path.join(home, ".openhuman"), { recursive: true });
        await fsp.writeFile(
          path.join(home, ".openhuman", "config.toml"),
          BASE_CONFIG(),
        );
        await startCore();
        await setupLocal({ withKey: false });
        ctx.note(
          "isolated-core",
          `ran on a fresh core without the account key (gate default, chat ${engineResults.inference.chat_route})`,
        );
      }
      process.stdout.write(`${engine.padEnd(8)}: ${scenario.id} ... `);
      const started = Date.now();
      try {
        await scenario.run(ctx);
      } catch (e) {
        sres.error = scrub(e.stack || e.message, secrets);
        findings.add({
          engine,
          scenario: scenario.id,
          id: "crashed",
          severity: "high",
          expected: "the scenario runs to the end",
          actual: sres.error.slice(0, 600),
          evidence: "core.log",
          basis: "READ",
        });
      }
      sres.ms = Date.now() - started;
      console.log(
        `${sres.error ? "CRASHED" : "done"} ${sres.checks - sres.failed}/${sres.checks} checks, ${sres.notes} notes (${(sres.ms / 1000).toFixed(0)}s)`,
      );
    }
  } finally {
    // --- teardown -----------------------------------------------------------
    if (engine === "builtin" && core && !core.exited) {
      try {
        // Positive control, before the real clean-up: an empty answer proves
        // nothing unless this engine, right now, can show an item, forget it
        // and show it gone. A degraded backend answering "empty" fails here.
        // The probe carries the run's tag, so a retry finds it if this fails.
        const tagged = async () =>
          (
            await core.rpc("openhuman.memory_items_list", {
              filter: { tags_any: [marker] },
              limit: 100,
            })
          )?.items ?? [];
        const probe = await core.rpc("openhuman.memory_learn", {
          text: `memscen clean-up probe ${runId}`,
          kind: "other",
          meta: { tags: [marker] },
        });
        const control = (what, ok) =>
          waitFor(async () => (await ok()) || null, {
            timeoutMs: 60_000,
            intervalMs: 2000,
            what,
          }).catch(() => {
            throw new Error(
              `positive control failed: ${what}; an empty answer is not evidence`,
            );
          });
        await control("the probe is listed", async () =>
          (await tagged()).some((h) => h.id === probe?.id),
        );
        const gone = await core.rpc("openhuman.memory_forget", {
          ids: [probe?.id],
        });
        if (gone?.forgotten !== 1)
          throw new Error(
            `positive control failed: forgetting the probe counted ${gone?.forgotten}`,
          );
        await control("the probe is gone", async () => {
          const got = await core.rpc("openhuman.memory_items_get", {
            ids: [probe?.id],
          });
          return (
            !(got?.items ?? []).length &&
            !(await tagged()).some((h) => h.id === probe?.id)
          );
        });
        // Everything this run stored: the ledger, every item in its threads,
        // every item carrying its marker. Written down first, so a failed
        // clean-up can be retried with --cleanup <run-dir>.
        const unlisted = [];
        for (const filter of [
          ...threads.map((t) => ({ thread_id: t })),
          { tags_any: [marker] },
        ]) {
          const r = await core.tryRpc("openhuman.memory_items_list", {
            filter,
            limit: 100,
          });
          if (r.ok) ledger.addAll((r.value?.items ?? []).map((h) => h.id));
          else unlisted.push(JSON.stringify(filter));
        }
        await fsp.writeFile(
          path.join(dir, "cleanup.json"),
          JSON.stringify({ marker, threads, ids: ledger.list() }, null, 2),
        );
        // Fails closed: an error forgetting or reading back throws.
        const survivors = await forgetLedger(core, ledger);
        if (unlisted.length)
          throw new Error(
            `could not list ${unlisted.join(", ")}; items there may survive`,
          );
        // And nothing the run's threads or tag hold is left either.
        const filters = [
          ...threads.map((t) => ({ thread_id: t })),
          { tags_any: [marker] },
        ];
        for (const filter of filters) {
          const left = await core.rpc("openhuman.memory_items_list", {
            filter,
            limit: 100,
          });
          for (const h of left?.items ?? []) survivors.push(h.id);
        }
        engineResults.cleanupListsEmpty = filters.length;
        const untouched = guardPlaced
          ? await verifyBuiltinUntouched(core)
          : {
              ok: true,
              note: "aborted before the migration guard; no scenario ran",
            };
        engineResults.cleanup = {
          stored: ledger.list().length,
          survivors,
          untouched,
        };
        if (survivors.length)
          findings.add({
            engine,
            scenario: "teardown",
            id: "survivors",
            severity: "high",
            expected: "every item this run stored is forgotten",
            actual: survivors,
            evidence: "rpc.jsonl",
            basis: "READ",
          });
        if (strayTools.length)
          findings.add({
            engine,
            scenario: "guard",
            id: "stray-tools",
            severity: "high",
            expected: "builtin turns use memory-side tools only",
            actual: [...new Set(strayTools)],
            evidence: "scenarios/*/transcript.jsonl",
            basis: "READ",
          });
        if (!untouched.ok)
          findings.add({
            engine,
            scenario: "teardown",
            id: "account-touched",
            severity: "high",
            expected: "no migration or import ran on the real account",
            actual: untouched,
            evidence: "rpc.jsonl",
            basis: "READ",
          });
        console.log(
          `builtin : cleanup forgot ${ledger.list().length} ids, ${survivors.length} survived (${engineResults.cleanupListsEmpty} thread/tag lists re-read; probe control passed first); migration/import untouched: ${untouched.ok}`,
        );
      } catch (e) {
        findings.add({
          engine,
          scenario: "teardown",
          id: "cleanup-failed",
          severity: "high",
          expected: "cleanup completes",
          actual: scrub(e.message, secrets),
          evidence: "rpc.jsonl",
          basis: "READ",
        });
        console.log(
          `builtin : CLEANUP FAILED (${scrub(e.message, secrets).slice(0, 200)}); retry: node scripts/memory-scenarios/run.mjs --cleanup ${runDir}`,
        );
      }
    }
    events?.close();
    await core?.stop();
    await cortex
      ?.stop()
      .catch((e) => console.error(`local   : teardown failed: ${e.message}`));
  }
}

// ---------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------

function renderReport({ runId, results, findings }) {
  const lines = [`# Memory scenarios ${runId}`, ""];
  for (const [engine, er] of Object.entries(results.engines)) {
    lines.push(`## ${engine}`, "");
    lines.push(`- inference: ${JSON.stringify(er.inference ?? {})}`);
    if (er.inference && er.inference.recall_quality_checked === false)
      lines.push(
        "- **recall-quality checks skipped**: Ollama was unreachable; the engine ran on the deterministic mock inference.",
      );
    if (er.cleanup) lines.push(`- cleanup: ${JSON.stringify(er.cleanup)}`);
    lines.push(
      "",
      "| scenario | checks passed | notes | time | crashed |",
      "| --- | --- | --- | --- | --- |",
    );
    for (const [id, s] of Object.entries(er.scenarios))
      lines.push(
        `| ${id} | ${s.checks - s.failed}/${s.checks} | ${s.notes} | ${((s.ms ?? 0) / 1000).toFixed(0)}s | ${s.error ? "yes" : ""} |`,
      );
    lines.push("");
  }
  const real = findings.filter((f) => f.severity !== "note");
  lines.push(
    `## Findings (${real.length})`,
    "",
    "| id | severity | basis | expected | actual |",
    "| --- | --- | --- | --- | --- |",
  );
  const order = { high: 0, medium: 1, low: 2 };
  for (const f of [...real].sort(
    (a, b) => (order[a.severity] ?? 3) - (order[b.severity] ?? 3),
  ))
    lines.push(
      `| ${f.id} | ${f.severity} | ${f.basis} | ${f.expected} | ${String(f.actual).replace(/\|/g, "\\|").slice(0, 300)} |`,
    );
  lines.push("", "## Notes", "");
  for (const f of findings.filter((x) => x.severity === "note"))
    lines.push(`- **${f.id}** (${f.basis}): ${f.actual}`);
  lines.push(
    "",
    "## Known gaps (not findings)",
    "",
    "- Channel messages are not logged to memory today, so the channel sender name (#7131) is inert: an open product question.",
  );
  return lines.join("\n") + "\n";
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.gradeOnly) {
    const findings = JSON.parse(
      await fsp.readFile(path.join(opts.gradeOnly, "findings.json"), "utf8"),
    );
    const results = JSON.parse(
      await fsp.readFile(path.join(opts.gradeOnly, "results.json"), "utf8"),
    );
    const md = renderReport({
      runId: path.basename(opts.gradeOnly),
      results,
      findings,
    });
    await fsp.writeFile(path.join(opts.gradeOnly, "report.md"), md);
    console.log(md);
    return;
  }
  if (!fs.existsSync(opts.coreBin))
    throw new Error(
      `core binary not found at ${opts.coreBin}\nbuild it: cargo build -p openhuman-cli --bin openhuman-core`,
    );
  const engines = opts.engine === "both" ? ["local", "builtin"] : [opts.engine];
  if (engines.includes("builtin")) await builtinCredential(); // refuse early, before anything starts
  const runId = new Date().toISOString().replace(/[:.]/g, "-");
  const runDir = path.join(opts.runRoot, runId);
  await fsp.mkdir(runDir, { recursive: true });
  console.log(`run dir : ${runDir}`);

  const findings = new Findings();
  const results = {
    run_id: runId,
    started: new Date().toISOString(),
    engines: {},
  };
  for (const engine of engines) {
    try {
      await runEngine(engine, { opts, runDir, runId, findings, results });
    } catch (e) {
      const secretSafe = scrub(e.message, []);
      console.error(`${engine}: ${secretSafe}`);
      findings.add({
        engine,
        scenario: "setup",
        id: "aborted",
        severity: "high",
        expected: "the engine sets up",
        actual: secretSafe,
        evidence: `${engine}/core.log`,
      });
    }
  }
  results.finished = new Date().toISOString();
  await fsp.writeFile(
    path.join(runDir, "findings.json"),
    JSON.stringify(findings.items, null, 2),
  );
  await fsp.writeFile(
    path.join(runDir, "checks.json"),
    JSON.stringify(findings.checks, null, 2),
  );
  await fsp.writeFile(
    path.join(runDir, "results.json"),
    JSON.stringify(results, null, 2),
  );
  const md = renderReport({ runId, results, findings: findings.items });
  await fsp.writeFile(path.join(runDir, "report.md"), md);
  console.log(
    `\nreport  : ${path.join(runDir, "report.md")}\nfindings: ${findings.items.filter((f) => f.severity !== "note").length}`,
  );
}

main()
  .then(() => process.exit(0))
  .catch((e) => {
    console.error(`\nfatal: ${scrub(e.stack || e.message, [])}`);
    process.exit(1);
  });
