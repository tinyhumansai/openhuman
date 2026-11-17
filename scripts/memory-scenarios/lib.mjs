// Harness plumbing shared by run.mjs and the scenarios: a headless core over
// JSON-RPC (shaped like openhuman-benchmarks' life-scenarios/run.mjs), the composer's event
// stream, a turn driver, an RPC log, the findings list and the ledger of every
// id a run stored.

import fs from "node:fs";
import fsp from "node:fs/promises";
import net from "node:net";
import path from "node:path";
import { spawn } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";

export { sleep, randomUUID };

/** A free loopback port. */
export async function freePort() {
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

/**
 * Remove anything token-shaped from text bound for a file: JWTs (`eyJ…`),
 * bearer values and the given secrets. Every file this suite writes goes
 * through it, so a run directory can be shared.
 */
export function scrub(text, secrets = []) {
  let out = String(text);
  for (const s of secrets)
    if (s && s.length >= 8) out = out.split(s).join("<redacted>");
  out = out.replace(
    /eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*/g,
    "<jwt>",
  );
  out = out.replace(/tiny_[A-Za-z0-9_-]{8,}/g, "<tiny-key>");
  out = out.replace(/(bearer\s+)[A-Za-z0-9._~+/-]{16,}/gi, "$1<redacted>");
  return out;
}

/** Append-only JSONL writer whose lines are scrubbed. */
export class JsonlLog {
  constructor(file, secrets) {
    this.file = file;
    this.secrets = secrets;
  }
  async write(obj) {
    await fsp.mkdir(path.dirname(this.file), { recursive: true });
    await fsp.appendFile(
      this.file,
      scrub(JSON.stringify(obj), this.secrets) + "\n",
    );
  }
}

/** Retry `fn` until it stops throwing. */
export async function withRetries(fn, { attempts = 10, delayMs = 500, what }) {
  let last;
  for (let i = 0; i < attempts; i += 1) {
    try {
      return await fn();
    } catch (e) {
      last = e;
      await sleep(delayMs);
    }
  }
  throw new Error(
    `${what} never settled after ${attempts} attempts: ${last?.message ?? last}`,
  );
}

/** Poll `probe` until it returns a truthy value, else throw. */
export async function waitFor(
  probe,
  { timeoutMs = 60_000, intervalMs = 500, what },
) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try {
      const v = await probe();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await sleep(intervalMs);
  }
  throw new Error(
    `${what} not reached within ${timeoutMs}ms${last ? `: ${last.message}` : ""}`,
  );
}

/** An RPC error that keeps the JSON-RPC error object for checks. */
export class RpcError extends Error {
  constructor(method, error) {
    super(`RPC ${method} error: ${JSON.stringify(error).slice(0, 500)}`);
    this.method = method;
    this.rpc = error;
  }
}

/**
 * `openhuman-core serve` with its own HOME. Shaped like life-scenarios' Core:
 * same env isolation, same health wait, same envelope unwrapping. Every RPC is
 * logged (scrubbed) to `rpcLog`.
 */
export class Core {
  constructor({ coreBin, rpcLog, secrets = [] }) {
    this.coreBin = coreBin;
    this.rpcLog = rpcLog;
    this.secrets = secrets;
    this.proc = null;
    this.port = 0;
    this.token = randomBytes(24).toString("hex");
    this.exited = null;
    this.seq = 0;
  }

  get url() {
    return `http://127.0.0.1:${this.port}`;
  }

  /** `extraEnv` carries what the engine needs (e.g. the builtin session token). */
  async start({ home, actionDir, logPath, extraEnv = {} }) {
    this.port = await freePort();
    await fsp.mkdir(actionDir, { recursive: true });
    const log = fs.createWriteStream(logPath, { flags: "a" });
    const env = {
      ...process.env,
      HOME: home,
      OPENHUMAN_HOME: path.join(home, ".openhuman"),
      OPENHUMAN_CORE_TOKEN: this.token,
      OPENHUMAN_CORE_PORT: String(this.port),
      OPENHUMAN_CORE_HOST: "127.0.0.1",
      OPENHUMAN_ACTION_DIR: actionDir,
      RUST_LOG: process.env.RUST_LOG || "info",
      ...extraEnv,
    };
    // The operator's own session or key must never reach a local-engine core.
    if (!("OPENHUMAN_BACKEND_SESSION_TOKEN" in extraEnv))
      delete env.OPENHUMAN_BACKEND_SESSION_TOKEN;
    if (!("OPENHUMAN_BACKEND_API_KEY" in extraEnv))
      delete env.OPENHUMAN_BACKEND_API_KEY;
    this.proc = spawn(this.coreBin, ["serve"], {
      env,
      stdio: ["ignore", "pipe", "pipe"],
      cwd: actionDir,
    });
    // The core log is scrubbed line by line on its way to disk.
    const pipe = (stream) => {
      let buf = "";
      stream.on("data", (chunk) => {
        buf += chunk.toString();
        let i;
        while ((i = buf.indexOf("\n")) >= 0) {
          log.write(scrub(buf.slice(0, i + 1), this.secrets));
          buf = buf.slice(i + 1);
        }
      });
      stream.on("end", () => buf && log.write(scrub(buf, this.secrets)));
    };
    pipe(this.proc.stdout);
    pipe(this.proc.stderr);
    this.proc.on("exit", (code, sig) => {
      this.exited = { code, sig };
    });
    return waitFor(
      async () => {
        if (this.exited)
          throw new Error(
            `core exited during boot (${JSON.stringify(this.exited)}); see ${logPath}`,
          );
        const r = await fetch(`${this.url}/health`, {
          signal: AbortSignal.timeout(3000),
        });
        return r.ok ? r.json() : null;
      },
      { timeoutMs: 120_000, what: `core health on ${this.url}` },
    );
  }

  /** Call a method; returns the result, throws RpcError on a JSON-RPC error. */
  async rpc(method, params = {}, timeoutMs = 120_000) {
    const id = `ms-${++this.seq}`;
    const started = Date.now();
    let body;
    try {
      const res = await fetch(`${this.url}/rpc`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${this.token}`,
        },
        body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
        signal: AbortSignal.timeout(timeoutMs),
      });
      const text = await res.text();
      if (!res.ok)
        throw new Error(
          `RPC ${method} HTTP ${res.status}: ${text.slice(0, 400)}`,
        );
      body = JSON.parse(text);
    } catch (e) {
      await this.rpcLog?.write({
        id,
        method,
        params,
        ms: Date.now() - started,
        transport_error: e.message,
      });
      throw e;
    }
    await this.rpcLog?.write({
      id,
      method,
      params,
      ms: Date.now() - started,
      result: body.result,
      error: body.error,
    });
    if (body.error) throw new RpcError(method, body.error);
    const r = body.result;
    // `apply_log_envelope` wraps a result that carried log lines.
    if (r && typeof r === "object" && "result" in r && "logs" in r)
      return r.result;
    return r;
  }

  /** `rpc` that returns `{ok, value|error}` instead of throwing. */
  async tryRpc(method, params, timeoutMs) {
    try {
      return { ok: true, value: await this.rpc(method, params, timeoutMs) };
    } catch (e) {
      return { ok: false, error: e.rpc ?? { message: e.message } };
    }
  }

  async stop() {
    if (!this.proc || this.exited) return;
    this.proc.kill("SIGTERM");
    const deadline = Date.now() + 15_000;
    while (!this.exited && Date.now() < deadline) await sleep(100);
    if (!this.exited) this.proc.kill("SIGKILL");
  }
}

/** The composer's event stream, `GET /events?client_id=…`. */
export class EventStream {
  constructor(core, clientId) {
    this.core = core;
    this.clientId = clientId;
    this.handlers = new Set();
    this.controller = new AbortController();
  }
  onEvent(fn) {
    this.handlers.add(fn);
    return () => this.handlers.delete(fn);
  }
  async connect() {
    const url = `${this.core.url}/events?client_id=${encodeURIComponent(this.clientId)}`;
    const res = await fetch(url, {
      headers: {
        authorization: `Bearer ${this.core.token}`,
        accept: "text/event-stream",
      },
      signal: this.controller.signal,
    });
    if (!res.ok || !res.body)
      throw new Error(`events subscribe failed: HTTP ${res.status}`);
    (async () => {
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
              try {
                const ev = JSON.parse(line.slice(5).trim());
                for (const h of this.handlers) h(ev);
              } catch {
                /* not JSON */
              }
            }
          }
        }
      } catch {
        /* aborted or core stopped */
      }
    })();
  }
  close() {
    this.controller.abort();
  }
}

/**
 * One composer turn, exactly as the desktop app sends it
 * (`channel_web_chat`, then `chat_done` | `chat_error` on /events).
 */
export async function sendTurn({
  core,
  events,
  clientId,
  threadId,
  message,
  timeoutMs = 240_000,
}) {
  let requestId = null;
  const toolCalls = [];
  const finished = new Promise((resolve) => {
    const off = events.onEvent((ev) => {
      if (ev.client_id && ev.client_id !== clientId) return;
      if (ev.event === "tool_call")
        toolCalls.push(ev.tool_name || ev.tool || "");
      if (ev.event === "chat_done" || ev.event === "chat_error") {
        if (requestId && ev.request_id && ev.request_id !== requestId) return;
        if (!requestId && ev.thread_id && ev.thread_id !== threadId) return;
        off();
        resolve(ev);
      }
    });
  });
  const started = Date.now();
  const ack = await core.rpc("openhuman.channel_web_chat", {
    client_id: clientId,
    thread_id: threadId,
    message,
    source: "type",
    queue_mode: "interrupt",
  });
  requestId = ack?.request_id || ack?.requestId || null;
  const done = await Promise.race([
    finished,
    sleep(timeoutMs).then(() => null),
  ]);
  const ms = Date.now() - started;
  if (!done)
    return { error: `no chat_done within ${timeoutMs}ms`, ms, toolCalls };
  if (done.event === "chat_error")
    return {
      error: `chat_error (${done.error_type || "unknown"}): ${String(done.message || "").slice(0, 300)}`,
      ms,
      toolCalls,
    };
  return {
    reply: done.full_response || done.response || "",
    ms,
    toolCalls,
    usage: done.usage,
  };
}

/**
 * The findings list. A finding is a defect or an issue the run observed; it
 * says whether it was READ (seen in a response, a log line or a CortexDB read)
 * or INFERRED (deduced from what was seen).
 */
export class Findings {
  constructor() {
    this.items = [];
    this.checks = [];
  }
  /** Record a check; a failed one becomes a finding. */
  check({
    engine,
    scenario,
    id,
    ok,
    expected,
    actual,
    evidence,
    severity = "medium",
    basis = "READ",
  }) {
    this.checks.push({ engine, scenario, id, ok, expected, actual, evidence });
    if (!ok)
      this.add({
        engine,
        scenario,
        id,
        severity,
        expected,
        actual,
        evidence,
        basis,
      });
    return ok;
  }
  add({
    engine,
    scenario,
    id,
    severity = "medium",
    expected,
    actual,
    evidence,
    basis = "READ",
  }) {
    this.items.push({
      id: `${engine}/${scenario}/${id}`,
      engine,
      scenario,
      severity,
      expected,
      actual:
        typeof actual === "string"
          ? actual
          : (JSON.stringify(actual) ?? String(actual)).slice(0, 600),
      evidence,
      basis,
    });
  }
  /** Something worth the user's eye that is not a failed expectation. */
  note({ engine, scenario, id, text, evidence, basis = "READ" }) {
    this.add({
      engine,
      scenario,
      id,
      severity: "note",
      expected: "(observation)",
      actual: text,
      evidence,
      basis,
    });
  }
}

/**
 * Every id a run stored, by engine. On the builtin engine teardown forgets
 * exactly these and checks they are gone; nothing else on the account is
 * touched.
 */
export class Ledger {
  constructor() {
    this.ids = new Set();
  }
  add(id) {
    if (id) this.ids.add(String(id));
  }
  addAll(ids) {
    for (const id of ids ?? []) this.add(id);
  }
  list() {
    return [...this.ids];
  }
}

/** First non-null value at any of `paths` (dotted) in `obj`. */
export function pick(obj, ...paths) {
  for (const p of paths) {
    let v = obj;
    for (const k of p.split(".")) v = v?.[k];
    if (v !== undefined && v !== null) return v;
  }
  return undefined;
}

/** Count lines in a file matching `re` (0 when absent). */
export async function countLogLines(file, re) {
  const text = await fsp.readFile(file, "utf8").catch(() => "");
  return text.split("\n").filter((l) => re.test(l)).length;
}
