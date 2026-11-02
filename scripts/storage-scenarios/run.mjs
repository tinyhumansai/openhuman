#!/usr/bin/env node
// Storage scenarios: how tinystoragedrivers works inside OpenHuman, driven
// through a real headless core over JSON-RPC, and checked on disk. Every
// failed check is a finding for a person; nothing is fixed during a run. See
// README.md.

import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { randomBytes } from "node:crypto";
import { fileURLToPath } from "node:url";
import {
  Core,
  JsonlLog,
  Findings,
  scrub,
  killHard,
  mintSessionToken,
  findFiles,
} from "./lib.mjs";
import { SCENARIOS } from "./scenarios.mjs";
import { ISOLATED } from "./isolated.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..");
const DRIVERS = ["sqlite", "file", "memory"];

// The run sets its own storage URL, workspace and master key per core; the
// operator's must never leak into one.
for (const k of [
  "OPENHUMAN_STORAGE_URL",
  "OPENHUMAN_WORKSPACE",
  "OPENHUMAN_KEYRING_MASTER_KEY",
  "OPENHUMAN_KEYRING_MASTER_KEY_FILE",
  "OPENHUMAN_KEYRING_BACKEND",
  "OPENHUMAN_BACKEND_SESSION_TOKEN",
  "OPENHUMAN_BACKEND_API_KEY",
])
  delete process.env[k];

// Credential and secret paths log their store/load hits and misses at debug;
// a secrets check that fails is explained by those lines in core.log.
process.env.RUST_LOG ||=
  "info,openhuman_core::security::credentials=debug,openhuman_core::security::secret_storage=debug,openhuman_core::storage::secrets=debug";

function parseArgs(argv) {
  const opts = {
    only: [],
    drivers: DRIVERS,
    gradeOnly: "",
    coreBin: path.join(REPO, "target", "debug", "openhuman-core"),
    runRoot: path.join(REPO, "target", "storage-scenarios"),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => argv[++i];
    if (a === "--only") opts.only.push(...next().split(",").map((s) => s.trim().toUpperCase()));
    else if (a === "--driver") opts.drivers = next().split(",");
    else if (a === "--grade-only") opts.gradeOnly = next();
    else if (a === "--core-bin") opts.coreBin = next();
    else throw new Error(`unknown argument ${a}`);
  }
  for (const d of opts.drivers)
    if (!DRIVERS.includes(d)) throw new Error(`unknown driver ${d} (${DRIVERS.join(", ")})`);
  return opts;
}

const BASE_CONFIG = [
  "schema_version = 13",
  // Every backend-bound call (notification triage, a fictional API key's
  // first use) goes to a closed loopback port: the run never reaches a
  // network or a model.
  'api_url = "http://127.0.0.1:9"',
  "onboarding_completed = true",
  "chat_onboarding_completed = true",
  "",
  "[observability]",
  "analytics_enabled = false",
  "share_usage_data = false",
  "",
  // No background job runs on its own: imported legacy cron jobs are due.
  "[scheduler_gate]",
  'mode = "off"',
  "",
  "[cron]",
  "enabled = true",
  "",
  "[composio]",
  'mode = "disabled"',
  "",
].join("\n");

/** Set `key = value` lines in a TOML section of `file` (a small, line-based edit). */
async function editToml(file, section, kv) {
  const lines = (await fsp.readFile(file, "utf8").catch(() => "")).split("\n");
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
    const at = lines.slice(start + 1, end).findIndex((l) => l.trim().startsWith(`${k} =`));
    if (at >= 0) lines[start + 1 + at] = line;
    else lines.splice(end++, 0, line);
  }
  await fsp.writeFile(file, lines.join("\n"));
}

/**
 * One core with its own HOME. Scenario code talks to it through the same
 * methods whether it is the driver's main core or an isolated one.
 */
class Rig {
  constructor({ name, dir, driver, url, masterKey = true, findings, binding, secrets, results }) {
    this.name = name;
    this.dir = dir;
    this.driver = driver;
    this.home = path.join(dir, "home");
    this.oh = path.join(this.home, ".openhuman");
    this.findings = findings;
    this.binding = binding;
    this.secrets = secrets;
    this.results = results;
    this.methods = null;
    this.core = null;
    // The auto-updater would otherwise ask api.github.com for a release: the
    // run stays offline.
    this.env = { OPENHUMAN_AUTO_UPDATE_ENABLED: "false" };
    if (masterKey) {
      const key = randomBytes(32).toString("hex");
      this.secrets.push(key);
      this.env.OPENHUMAN_KEYRING_MASTER_KEY = key;
    }
    this.storageDir = null;
    if (url) {
      const resolved = url.replace("<home>", this.home);
      this.env.OPENHUMAN_STORAGE_URL = resolved;
      if (resolved.startsWith("file:")) this.storageDir = resolved.slice(5);
    }
    this.rpcLog = new JsonlLog(path.join(dir, "rpc.jsonl"), secrets);
  }

  baseConfig() {
    return BASE_CONFIG;
  }
  editConfig(file, section, kv) {
    return editToml(file, section, kv);
  }
  addSecret(s) {
    this.secrets.push(s);
  }

  async start() {
    await fsp.mkdir(this.oh, { recursive: true });
    if (!fs.existsSync(path.join(this.oh, "config.toml")))
      await fsp.writeFile(path.join(this.oh, "config.toml"), BASE_CONFIG);
    this.core = new Core({ coreBin: this.binding.coreBin, rpcLog: this.rpcLog, secrets: this.secrets });
    await this.core.start({
      home: this.home,
      actionDir: path.join(this.dir, "action"),
      logPath: path.join(this.dir, "core.log"),
      extraEnv: this.env,
    });
    if (!this.methods) {
      const text = await fetch(`${this.core.url}/schema`).then((r) => r.text()).catch(() => "");
      this.methods = new Set(text.match(/openhuman\.[a-z0-9_]+/g) ?? []);
    }
  }
  async stop() {
    await this.core?.stop();
  }
  async restart() {
    await this.stop();
    await this.start();
  }
  async kill9() {
    await killHard(this.core);
  }
  get alive() {
    return !!this.core && !this.core.exited;
  }

  /** Whether the build serves `method` (true when /schema could not be read). */
  has(method) {
    return !this.methods?.size || this.methods.has(method);
  }
  rpc(method, params, timeoutMs) {
    return this.core.rpc(method, params, timeoutMs);
  }
  tryRpc(method, params, timeoutMs) {
    return this.core.tryRpc(method, params, timeoutMs);
  }

  /**
   * Sign `userId` in with a locally minted session (never validated by the
   * core). A brand-new profile is pre-seeded as layout v2 with the run's
   * config, so its own config keeps the scheduler gate off.
   */
  async signIn(userId) {
    const dir = path.join(this.oh, "users", userId);
    if (!fs.existsSync(dir)) {
      await fsp.mkdir(dir, { recursive: true });
      await fsp.writeFile(path.join(dir, ".layout"), "2\n");
      await fsp.writeFile(path.join(dir, "config.toml"), BASE_CONFIG);
    }
    const token = mintSessionToken(userId);
    this.addSecret(token);
    await this.rpc("openhuman.auth_set_credential", { token, kind: "session", userId });
    return token;
  }

  /** Every SQLite storage database under this HOME (backups excluded). */
  async dbFiles() {
    return (await findFiles(this.home, "storage.db")).filter((p) => !p.includes(`${path.sep}backups${path.sep}`));
  }

  check(c) {
    return this.findings.check({ engine: this.driver, scenario: this.binding.scenario, ...c });
  }
  note(id, text) {
    this.findings.note({ engine: this.driver, scenario: this.binding.scenario, id, text });
  }
  result(key, value) {
    this.results[`${this.binding.scenario}/${key}`] = value;
  }
}

const ALL_SCENARIOS = [...SCENARIOS, ...ISOLATED];
const picked = (opts, s) => !opts.only.length || opts.only.some((o) => s.id.toUpperCase().startsWith(o));

/**
 * The scenarios to run on `driver`, in declaration order. A scenario's
 * prerequisites (`needs`) run with it, so `--only B` still has A's records
 * to check.
 */
function selection(opts, driver) {
  const all = ALL_SCENARIOS.filter((s) => s.drivers.includes(driver));
  const needed = new Set(all.filter((s) => picked(opts, s)).flatMap((s) => [s.id, ...(s.needs ?? [])]));
  return all.filter((s) => needed.has(s.id));
}

async function runDriver(driver, { opts, runDir, runId, findings, results }) {
  const dir = path.join(runDir, driver);
  const secrets = [];
  const binding = { scenario: "setup", coreBin: opts.coreBin };
  const driverResults = { scenarios: {}, results: {} };
  results.drivers[driver] = driverResults;
  const url = { sqlite: null, file: "file:<home>/storage-file", memory: "memory" }[driver];
  const main = new Rig({ name: "main", dir, driver, url, findings, binding, secrets, results: driverResults.results });
  const isolated = [];
  // Scenarios talk to the main rig directly; it carries the run's tag, the
  // state shared between scenarios and the factory for isolated rigs.
  const ctx = Object.assign(main, {
    tag: runId.slice(-12).replace(/[^0-9A-Za-z]/g, "").toLowerCase(),
    state: {},
    isolated: async (name, o = {}) => {
      const r = new Rig({ name, dir: path.join(dir, `iso-${name}`), driver, url: o.url, masterKey: o.masterKey !== false, findings, binding, secrets, results: driverResults.results });
      isolated.push(r);
      return r;
    },
  });

  const list = selection(opts, driver);
  if (!list.length) {
    // `main()` already refused a selection that runs nowhere; this driver
    // simply has none of the selected scenarios.
    console.log(`${driver.padEnd(7)}: no selected scenario runs on this driver; skipped`);
    driverResults.skipped = true;
    return;
  }
  try {
    await main.start();
    for (const s of list) {
      binding.scenario = s.id;
      const before = findings.checks.length;
      const notesBefore = findings.items.filter((f) => f.severity === "note").length;
      const started = Date.now();
      let error = null;
      console.log(`${driver.padEnd(7)}: ${s.id}`);
      try {
        if (!main.alive) await main.start();
        await s.run(ctx);
      } catch (e) {
        error = scrub(e.message, secrets);
        findings.add({ engine: driver, scenario: s.id, id: "crashed", severity: "high", expected: "the scenario runs to the end", actual: error, evidence: `${driver}/core.log` });
      } finally {
        // Isolated cores never outlive their scenario (F restarts E's rig itself).
        for (const r of isolated.splice(0)) await r.stop().catch(() => {});
      }
      const checks = findings.checks.slice(before).filter((c) => c.engine === driver);
      driverResults.scenarios[s.id] = {
        checks: checks.length,
        failed: checks.filter((c) => !c.ok).length,
        notes: findings.items.filter((f) => f.severity === "note").length - notesBefore,
        ms: Date.now() - started,
        error,
      };
    }
  } finally {
    // A failed boot or scenario never leaves a core running.
    await main.stop().catch(() => {});
    if (ctx.state.legacy?.rig) await ctx.state.legacy.rig.stop().catch(() => {});
  }
}

function renderReport({ runId, results, findings }) {
  const lines = [`# Storage scenarios ${runId}`, ""];
  for (const [driver, dr] of Object.entries(results.drivers)) {
    if (dr.skipped) {
      lines.push(`## ${driver}`, "", "Skipped: no selected scenario runs on this driver.", "");
      continue;
    }
    lines.push(`## ${driver}`, "", "| scenario | checks passed | notes | time | crashed |", "| --- | --- | --- | --- | --- |");
    for (const [id, s] of Object.entries(dr.scenarios))
      lines.push(`| ${id} | ${s.checks - s.failed}/${s.checks} | ${s.notes} | ${((s.ms ?? 0) / 1000).toFixed(0)}s | ${s.error ? "yes" : ""} |`);
    lines.push("");
  }
  const real = findings.filter((f) => f.severity !== "note");
  lines.push(`## Findings (${real.length})`, "", "| id | severity | basis | expected | actual |", "| --- | --- | --- | --- | --- |");
  const order = { high: 0, medium: 1, low: 2 };
  for (const f of [...real].sort((a, b) => (order[a.severity] ?? 3) - (order[b.severity] ?? 3)))
    lines.push(`| ${f.id} | ${f.severity} | ${f.basis} | ${f.expected} | ${String(f.actual).replace(/\|/g, "\\|").replace(/\n/g, " ").slice(0, 300)} |`);
  lines.push("", "## Notes", "");
  for (const f of findings.filter((x) => x.severity === "note")) lines.push(`- **${f.id}** (${f.basis}): ${f.actual}`);
  lines.push(
    "",
    "## Not driven (not findings)",
    "",
    "- KV port: no RPC writes `tsd.kv` without a bound memory engine.",
    "- Agent and module principal scopes: need an agent turn or a module storage grant (no module declares one).",
    "- SaaS tenants: need the SaaS server mode and a profile lease.",
    "- Artifact blobs: no RPC creates an artifact; the blob port is driven through voice notes.",
  );
  return lines.join("\n") + "\n";
}

async function writeOutputs(runDir, runId, results, findings) {
  await fsp.writeFile(path.join(runDir, "findings.json"), JSON.stringify(findings.items, null, 2));
  await fsp.writeFile(path.join(runDir, "checks.json"), JSON.stringify(findings.checks, null, 2));
  await fsp.writeFile(path.join(runDir, "results.json"), JSON.stringify(results, null, 2));
  const md = renderReport({ runId, results, findings: findings.items });
  await fsp.writeFile(path.join(runDir, "report.md"), md);
  return md;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.gradeOnly) {
    const findings = JSON.parse(await fsp.readFile(path.join(opts.gradeOnly, "findings.json"), "utf8"));
    const results = JSON.parse(await fsp.readFile(path.join(opts.gradeOnly, "results.json"), "utf8"));
    const md = renderReport({ runId: path.basename(opts.gradeOnly), results, findings });
    await fsp.writeFile(path.join(opts.gradeOnly, "report.md"), md);
    console.log(md);
    return;
  }
  // A selection that runs nothing must not read as a completed run.
  const unknown = opts.only.filter((o) => !ALL_SCENARIOS.some((s) => s.id.toUpperCase().startsWith(o)));
  if (unknown.length)
    throw new Error(`--only ${unknown.join(",")} matches no scenario (${ALL_SCENARIOS.map((s) => s.id).join(", ")})`);
  // Every requested prefix must run somewhere: `--driver file --only A,I`
  // would otherwise drop I without a word.
  const runsOn = (o) => ALL_SCENARIOS.some((s) => s.id.toUpperCase().startsWith(o) && s.drivers.some((d) => opts.drivers.includes(d)));
  const nowhere = opts.only.filter((o) => !runsOn(o));
  if (nowhere.length)
    throw new Error(`--only ${nowhere.join(",")} runs on no selected driver (--driver ${opts.drivers.join(",")}; ${ALL_SCENARIOS.filter((s) => nowhere.some((o) => s.id.toUpperCase().startsWith(o))).map((s) => `${s.id}: ${s.drivers.join("/")}`).join("; ")})`);
  if (!opts.drivers.some((d) => selection(opts, d).length))
    throw new Error(`no scenario runs on --driver ${opts.drivers.join(",")}`);
  if (!fs.existsSync(opts.coreBin))
    throw new Error(`core binary not found at ${opts.coreBin}\nbuild it: cargo build -p openhuman-cli --bin openhuman-core`);
  const runId = new Date().toISOString().replace(/[:.]/g, "-");
  // Resolved: the secrets backend refuses a directory with a symlinked
  // ancestor, and a symlinked target/ must not decide a secrets check.
  await fsp.mkdir(opts.runRoot, { recursive: true });
  const runDir = path.join(fs.realpathSync(opts.runRoot), runId);
  await fsp.mkdir(runDir, { recursive: true });
  console.log(`run dir : ${runDir}`);
  const findings = new Findings();
  const results = { run_id: runId, started: new Date().toISOString(), drivers: {} };
  for (const driver of opts.drivers) {
    try {
      await runDriver(driver, { opts, runDir, runId, findings, results });
    } catch (e) {
      const msg = scrub(e.message, []);
      console.error(`${driver}: ${msg}`);
      findings.add({ engine: driver, scenario: "setup", id: "aborted", severity: "high", expected: "the driver's core boots", actual: msg, evidence: `${driver}/core.log` });
    }
  }
  results.finished = new Date().toISOString();
  await writeOutputs(runDir, runId, results, findings);
  console.log(`\nreport  : ${path.join(runDir, "report.md")}\nfindings: ${findings.items.filter((f) => f.severity !== "note").length}`);
}

main()
  .then(() => process.exit(0))
  .catch((e) => {
    console.error(`\nfatal: ${scrub(e.stack || e.message, [])}`);
    process.exit(1);
  });
