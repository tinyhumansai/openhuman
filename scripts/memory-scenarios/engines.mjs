// The two engines a run tests.
//
// - local: a throwaway CortexDB container per run (its own compose project,
//   volume and port; never the user's `cortexdb` / `cortexdb-data` / :3141),
//   inference through the local Ollama when reachable.
// - builtin: the user's real account. Guards in `builtinGuards` keep the run
//   from moving, erasing or importing anything of the account's own.

import fsp from "node:fs/promises";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { randomBytes } from "node:crypto";
import { freePort, waitFor, pick } from "./lib.mjs";

const exec = promisify(execFile);

/** Names a run must never touch. */
const FORBIDDEN_PROJECTS = new Set([
  "cortexdb",
  "tinymemory-cortexdb",
  "tinymemory-cortexdb-test",
]);
const FORBIDDEN_PORT = 3141;

const OLLAMA = process.env.MEMSCEN_OLLAMA_URL || "http://127.0.0.1:11434";
export const OLLAMA_EMBED_MODEL =
  process.env.MEMSCEN_EMBED_MODEL || "nomic-embed-text";
export const OLLAMA_EMBED_DIMS = process.env.MEMSCEN_EMBED_DIMS || "768";
export const OLLAMA_URL = OLLAMA;
export const OLLAMA_CHAT_MODEL =
  process.env.MEMSCEN_CHAT_MODEL || "llama3.2:3b";

/** Ollama's model names, or null when it is unreachable. */
async function ollamaModels() {
  try {
    const r = await fetch(`${OLLAMA}/api/tags`, {
      signal: AbortSignal.timeout(5000),
    });
    if (!r.ok) return null;
    return ((await r.json()).models ?? []).map((m) => m.name);
  } catch {
    return null;
  }
}

/** Pull `model` through Ollama's API when it is missing. */
async function ensureOllamaModel(model, have) {
  const want = model.includes(":") ? model : `${model}:latest`;
  if (have.includes(want) || have.includes(model)) return;
  const r = await fetch(`${OLLAMA}/api/pull`, {
    method: "POST",
    body: JSON.stringify({ model, stream: false }),
    signal: AbortSignal.timeout(20 * 60_000),
  });
  if (!r.ok) throw new Error(`ollama pull ${model}: HTTP ${r.status}`);
}

/**
 * Start the per-run CortexDB. Returns `{endpoint, apiKey, inference, stop}`.
 * `inference` says what backs embeddings and extraction, for report.md.
 */
export async function startLocalCortex({
  runId,
  scriptDir,
  tinymemoryDir,
  keep,
  log,
}) {
  const project = `memscen-${runId
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, "")
    .slice(-24)}`;
  if (FORBIDDEN_PROJECTS.has(project))
    throw new Error(`refusing compose project ${project}`);
  const port = await freePort();
  if (port === FORBIDDEN_PORT) throw new Error("refusing port 3141");
  const apiKey = `memscen-${randomBytes(12).toString("hex")}`;

  const models = await ollamaModels();
  let compose;
  let env;
  let inference;
  if (models) {
    await ensureOllamaModel(OLLAMA_EMBED_MODEL, models);
    await ensureOllamaModel(OLLAMA_CHAT_MODEL, models);
    compose = ["-f", path.join(scriptDir, "cortexdb", "docker-compose.yml")];
    env = {
      CORTEXDB_VERSION: "v0.10.5",
      MEMSCEN_CORTEX_KEY: apiKey,
      MEMSCEN_CORTEX_PORT: String(port),
      // From inside the container the host's loopback is host.docker.internal.
      CORTEX_INFERENCE_URL: `${OLLAMA.replace("127.0.0.1", "host.docker.internal").replace("localhost", "host.docker.internal")}/v1`,
      // Embeddings use Ollama's native API at the bare base: CortexDB pins the
      // provider as `ollama:<model>`, and under /v1 every embedding 404s
      // ("model not found"), leaving each store waiting for an index that never
      // comes (408 WAIT_TIMEOUT after 30 s).
      CORTEX_EMBEDDING_URL: OLLAMA.replace(
        "127.0.0.1",
        "host.docker.internal",
      ).replace("localhost", "host.docker.internal"),
      CORTEX_EMBEDDING_MODEL: OLLAMA_EMBED_MODEL,
      CORTEX_EMBEDDING_DIMS: OLLAMA_EMBED_DIMS,
      CORTEX_CHAT_MODEL: OLLAMA_CHAT_MODEL,
    };
    inference = {
      kind: "ollama",
      embeddings: `${OLLAMA_EMBED_MODEL} (${OLLAMA_EMBED_DIMS} dims)`,
      chat: OLLAMA_CHAT_MODEL,
      recall_quality_checked: true,
    };
  } else {
    // tinymemory's compose with its deterministic inference double: plumbing
    // still works, retrieval is not semantic.
    compose = [
      "-f",
      path.join(tinymemoryDir, "integration", "cortexdb", "docker-compose.yml"),
    ];
    env = {
      CORTEXDB_VERSION: "v0.10.5",
      CORTEXDB_PORT: String(port),
      TINYMEMORY_TEST_CORTEX_KEY: apiKey,
    };
    inference = {
      kind: "mock-inference",
      embeddings: "deterministic",
      chat: "deterministic",
      recall_quality_checked: false,
    };
  }

  const dc = (...args) =>
    exec(
      "docker",
      ["compose", "--project-name", project, ...compose, ...args],
      {
        env: { ...process.env, ...env },
        maxBuffer: 16 * 1024 * 1024,
      },
    );
  log?.(
    `local   : compose project ${project}, port ${port}, inference ${inference.kind}`,
  );
  await dc(
    "up",
    "-d",
    ...(models ? [] : ["--build", "--wait", "mock-inference"]),
  );
  if (!models) await dc("up", "-d", "cortex");
  const endpoint = `http://127.0.0.1:${port}`;
  await waitFor(
    async () =>
      (
        await fetch(`${endpoint}/v1/admin/ready`, {
          signal: AbortSignal.timeout(3000),
        })
      ).ok,
    { timeoutMs: 180_000, intervalMs: 1000, what: "CortexDB ready" },
  );
  const version = await fetch(`${endpoint}/v1/admin/version`)
    .then((r) => r.text())
    .catch(() => "?");

  return {
    project,
    endpoint,
    apiKey,
    inference: { ...inference, cortexdb_version: version.trim() },
    /** Stop the CortexDB container (to force engine errors), and start it again. */
    pause: () => dc("stop", "cortex"),
    resume: async () => {
      await dc("start", "cortex");
      await waitFor(
        async () => (await fetch(`${endpoint}/v1/admin/ready`)).ok,
        {
          timeoutMs: 120_000,
          what: "CortexDB ready again",
        },
      );
    },
    async stop() {
      await dc("down", "--remove-orphans", ...(keep ? [] : ["--volumes"]));
    },
  };
}

/**
 * Read CortexDB directly with the run's own key, for checks that need what the
 * engine stored rather than what the core reports (labels, basenames, scopes).
 */
export function cortexReader({ endpoint, apiKey }) {
  const call = async (method, route, body) => {
    const r = await fetch(`${endpoint}${route}`, {
      method,
      headers: {
        authorization: `Bearer ${apiKey}`,
        "content-type": "application/json",
      },
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(30_000),
    });
    const text = await r.text();
    let json;
    try {
      json = JSON.parse(text);
    } catch {
      json = { raw: text };
    }
    return { status: r.status, body: json };
  };
  return {
    get: (route) => call("GET", route),
    post: (route, body) => call("POST", route, body),
  };
}

// ---------------------------------------------------------------------------
// builtin: the real account
// ---------------------------------------------------------------------------

export const API_KEY_ENV = "OPENHUMAN_BACKEND_API_KEY";
export const SESSION_ENV = "OPENHUMAN_BACKEND_SESSION_TOKEN";
const HOME_DIR = process.env.HOME || "";
const SESSION_FILE =
  process.env.MEMSCEN_SESSION_FILE || path.join(HOME_DIR, ".memscen-session");
const KEY_FILE =
  process.env.MEMSCEN_KEY_FILE || path.join(HOME_DIR, ".memscen-key");

/**
 * The account credential for the builtin engine, read at spawn time:
 * the app's session JWT from ~/.memscen-session when present (preferred: it
 * carries the memory scope), else the API key from ~/.memscen-key. It reaches
 * the core only through the environment (credentials/ops/boot_env.rs
 * seeds it); it is never printed, logged, written to the run dir or passed
 * to an RPC. `subject` (a session's user id) is returned for guard placement
 * only and is never printed either.
 */
export async function builtinCredential({ only } = {}) {
  const read = (f) =>
    fsp
      .readFile(f, "utf8")
      .then((t) => t.trim())
      .catch(() => null);
  const session = only === "key" ? null : await read(SESSION_FILE);
  if (session) {
    const parts = session.split(".");
    if (parts.length !== 3)
      throw new Error(`${SESSION_FILE} does not hold a JWT`);
    let subject;
    try {
      const payload = JSON.parse(Buffer.from(parts[1], "base64url").toString());
      subject = payload.sub ?? payload._id ?? payload.userId;
    } catch {
      throw new Error(`${SESSION_FILE}: the JWT payload does not decode`);
    }
    if (!subject)
      throw new Error(`${SESSION_FILE}: the session carries no subject`);
    return {
      kind: "session",
      env: { [SESSION_ENV]: session },
      secret: session,
      subject: String(subject),
    };
  }
  const key = await read(KEY_FILE);
  if (!key)
    throw new Error(
      `builtin needs ${SESSION_FILE} (session JWT) or ${KEY_FILE} (API key)`,
    );
  if (!/^tiny_[A-Za-z0-9_-]{8,}$/.test(key))
    throw new Error(`${KEY_FILE} does not hold a tiny_ API key`);
  return {
    kind: "key",
    env: { [API_KEY_ENV]: key },
    secret: key,
    subject: null,
  };
}

/** The active workspace directory, read from the running core. */
export async function activeWorkspace(core) {
  const snap = await core.rpc("openhuman.config_get", {});
  const cfg = pick(snap, "config", "snapshot.config", "snapshot") ?? snap;
  const ws = pick(cfg, "workspace_dir") ?? pick(snap, "workspace_dir");
  if (!ws) throw new Error("config_get carries no workspace_dir");
  return { workspace: ws, config: cfg };
}

/**
 * Guard 1: mark the layout migration done for this throwaway workspace, so the
 * memory background job's tick never starts moving the account's real legacy
 * tree. Written as the job's own state file; `migration_status` then reads
 * `cleaned`. Returns the state read back, or throws.
 */
export async function placeMigrationGuard(core, statePath) {
  await fsp.mkdir(path.dirname(statePath), { recursive: true });
  const state = {
    phase: "cleaned",
    copied: 0,
    replayed: 0,
    switched: true,
    caught_up: true,
    cleaning: false,
    takeover: false,
  };
  await fsp.writeFile(statePath, JSON.stringify(state, null, 2));
  const status = await core.rpc("openhuman.memory_migration_status", {});
  const phase = pick(status, "state.phase");
  if (phase !== "cleaned" || status.running)
    throw new Error(
      `migration guard not in effect: phase=${phase} running=${status.running}`,
    );
  return status;
}

/** Guard 1, checked at the end: nothing moved, nothing imported. */
export async function verifyBuiltinUntouched(core) {
  const migration = await core.rpc("openhuman.memory_migration_status", {});
  const imp = await core.rpc("openhuman.memory_import_status", {});
  return {
    migration_phase: pick(migration, "state.phase"),
    migration_running: migration.running,
    migration_copied: pick(migration, "state.copied"),
    import_phase: pick(imp, "state.phase"),
    ok:
      pick(migration, "state.phase") === "cleaned" &&
      !migration.running &&
      (pick(migration, "state.copied") ?? 0) === 0 &&
      ["idle", undefined].includes(pick(imp, "state.phase")),
  };
}

/**
 * Guard 2: forget exactly the ids this run stored, then confirm each is gone.
 * Returns the ids that survived.
 */
export async function forgetLedger(core, ledger) {
  // core.rpc, not tryRpc: an error here must fail the clean-up, never read as
  // "nothing survived".
  const ids = ledger.list();
  for (let i = 0; i < ids.length; i += 50)
    await core.rpc("openhuman.memory_forget", { ids: ids.slice(i, i + 50) });
  const survivors = [];
  for (let i = 0; i < ids.length; i += 50) {
    const got = await core.rpc("openhuman.memory_items_get", {
      ids: ids.slice(i, i + 50),
    });
    for (const item of got?.items ?? []) survivors.push(item.id);
  }
  return survivors;
}

/**
 * A logging pass-through in front of the run's CortexDB, so checks can read
 * what the core actually SENT (attribution such as observed_actor and subject
 * is written on the wire and not returned on read-back). Every request body is
 * appended to `logFile` (JSONL, scrubbed); responses are passed through as is.
 */
export async function startCortexWireLog({ target, logFile, secrets = [] }) {
  const http = await import("node:http");
  const { scrub } = await import("./lib.mjs");
  const requests = [];
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const c of req) chunks.push(c);
    const body = Buffer.concat(chunks);
    let parsed = null;
    try {
      parsed = body.length ? JSON.parse(body.toString()) : null;
    } catch {
      /* not JSON */
    }
    const entry = {
      at: new Date().toISOString(),
      method: req.method,
      path: req.url,
      body: parsed,
    };
    requests.push(entry);
    await fsp
      .appendFile(logFile, scrub(JSON.stringify(entry), secrets) + "\n")
      .catch(() => {});
    const headers = { ...req.headers };
    delete headers.host;
    try {
      const upstream = await fetch(`${target}${req.url}`, {
        method: req.method,
        headers,
        body: ["GET", "HEAD"].includes(req.method) ? undefined : body,
        signal: AbortSignal.timeout(300_000),
      });
      const out = Buffer.from(await upstream.arrayBuffer());
      const h = {};
      upstream.headers.forEach((v, k) => {
        if (
          ![
            "content-encoding",
            "transfer-encoding",
            "content-length",
            "connection",
          ].includes(k)
        )
          h[k] = v;
      });
      res.writeHead(upstream.status, h);
      res.end(out);
    } catch (e) {
      res.writeHead(502, { "content-type": "application/json" });
      res.end(JSON.stringify({ error_code: "PROXY", message: e.message }));
    }
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    requests,
    close: () =>
      new Promise((r) => {
        // Keep-alive sockets would hold close() (and the process) open.
        server.closeAllConnections?.();
        server.close(r);
      }),
  };
}
