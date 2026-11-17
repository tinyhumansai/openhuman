import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { StorageDb, sqliteFile, grepTree } from "../storage-scenarios/lib.mjs";

// The table shapes of tinystoragedrivers' SQLite driver, under names other
// than the defaults: the reader must find them by their columns.
const TSD_SHAPE = `
  CREATE TABLE d (scope TEXT, coll TEXT, id TEXT, version INTEGER, schema INTEGER,
    created_ms INTEGER, updated_ms INTEGER, expires_ms INTEGER, deleted INTEGER, body TEXT);
  CREATE TABLE s (scope TEXT, name TEXT, seq INTEGER, body TEXT);
  CREATE TABLE b (scope TEXT, key TEXT, content_type TEXT, size INTEGER, sha256 BLOB,
    expires_ms INTEGER, bytes BLOB);
  CREATE TABLE k (scope TEXT, key TEXT, version INTEGER, expires_ms INTEGER, deleted INTEGER, value TEXT);
  INSERT INTO d VALUES ('local', 'cron.jobs', 'j1', 1, 0, 0, 0, NULL, 0, '{"name":"storscen-a"}');
  INSERT INTO d VALUES ('local', 'cron.jobs', 'j2', 1, 0, 0, 0, NULL, 1, '{"name":"gone"}');
  INSERT INTO d VALUES ('u:x', 'secrets', 'k', 1, 0, 0, 0, NULL, 0, 'enc2:abc');
  INSERT INTO s VALUES ('local', 'conversations.messages/t', 0, '{"content":"hello"}');
  INSERT INTO b VALUES ('local', 'voice_notes/n.webm', 'audio/webm', 3, x'00', NULL, x'010203');
  INSERT INTO k VALUES ('local', 'memory/state/jobs', 1, NULL, 0, '{}');
`;

test("StorageDb finds tsd tables by their columns and reads them read-only", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "storscen-"));
  const file = path.join(dir, "storage.db");
  sqliteFile(file, TSD_SHAPE);
  const db = new StorageDb(file);
  try {
    assert.deepEqual(db.tables, { docs: "d", streams: "s", blobs: "b", kv: "k" });
    assert.deepEqual(db.docs("cron.jobs").map((r) => r.id), ["j1"], "deleted rows are not live");
    assert.deepEqual(db.census(), [
      { scope: "local", coll: "cron.jobs", n: 1 },
      { scope: "u:x", coll: "secrets", n: 1 },
    ]);
    assert.deepEqual(db.scopes(), ["local", "u:x"]);
    assert.equal(Buffer.from(db.blob("%n.webm%")[0].bytes).toString("hex"), "010203");
    assert.deepEqual(db.integrity(), ["ok"]);
    assert.throws(() => db.db.exec("DELETE FROM d"), "opened read-only");
  } finally {
    db.close();
  }
});

test("grepTree finds bytes in binary files and misses absent ones", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "storscen-"));
  fs.mkdirSync(path.join(dir, "a"));
  fs.writeFileSync(path.join(dir, "a", "x.bin"), Buffer.concat([Buffer.from([0, 1]), Buffer.from("needle-42"), Buffer.from([255])]));
  assert.equal((await grepTree(dir, "needle-42")).length, 1);
  assert.equal((await grepTree(dir, "needle-43")).length, 0);
});
