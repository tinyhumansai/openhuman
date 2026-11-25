// Storage-scenario plumbing. The core rig (a headless `openhuman-core serve`
// with its own HOME, JSON-RPC, scrubbed logs, findings) is memory-scenarios';
// this file adds what storage checks need: a hard kill, a read-only view of
// the SQLite database, tree hashes and a plaintext grep.

import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { createHash, randomBytes } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { sleep } from "../memory-scenarios/lib.mjs";

export {
  Core,
  JsonlLog,
  Findings,
  RpcError,
  waitFor,
  withRetries,
  pick,
  scrub,
  sleep,
  randomUUID,
} from "../memory-scenarios/lib.mjs";

/** SIGKILL the core (no shutdown path runs) and wait for the exit. */
export async function killHard(core) {
  if (!core?.proc || core.exited) return;
  core.proc.kill("SIGKILL");
  const deadline = Date.now() + 10_000;
  while (!core.exited && Date.now() < deadline) await sleep(50);
}

/**
 * An unsigned session JWT for `userId`. The core never validates a session
 * against the backend (`auth_set_credential`); it only reads `sub` and `exp`.
 * `nonce` makes each token unique, so it can be grepped for on disk.
 */
export function mintSessionToken(userId, nonce = randomBytes(12).toString("hex")) {
  const b64 = (o) => Buffer.from(JSON.stringify(o)).toString("base64url");
  const now = Math.floor(Date.now() / 1000);
  return [
    b64({ alg: "none", typ: "JWT" }),
    b64({
      sub: userId,
      iat: now,
      exp: now + 86_400,
      email: `${userId}@storscen.invalid`,
      nonce,
    }),
    "storscen",
  ].join(".");
}

export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

/**
 * The list in an RPC answer. Answers come in two envelopes: `{logs, result}`
 * (unwrapped by Core.rpc, so the list may be the value itself) and the
 * threads domain's `{data, meta}`; inside either, the list may sit under
 * `key` (`{messages: [...]}`).
 */
export function listOf(value, key) {
  let v = value;
  if (v && typeof v === "object" && !Array.isArray(v) && "data" in v && "meta" in v) v = v.data;
  if (Array.isArray(v)) return v;
  const inner = v?.[key] ?? v?.result?.[key] ?? v?.result;
  return Array.isArray(inner) ? inner : [];
}

/** Files under `root` whose whole content equals `bytes`. */
export async function filesWithBytes(root, bytes) {
  const hits = [];
  async function walk(dir) {
    let entries = [];
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) await walk(p);
      else if (e.isFile() && (await fsp.stat(p)).size === bytes.length && (await fsp.readFile(p)).equals(bytes)) hits.push(p);
    }
  }
  await walk(root);
  return hits;
}

/** Every file under `root` (relative path -> sha256), for before/after comparisons. */
export async function treeDigest(root, { skip = () => false } = {}) {
  const out = {};
  async function walk(dir) {
    let entries = [];
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = path.join(dir, e.name);
      const rel = path.relative(root, p);
      if (skip(rel)) continue;
      if (e.isDirectory()) await walk(p);
      else if (e.isFile()) out[rel] = sha256(await fsp.readFile(p));
    }
  }
  await walk(root);
  return out;
}

/** Files under `root` whose bytes contain `needle` (binary-safe). */
export async function grepTree(root, needle, { skip = () => false } = {}) {
  const hits = [];
  const n = Buffer.from(needle);
  async function walk(dir) {
    let entries = [];
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = path.join(dir, e.name);
      if (skip(path.relative(root, p))) continue;
      if (e.isDirectory()) await walk(p);
      else if (e.isFile() && (await fsp.readFile(p)).includes(n)) hits.push(p);
    }
  }
  await walk(root);
  return hits;
}

/** Every file named `name` under `root`. */
export async function findFiles(root, name) {
  const found = [];
  async function walk(dir) {
    let entries = [];
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) await walk(p);
      else if (e.name === name) found.push(p);
    }
  }
  await walk(root);
  return found;
}

/**
 * A read-only view of a tinystoragedrivers SQLite database. The driver's table
 * names are configurable, so tables are found by their columns:
 * documents (scope, coll, id, body), streams (scope, name, seq, body), blobs
 * (scope, key, bytes) and KV (scope, key, value, version).
 *
 * Opening read-only never creates the file; the core must be stopped first
 * when the result has to be final (WAL frames are read either way).
 */
export class StorageDb {
  constructor(file) {
    if (!fs.existsSync(file)) throw new Error(`no database at ${file}`);
    this.file = file;
    this.db = new DatabaseSync(file, { readOnly: true });
    this.tables = {};
    const names = this.db
      .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
      .all()
      .map((r) => r.name);
    for (const t of names) {
      const cols = new Set(
        this.db.prepare(`PRAGMA table_info("${t}")`).all().map((c) => c.name),
      );
      const has = (...c) => c.every((x) => cols.has(x));
      if (has("scope", "coll", "id", "body", "version")) this.tables.docs = t;
      else if (has("scope", "name", "seq", "body")) this.tables.streams = t;
      else if (has("scope", "key", "bytes", "sha256")) this.tables.blobs = t;
      else if (has("scope", "key", "value", "version")) this.tables.kv = t;
    }
  }
  /** Rows as plain objects (node:sqlite returns null-prototype rows). */
  all(sql, ...args) {
    return this.db.prepare(sql).all(...args).map((r) => ({ ...r }));
  }
  integrity() {
    return this.all("PRAGMA integrity_check").map((r) => Object.values(r)[0]);
  }
  /** Live documents of `coll`, optionally in one scope. */
  docs(coll, scope) {
    if (!this.tables.docs) return [];
    const q = `SELECT scope, coll, id, version, body FROM "${this.tables.docs}" WHERE coll = ? AND deleted = 0`;
    return scope === undefined
      ? this.all(q, coll)
      : this.all(`${q} AND scope = ?`, coll, scope);
  }
  /** Rows of the documents table whose body contains `text`. */
  docsContaining(text) {
    if (!this.tables.docs) return [];
    return this.all(
      `SELECT scope, coll, id FROM "${this.tables.docs}" WHERE deleted = 0 AND instr(body, ?) > 0`,
      text,
    );
  }
  /** Count of documents per (scope, coll). */
  census() {
    if (!this.tables.docs) return [];
    return this.all(
      `SELECT scope, coll, count(*) AS n FROM "${this.tables.docs}" WHERE deleted = 0 GROUP BY scope, coll ORDER BY scope, coll`,
    );
  }
  streamRows(nameLike) {
    if (!this.tables.streams) return [];
    return this.all(
      `SELECT scope, name, seq, body FROM "${this.tables.streams}" WHERE name LIKE ? ORDER BY scope, name, seq`,
      nameLike,
    );
  }
  blob(keyLike) {
    if (!this.tables.blobs) return [];
    return this.all(
      `SELECT scope, key, size, bytes FROM "${this.tables.blobs}" WHERE key LIKE ?`,
      keyLike,
    );
  }
  kvRows() {
    if (!this.tables.kv) return [];
    return this.all(
      `SELECT scope, key FROM "${this.tables.kv}" WHERE deleted = 0`,
    );
  }
  /** Every distinct scope that holds any record. */
  scopes() {
    const out = new Set();
    for (const t of Object.values(this.tables))
      for (const r of this.all(`SELECT DISTINCT scope FROM "${t}"`))
        out.add(r.scope);
    return [...out].sort();
  }
  close() {
    this.db.close();
  }
}

/** Open, run `fn`, close. */
export function withDb(file, fn) {
  const db = new StorageDb(file);
  try {
    return fn(db);
  } finally {
    db.close();
  }
}

/** Write a SQLite file from DDL + rows (used to build legacy fixtures). */
export function sqliteFile(file, sql) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const db = new DatabaseSync(file);
  try {
    db.exec(sql);
  } finally {
    db.close();
  }
}
