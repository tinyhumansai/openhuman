// Scenarios that need a HOME of their own: D (two users), E (legacy import),
// F (layout v2 migration, then the no-master-key profile of #8076), G (Clear
// App Data, #8248) and J (sign-out keeps the API key, #8366). Each builds a
// rig with `ctx.isolated(name, opts)`: a fresh HOME and core, same checks.

import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { randomBytes } from "node:crypto";
import { pick, grepTree, withDb, treeDigest, listOf } from "./lib.mjs";
import { seedV1Profile, LEGACY_EXPECT, tinyflowsExpected } from "./legacy.mjs";

const FAR_FUTURE = "2099-01-01T00:00:00Z";
const json = (v) => JSON.stringify(v ?? null);
const errText = (r) => (r.ok ? "" : json(r.error));
const jobsOf = (r) => listOf(r.value, "jobs");
const exists = (p) => fs.existsSync(p);
const userDb = (dbs, user) => dbs.filter((p) => p.includes(`${path.sep}users${path.sep}${user}${path.sep}`));

/** One user's footprint on every RPC-reachable port. */
async function writeUserData(rig, user, tag) {
  const d = {
    cronName: `storscen-${user}-${tag}`,
    thread: `storscen-${user}-thread-${tag}`,
    note: `storscen_${user}_${tag}`.replace(/[^A-Za-z0-9_-]/g, "_"),
    provider: `storscen-${user}-provider`,
    marker: `storscen-marker-${user}-${tag}`,
  };
  const add = await rig.tryRpc("openhuman.cron_add", { name: d.cronName, schedule: { kind: "at", at: FAR_FUTURE }, job_type: "agent", prompt: d.marker });
  d.cronId = pick(add.value, "job.id", "result.job.id", "id");
  await rig.tryRpc("openhuman.threads_upsert", { id: d.thread, title: d.marker, created_at: new Date().toISOString() });
  await rig.tryRpc("openhuman.threads_message_append", { thread_id: d.thread, message: { id: `${d.thread}-m0`, content: d.marker, type: "text", extraMetadata: {}, sender: "user", createdAt: new Date().toISOString() } });
  await rig.tryRpc("openhuman.channel_web_voice_note_store", { note_id: d.note, audio_base64: Buffer.from(d.marker).toString("base64") });
  const token = `storscen-token-${randomBytes(8).toString("hex")}`;
  rig.addSecret(token);
  await rig.tryRpc("openhuman.auth_store_provider_credentials", { provider: d.provider, profile: "default", token });
  return d;
}

/** What `rig` sees of user data `d`: {cron, thread, note, provider} booleans. */
async function sees(rig, d) {
  const cron = jobsOf(await rig.tryRpc("openhuman.cron_list", {})).some((j) => j.id === d.cronId);
  const msgs = await rig.tryRpc("openhuman.threads_messages_list", { thread_id: d.thread });
  const thread = listOf(msgs.value, "messages").some((m) => m.content === d.marker);
  const nr = await rig.tryRpc("openhuman.channel_web_voice_note_read", { note_id: d.note });
  const b64 = pick(nr.value, "audio_base64", "result.audio_base64");
  const note = !!b64 && Buffer.from(b64, "base64").toString() === d.marker;
  const pl = await rig.tryRpc("openhuman.auth_list_provider_credentials", {});
  const provider = json(pl.value).includes(`"${d.provider}"`);
  return { cron, thread, note, provider };
}

// ---------------------------------------------------------------------------
// D. isolation
// ---------------------------------------------------------------------------

const isolation = {
  id: "D-isolation",
  title: "Two users: separate databases, no cross-reads",
  drivers: ["sqlite"],
  async run(ctx) {
    const rig = await ctx.isolated("D");
    await rig.start();
    await rig.signIn("storscen-alice");
    const alice = await writeUserData(rig, "storscen-alice", ctx.tag);
    await rig.signIn("storscen-bob");
    const bob = await writeUserData(rig, "storscen-bob", ctx.tag);

    const own = await sees(rig, bob);
    const other = await sees(rig, alice);
    for (const port of ["cron", "thread", "note", "provider"]) {
      ctx.check({
        id: `D-${port}`,
        ok: own[port] && !other[port],
        expected: `bob sees his own ${port} record (control) and not alice's`,
        actual: `own: ${own[port] ? "seen" : "NOT seen (control failed)"}; alice's: ${other[port] ? "SEEN" : "not seen"}`,
        severity: other[port] ? "high" : "medium",
      });
    }

    await rig.stop();
    const dbs = await rig.dbFiles();
    const [aDb] = userDb(dbs, "storscen-alice");
    const [bDb] = userDb(dbs, "storscen-bob");
    ctx.check({ id: "D-separate-files", ok: !!aDb && !!bDb && aDb !== bDb, expected: "users/storscen-alice/ and users/storscen-bob/ each have their own storage.db", actual: dbs.map((p) => path.relative(rig.oh, p)).join(", ") || "none" });
    if (aDb && bDb) {
      const aDir = path.dirname(aDb);
      const bDir = path.dirname(bDb);
      const bOwn = (await grepTree(bDir, bob.marker)).length;
      const bForeign = (await grepTree(bDir, alice.marker)).length;
      const aOwn = (await grepTree(aDir, alice.marker)).length;
      const aForeign = (await grepTree(aDir, bob.marker)).length;
      ctx.check({ id: "D-on-disk", ok: bOwn > 0 && aOwn > 0 && bForeign === 0 && aForeign === 0, expected: "each profile directory holds its own marker (control) and never the other's", actual: `alice dir: own ${aOwn}, bob's ${aForeign}; bob dir: own ${bOwn}, alice's ${bForeign}`, severity: "high" });
      withDb(aDb, (db) => ctx.result("D-scopes-alice", db.scopes()));
      withDb(bDb, (db) => ctx.result("D-scopes-bob", db.scopes()));
    }

    await rig.start();
    await rig.signIn("storscen-alice");
    const back = await sees(rig, alice);
    ctx.check({ id: "D-switch-back", ok: back.cron && back.thread && back.note, expected: "signing alice back in shows her records again", actual: json(back) });
    ctx.note("D-scope-coverage", "Agent and module principal scopes need an agent turn or a module storage grant (none declared); not driven. SaaS tenants need the SaaS server mode; not driven.");
    await rig.stop();
  },
};

// ---------------------------------------------------------------------------
// E + F. legacy import, then layout v2
// ---------------------------------------------------------------------------

const legacyImport = {
  id: "E-legacy-import",
  title: "A v1 workspace is imported once, idempotently, originals untouched",
  drivers: ["sqlite"],
  async run(ctx) {
    const rig = await ctx.isolated("E");
    const user = "storscen-legacy";
    const token = `storscen-legacy-secret-${randomBytes(12).toString("hex")}`;
    rig.addSecret(token);
    const { profile, workspace } = await seedV1Profile({ oh: rig.oh, userId: user, baseConfig: rig.baseConfig(), providerToken: token });
    ctx.state.legacy = { rig, user, token, profile, workspace };
    const before = await treeDigest(workspace);
    const authBefore = await fsp.readFile(path.join(profile, "auth-profiles.json"));

    await rig.start();
    const exp = await tinyflowsExpected();
    const cronIds = exp.cron.jobs.map(expectedJobId);
    const jobs = jobsOf(await rig.tryRpc("openhuman.cron_list", {}));
    const missingJobs = exp.cron.jobs.filter((e) => !jobs.some((j) => j.id === expectedJobId(e)));
    ctx.check({ id: "E-cron", ok: missingJobs.length === 0, expected: `cron_list has the ${cronIds.length} legacy jobs: shell and agent jobs keep their ids, a flow-schedule job is keyed flow:<flow id> (tinyflows-drivers schedule/jobs.rs flow_job_id)`, actual: missingJobs.length ? `missing ${missingJobs.map((e) => `${expectedJobId(e)} (${e.name})`).join(", ")}; listed: ${jobs.map((j) => `${j.id} (${j.name})`).join(", ").slice(0, 400)}` : "all listed" });
    const notifs = await rig.tryRpc("openhuman.notification_list", { limit: 100 });
    ctx.check({ id: "E-notifications", ok: json(notifs.value).includes('"n-1"'), expected: "notification_list has legacy notification n-1", actual: notifs.ok ? json(notifs.value).slice(0, 200) : errText(notifs) });
    const decisions = await rig.tryRpc("openhuman.approval_list_recent_decisions", { limit: 100 });
    ctx.check({ id: "E-approvals", ok: json(decisions.value).includes("already-approved"), expected: "approval_list_recent_decisions has the legacy decided request", actual: decisions.ok ? json(decisions.value).slice(0, 200) : errText(decisions) });
    if (rig.has("openhuman.flows_list")) {
      const flows = json((await rig.tryRpc("openhuman.flows_list", {})).value);
      ctx.check({ id: "E-flows", ok: flows.includes("flow-alpha") && flows.includes("flow-beta"), expected: "flows_list has flow-alpha and flow-beta", actual: flows.slice(0, 200) });
    }
    const usage = await rig.tryRpc("openhuman.cost_get_usage_log", { days: 30, limit: 100 });
    const costHits = LEGACY_EXPECT.costs.filter((id) => json(usage.value).includes(id)).length;
    ctx.check({ id: "E-costs", ok: costHits === LEGACY_EXPECT.costs.length, expected: `cost_get_usage_log lists the ${LEGACY_EXPECT.costs.length} legacy cost records`, actual: usage.ok ? `${costHits} listed` : errText(usage) });
    const creds = await rig.tryRpc("openhuman.auth_list_provider_credentials", {});
    ctx.check({ id: "E-auth-profile", ok: json(creds.value).includes(LEGACY_EXPECT.provider), expected: "the legacy auth profile is listed (adopted on first read)", actual: creds.ok ? json(creds.value).slice(0, 200) : errText(creds) });

    await rig.stop();
    const after = await treeDigest(workspace);
    // Only the seeded originals count: the core may add files of its own
    // (its storage directory, built-in flow skills) beside them.
    const changed = Object.keys(before).filter(
      (k) => before[k] !== after[k] && !k.endsWith("-shm") && !k.endsWith("-wal") && k !== "state/costs.jsonl",
    );
    ctx.check({ id: "E-originals-untouched", ok: changed.length === 0, expected: `every one of the ${Object.keys(before).length} seeded legacy files is byte-identical after import (cost ledger aside)`, actual: changed.length ? changed.slice(0, 8).map((k) => `${k}${after[k] ? " (modified)" : " (removed)"}`).join(", ") : "identical", severity: "high" });
    ctx.check({ id: "E-costs-renamed", ok: !exists(path.join(workspace, "state/costs.jsonl")) && exists(path.join(workspace, "state/costs.jsonl.migrated")), expected: "state/costs.jsonl renamed to costs.jsonl.migrated", actual: fs.readdirSync(path.join(workspace, "state")).join(", ") });
    ctx.check({ id: "E-auth-file-untouched", ok: (await fsp.readFile(path.join(profile, "auth-profiles.json"))).equals(authBefore), expected: "auth-profiles.json left in place unchanged (adoption copies, never deletes)", actual: "compared bytes" });

    const dbFile = path.join(workspace, "storage", "storage.db");
    ctx.check({ id: "E-db-location", ok: exists(dbFile), expected: "the v1 default database is <workspace>/storage/storage.db", actual: (await rig.dbFiles()).map((p) => path.relative(rig.oh, p)).join(", ") });
    if (!exists(dbFile)) return;
    const first = withDb(dbFile, (db) => ({ census: db.census(), markers: markerIds(db) }));
    ctx.result("E-census", first.census);
    // A finished import leaves `done/storage/storage_import/<domain>@<digest>`;
    // `claim/...` is the lease taken while it ran.
    const missing = LEGACY_EXPECT.markerDomains.filter((d) => !first.markers.some((m) => m.startsWith("done/") && m.includes(`storage_import/${d}@`)));
    ctx.check({ id: "E-markers", ok: missing.length === 0, expected: `an import marker per domain (${LEGACY_EXPECT.markerDomains.join(", ")})`, actual: `markers: ${first.markers.join(", ") || "none"}; missing ${missing.join(", ") || "none"}` });
    const leaked = await grepTree(path.join(workspace, "storage"), token);
    ctx.check({ id: "E-secret-encrypted", ok: leaked.length === 0, expected: "the legacy plaintext token is not in the database after adoption", actual: `${leaked.length} files`, severity: "high" });

    // A second boot imports nothing new.
    await rig.start();
    await rig.stop();
    const second = withDb(dbFile, (db) => ({ census: db.census(), markers: markerIds(db) }));
    // The imported domains' own collections in the tenant scope; storage
    // bookkeeping (tsd.*, leases, journals) and module scopes move on their own.
    const imported = (r) => r.scope === "local" && !/^tsd\.|journal|leases|admission|^storage\./.test(r.coll);
    // Both directions: a collection emptied by the second boot is absent
    // from its census and must still count.
    const count = (census, r) => census.find((x) => x.scope === r.scope && x.coll === r.coll)?.n ?? 0;
    const delta = [...first.census.filter(imported), ...second.census.filter(imported)]
      .filter((r, i, all) => all.findIndex((x) => x.scope === r.scope && x.coll === r.coll) === i)
      .filter((r) => count(first.census, r) !== count(second.census, r))
      .map((r) => ({ scope: r.scope, coll: r.coll, first: count(first.census, r), second: count(second.census, r) }));
    const legacyMarkers = (m) => m.filter((x) => x.startsWith("done/") && x.includes("storage_import/")).sort();
    ctx.check({ id: "E-idempotent", ok: delta.length === 0 && json(legacyMarkers(second.markers)) === json(legacyMarkers(first.markers)), expected: "a second boot adds no imported record and no import marker", actual: delta.length ? json(delta).slice(0, 300) : `unchanged; ${legacyMarkers(second.markers).length} markers` });
    const other = second.census.filter((r) => !imported(r)).filter((r) => (first.census.find((x) => x.scope === r.scope && x.coll === r.coll)?.n ?? 0) !== r.n);
    if (other.length) ctx.note("E-bookkeeping-delta", `second boot changed storage bookkeeping (not imported records): ${json(other).slice(0, 400)}`);
    ctx.state.legacy.census = first.census;
  },
};

const layoutMigration = {
  id: "F-layout-migration",
  needs: ["E-legacy-import"],
  title: "Layout v2 opt-in migration: archive, data intact; no master key leaves credentials in place (#8076)",
  drivers: ["sqlite"],
  async run(ctx) {
    const L = ctx.state.legacy;
    if (!L) {
      ctx.note("F-skipped", "E did not run, so there is no v1 profile to migrate.");
      return;
    }
    const { rig, user, profile } = L;
    await rig.editConfig(path.join(profile, "config.toml"), "storage", { auto_migrate_layout: true });
    await rig.start();
    const status = await rig.tryRpc("openhuman.storage_status", {});
    ctx.check({ id: "F-status", ok: json(status.value).includes('"layout_version":2') || pick(status.value, "layout_version", "result.layout_version") === 2, expected: "storage_status reports layout_version 2", actual: status.ok ? json(status.value).slice(0, 200) : errText(status) });
    const jobs = jobsOf(await rig.tryRpc("openhuman.cron_list", {}));
    const cronIds = (await tinyflowsExpected()).cron.jobs.map(expectedJobId);
    ctx.check({ id: "F-data-intact-rpc", ok: cronIds.every((id) => jobs.some((j) => j.id === id)), expected: "the imported cron jobs are still listed after the move", actual: `${jobs.filter((j) => cronIds.includes(j.id)).length}/${cronIds.length}` });
    const creds = json((await rig.tryRpc("openhuman.auth_list_provider_credentials", {})).value);
    ctx.check({ id: "F-credential-intact", ok: creds.includes(LEGACY_EXPECT.provider), expected: "the legacy credential is still listed after the move", actual: creds.slice(0, 200) });
    await rig.stop();

    const marker = path.join(profile, ".layout");
    ctx.check({ id: "F-marker", ok: exists(marker) && fs.readFileSync(marker, "utf8").trim() === "2", expected: "users/<id>/.layout reads 2", actual: exists(marker) ? fs.readFileSync(marker, "utf8").trim() : "absent" });
    ctx.check({ id: "F-files", ok: exists(path.join(profile, "storage.db")) && exists(path.join(profile, "files")), expected: "users/<id>/storage.db and files/ exist", actual: fs.readdirSync(profile).join(", ") });
    const archives = (await findAll(rig.oh, (n) => n.endsWith(".tar.zst")));
    ctx.check({ id: "F-backup", ok: archives.length > 0 && archives.every((p) => fs.statSync(p).size > 0), expected: "a non-empty backups/<user>-<ts>.tar.zst exists", actual: archives.map((p) => `${path.relative(rig.oh, p)} (${fs.statSync(p).size} B)`).join(", ") || "none" });
    if (exists(path.join(profile, "storage.db")) && L.census) {
      const now = withDb(path.join(profile, "storage.db"), (db) => db.census());
      const lost = L.census.filter((r) => {
        const n = now.find((x) => x.scope === r.scope && x.coll === r.coll)?.n ?? 0;
        return n < r.n;
      });
      ctx.check({ id: "F-counts", ok: lost.length === 0, expected: "every (scope, collection) has at least as many records as before the move", actual: lost.length ? json(lost).slice(0, 300) : "no collection shrank", severity: "high" });
    }
    ctx.check({ id: "F-credential-file-archived", ok: !exists(path.join(profile, "auth-profiles.json")), expected: "with a master key, auth-profiles.json is imported, archived and removed", actual: exists(path.join(profile, "auth-profiles.json")) ? "still in place" : "removed" });

    // F2 (#8076): the same move with no resolvable master key.
    const nokey = await ctx.isolated("F2", { masterKey: false });
    const token = `storscen-nokey-secret-${randomBytes(12).toString("hex")}`;
    nokey.addSecret(token);
    const seeded = await seedV1Profile({ oh: nokey.oh, userId: user, baseConfig: nokey.baseConfig(), providerToken: token });
    await nokey.editConfig(path.join(seeded.profile, "config.toml"), "storage", { auto_migrate_layout: true });
    const authBefore = await fsp.readFile(path.join(seeded.profile, "auth-profiles.json"));
    const boot = await nokey.start().then(() => "booted", (e) => `boot failed: ${e.message}`);
    await nokey.stop();
    const authFile = path.join(seeded.profile, "auth-profiles.json");
    const kept = exists(authFile) && (await fsp.readFile(authFile)).equals(authBefore);
    ctx.check({ id: "F2-credentials-left-in-place", ok: kept, expected: "with no master key the credential file is not imported and stays byte-identical (#8076)", actual: `${boot}; auth-profiles.json ${exists(authFile) ? (kept ? "unchanged" : "CHANGED") : "GONE"}`, severity: "high" });
    const leak = await grepTree(path.join(nokey.oh, "users", user), token);
    const leakDb = leak.filter((p) => p.endsWith(".db") || p.endsWith("-wal"));
    ctx.check({ id: "F2-no-plaintext-in-db", ok: leakDb.length === 0, expected: "the plaintext token is in no database file", actual: leakDb.map((p) => path.relative(nokey.oh, p)).join(", ") || "none", severity: "high" });
    const layout = path.join(seeded.profile, ".layout");
    ctx.note("F2-layout", `no-master-key profile after boot: .layout ${exists(layout) ? fs.readFileSync(layout, "utf8").trim() : "absent"}; entries: ${fs.readdirSync(seeded.profile).join(", ")}`);
  },
};

/** A legacy job's id after import: a flow's schedule job is keyed by its flow. */
const expectedJobId = (job) => (job.job_type === "flow" ? `flow:${job.command}` : job.id);

/**
 * Ids of the `storage.import_state` documents, decoded: tsd stores a key as
 * `/`-joined hex segments (`claim/73746f72616765/...`), the step id is inside.
 */
function markerIds(db) {
  const hex = (seg) => (/^([0-9a-f]{2})+$/.test(seg) ? Buffer.from(seg, "hex").toString("utf8") : seg);
  return db.docs("storage.import_state").filter((d) => d.scope === "local").map((d) => d.id.split("/").map(hex).join("/"));
}

async function findAll(root, pred) {
  const out = [];
  async function walk(dir) {
    for (const e of await fsp.readdir(dir, { withFileTypes: true }).catch(() => [])) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) await walk(p);
      else if (pred(e.name)) out.push(p);
    }
  }
  await walk(root);
  return out;
}

// ---------------------------------------------------------------------------
// G. Clear App Data (#8248)
// ---------------------------------------------------------------------------

const clearAppData = {
  id: "G-clear-app-data",
  title: "Clear App Data removes only the current user's folder",
  drivers: ["sqlite"],
  async run(ctx) {
    const rig = await ctx.isolated("G");
    await rig.start();
    await rig.signIn("storscen-g2");
    await writeUserData(rig, "storscen-g2", ctx.tag);
    await rig.signIn("storscen-g1");
    await writeUserData(rig, "storscen-g1", ctx.tag);
    await rig.stop();
    const g1 = path.join(rig.oh, "users", "storscen-g1");
    const g2 = path.join(rig.oh, "users", "storscen-g2");
    const g2Before = await treeDigest(g2);
    ctx.check({ id: "G-control", ok: exists(g1) && Object.keys(g2Before).length > 0, expected: "both user folders exist before the reset (control)", actual: `g1 ${exists(g1)}, g2 ${Object.keys(g2Before).length} files` });
    await rig.start();
    const reset = await rig.tryRpc("openhuman.config_reset_local_data", {});
    ctx.check({ id: "G-reset", ok: reset.ok, expected: "config_reset_local_data succeeds", actual: reset.ok ? json(reset.value).slice(0, 200) : errText(reset) });
    await rig.stop();
    ctx.check({ id: "G-current-gone", ok: !exists(g1), expected: "users/storscen-g1 (the active user) is removed", actual: exists(g1) ? `still there: ${fs.readdirSync(g1).join(", ")}` : "removed", severity: "high" });
    const g2After = await treeDigest(g2, { skip: (r) => r.endsWith("-shm") });
    const g2BeforeNoShm = Object.fromEntries(Object.entries(g2Before).filter(([k]) => !k.endsWith("-shm")));
    ctx.check({ id: "G-other-untouched", ok: json(g2After) === json(g2BeforeNoShm), expected: "users/storscen-g2 is byte-identical", actual: json(g2After) === json(g2BeforeNoShm) ? "identical" : "CHANGED", severity: "high" });
    ctx.check({ id: "G-active-user-cleared", ok: !exists(path.join(rig.oh, "active_user.toml")), expected: "active_user.toml removed", actual: exists(path.join(rig.oh, "active_user.toml")) ? "still there" : "removed" });

    // G2: an explicit backend is not a folder; what survives a reset is reported.
    const shared = await ctx.isolated("G2", { url: "sqlite:<home>/shared-storage" });
    await shared.start();
    await shared.signIn("storscen-g1");
    const d = await writeUserData(shared, "storscen-g1", ctx.tag);
    const control = await sees(shared, d);
    await shared.tryRpc("openhuman.config_reset_local_data", {});
    await shared.restart();
    await shared.signIn("storscen-g1");
    const after = await sees(shared, d);
    ctx.note("G2-explicit-backend", `explicit OPENHUMAN_STORAGE_URL: before reset ${json(control)}; after reset and sign-in again ${json(after)}. The reset removes the user folder only; records on an explicit backend are outside it.`);
    await shared.stop();
  },
};

// ---------------------------------------------------------------------------
// J. #8366: a session sign-out keeps the API key
// ---------------------------------------------------------------------------

const signOutKeepsKey = {
  id: "J-signout-keeps-key",
  title: "Signing out of the session keeps the API key on a storage backend (#8366)",
  drivers: ["sqlite"],
  async run(ctx) {
    const rig = await ctx.isolated("J");
    await rig.start();
    await rig.signIn("storscen-j1");
    const key = `storscen-apikey-${randomBytes(16).toString("hex")}`;
    rig.addSecret(key);
    const set = await rig.tryRpc("openhuman.auth_set_credential", { token: key, kind: "api-key" });
    ctx.check({ id: "J-key-set", ok: set.ok, expected: "auth_set_credential stores the API key", actual: set.ok ? "ok" : errText(set) });
    const credOfEarly = await rig.tryRpc("openhuman.auth_get_state", {});
    const early = pick(credOfEarly.value, "credential", "state.credential", "result.credential");
    ctx.check({ id: "J-control", ok: early === "api-key", expected: "before the sign-out, auth_get_state is authenticated by the API key (control)", actual: credOfEarly.ok ? json(credOfEarly.value).slice(0, 200) : errText(credOfEarly) });
    const clear = await rig.tryRpc("openhuman.auth_clear_credential", { kind: "session" });
    ctx.check({ id: "J-signout", ok: clear.ok, expected: "auth_clear_credential {kind: session} succeeds", actual: clear.ok ? json(clear.value).slice(0, 200) : errText(clear) });
    const credOf = async () => {
      const s = await rig.tryRpc("openhuman.auth_get_state", {});
      return { auth: pick(s.value, "is_authenticated", "isAuthenticated", "state.is_authenticated", "state.isAuthenticated", "result.is_authenticated", "result.isAuthenticated"), cred: pick(s.value, "credential", "state.credential", "result.credential"), raw: s.ok ? json(s.value).slice(0, 200) : errText(s) };
    };
    const now = await credOf();
    ctx.check({ id: "J-key-kept", ok: now.auth === true && now.cred === "api-key", expected: "after the session sign-out auth_get_state is authenticated by credential \"api-key\"", actual: now.raw, severity: "high" });
    await rig.restart();
    const later = await credOf();
    ctx.check({ id: "J-key-kept-after-restart", ok: later.auth === true && later.cred === "api-key", expected: "the API key is still in effect after a restart", actual: later.raw, severity: "high" });
    await rig.stop();
    const hits = await grepTree(rig.home, key);
    ctx.check({ id: "J-key-not-plaintext", ok: hits.length === 0, expected: "the API key appears nowhere under HOME in plaintext", actual: hits.map((p) => path.relative(rig.home, p)).join(", ") || "none", severity: "high" });
  },
};

export const ISOLATED = [isolation, legacyImport, layoutMigration, clearAppData, signOutKeepsKey];
