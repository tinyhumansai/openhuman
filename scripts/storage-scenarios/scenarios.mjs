// The storage scenarios that run on the driver's own core: A (write and read
// back per port), B (durability), C (on-disk truth), H (the memory driver
// keeps nothing) and I (concurrency). The ones that need a HOME of their own
// (D, E, F, G, J) are in isolated.mjs.
//
// `ctx` (built by run.mjs): driver ("sqlite" | "file" | "memory"), tag,
// state (shared between scenarios of one driver), rpc(), tryRpc(), has(),
// check(), note(), signIn(), start/stop/restart/kill9(), home, oh,
// storageDir (file driver), dbFiles(), isolated().

import fsp from "node:fs/promises";
import path from "node:path";
import { randomBytes } from "node:crypto";
import { pick, sleep, grepTree, withDb, sha256, listOf, filesWithBytes } from "./lib.mjs";
import { tinyflowsExpected } from "./legacy.mjs";

const FAR_FUTURE = "2099-01-01T00:00:00Z";
const json = (v) => JSON.stringify(v ?? null);
const errText = (r) => (r.ok ? "" : json(r.error));

/** The A records' contents, built once per driver. */
function fixtures(ctx) {
  const t = ctx.tag;
  const big = `storscen-${t}-msg2 ` + "x".repeat(65_536);
  const contents = [
    `storscen-${t}-msg0 plain ascii`,
    `storscen-${t}-msg1 Zoë — 日本語 — emoji 🚀 — RTL שלום`,
    big,
    `storscen-${t}-msg3 {"looks":"like json","n":[1,2,3],"nested":{"q":"\\"quoted\\""}}`,
    `storscen-${t}-msg4 line one\nline two\r\n\ttab, back\\slash, NUL-free`,
    ...Array.from({ length: 15 }, (_, i) => `storscen-${t}-msg${i + 5} #${i + 5}`),
  ];
  return {
    sessionUser: `storscen-a-${t}`,
    cronName: `storscen-cron-${t}`,
    cronPrompt: `storscen fixture prompt ${t}: this job never runs`,
    notifTitle: `storscen notification ${t}`,
    notifBody: `Body for ${t}: café ☕ <b>not html</b>`,
    flowTrustId: `storscen-flow-${t}`,
    flowTrustTool: `storscen_tool_${t}`,
    threadId: `storscen-thread-${t}`,
    messages: contents.map((content, i) => ({ id: `storscen-${t}-m${i}`, content })),
    noteId: `storscen_${t}`,
    noteBytes: randomBytes(4096),
    provider: `storscen-provider-${t}`,
    providerToken: `storscen-provider-secret-${randomBytes(16).toString("hex")}`,
  };
}

/** Read every A record back; `label` prefixes the check ids (A, B1, ...). */
async function readBackA(ctx, label) {
  const a = ctx.state.a;
  const out = {};
  if (a.cronId) {
    const r = await ctx.tryRpc("openhuman.cron_list", {});
    const jobs = listOf(r.value, "jobs");
    const job = jobs.find((j) => j.id === a.cronId);
    out.cron = ctx.check({
      id: `${label}-cron`,
      ok: !!job && job.name === a.cronName && job.prompt === a.cronPrompt,
      expected: `cron_list has job ${a.cronId} with its name and prompt unchanged`,
      actual: job ? json({ name: job.name, prompt: job.prompt }) : `not listed (${errText(r) || `${jobs.length} jobs`})`,
    });
  }
  if (a.notifId) {
    const r = await ctx.tryRpc("openhuman.notification_list", { provider: "gmail", limit: 200 });
    const items = listOf(r.value, "items");
    const n = items.find((x) => x.id === a.notifId);
    out.notif = ctx.check({
      id: `${label}-notification`,
      ok: !!n && n.title === a.notifTitle && n.body === a.notifBody,
      expected: "notification_list returns the notification with title and body byte-exact",
      actual: n ? json({ title: n.title, body: n.body }) : `not listed (${errText(r) || `${items.length} items`})`,
    });
  }
  if (a.flowTrustGranted) {
    const r = await ctx.tryRpc("openhuman.approval_list_recent_decisions", { limit: 200 });
    out.approval = ctx.check({
      id: `${label}-approval`,
      ok: r.ok && listOf(r.value, "decisions").some((d) => d.tool_name === a.flowTrustTool && d.decision === "approve_always_for_flow"),
      expected: `approval_list_recent_decisions has an approve_always_for_flow decision for tool ${a.flowTrustTool} (rows carry the tool, not the flow id)`,
      actual: r.ok ? json(r.value).slice(0, 300) : errText(r),
    });
  }
  if (a.appended === 0) {
    // Nothing was accepted (A-thread-append is the finding); a read-back adds nothing.
  } else {
    const r = await ctx.tryRpc("openhuman.threads_messages_list", { thread_id: a.threadId });
    const msgs = listOf(r.value, "messages");
    const ours = msgs.filter((m) => String(m.id).startsWith(`storscen-${ctx.tag}-m`));
    const bad = a.messages.filter((m, i) => ours[i]?.id !== m.id || ours[i]?.content !== m.content);
    out.stream = ctx.check({
      id: `${label}-stream`,
      ok: r.ok && ours.length === a.messages.length && bad.length === 0,
      expected: `threads_messages_list returns all ${a.messages.length} messages in order, byte-exact (incl. 64 KiB, unicode, JSON-ish, CR/LF)`,
      actual: r.ok
        ? `${ours.length} returned; mismatched: ${bad.map((m) => m.id).join(",") || "none"}`
        : errText(r),
    });
  }
  if (a.noteStored) {
    const r = await ctx.tryRpc("openhuman.channel_web_voice_note_read", { note_id: a.noteId });
    const b64 = pick(r.value, "audio_base64", "result.audio_base64");
    const got = b64 ? Buffer.from(b64, "base64") : null;
    out.blob = ctx.check({
      id: `${label}-blob`,
      ok: !!got && got.equals(a.noteBytes),
      expected: "channel_web_voice_note_read returns the 4096 stored bytes exactly",
      actual: got ? `${got.length} bytes, sha256 ${sha256(got).slice(0, 16)} (want ${sha256(a.noteBytes).slice(0, 16)})` : `no bytes (${errText(r) || json(r.value)})`,
    });
  }
  {
    const r = await ctx.tryRpc("openhuman.auth_list_provider_credentials", { provider: a.provider });
    const list = pick(r.value, "profiles", "result.profiles", "result") ?? r.value;
    const p = (Array.isArray(list) ? list : []).find((x) => x.provider === a.provider);
    out.secret = ctx.check({
      id: `${label}-secret-listed`,
      ok: !!p && (p.hasToken === true || p.has_token === true),
      expected: "auth_list_provider_credentials lists the provider with hasToken (masked read-back)",
      actual: p ? json(p) : `not listed (${errText(r) || json(r.value).slice(0, 200)})`,
    });
  }
  {
    const r = await ctx.tryRpc("openhuman.auth_get_session_token", {});
    const tok = typeof r.value === "string" ? r.value : pick(r.value, "token", "result.token", "session_token");
    out.session = ctx.check({
      id: `${label}-secret-roundtrip`,
      ok: tok === a.sessionToken,
      expected: "auth_get_session_token returns the stored session token exactly (decrypted read-back)",
      actual: r.ok ? (tok === a.sessionToken ? "exact" : `different value (${tok ? `${String(tok).length} chars` : "none"})`) : errText(r),
      severity: "high",
    });
  }
  return out;
}

/** Secret document ids per scope, hex segments decoded (`secrets` collection). */
function secretIds(db) {
  const hex = (seg) => (/^([0-9a-f]{2})+$/.test(seg) ? Buffer.from(seg, "hex").toString("utf8") : seg);
  const out = {};
  for (const d of db.docs("secrets")) (out[d.scope] ??= []).push(d.id.split("/").map(hex).join("/"));
  return out;
}

// ---------------------------------------------------------------------------
// A. write -> read back per port
// ---------------------------------------------------------------------------

const writeRead = {
  id: "A-write-read",
  title: "Write, then read back, on every port through the real RPCs",
  drivers: ["sqlite", "file", "memory"],
  async run(ctx) {
    const a = (ctx.state.a = fixtures(ctx));
    ctx.addSecret(a.providerToken);

    // A0: a signed-in user, so every record lives in that user's profile.
    a.sessionToken = await ctx.signIn(a.sessionUser);

    // Documents: a cron job, a notification, a flow pre-authorisation.
    const cron = await ctx.tryRpc("openhuman.cron_add", {
      name: a.cronName,
      schedule: { kind: "at", at: FAR_FUTURE },
      job_type: "agent",
      prompt: a.cronPrompt,
      delete_after_run: false,
    });
    a.cronId = pick(cron.value, "job.id", "result.job.id", "id");
    ctx.check({
      id: "A-cron-add",
      ok: !!a.cronId,
      expected: "cron_add stores a job and returns its id",
      actual: a.cronId ?? errText(cron),
    });
    const notif = await ctx.tryRpc("openhuman.notification_ingest", {
      provider: "gmail",
      account_id: `storscen-${ctx.tag}`,
      title: a.notifTitle,
      body: a.notifBody,
      raw_payload: { storscen: ctx.tag, list: [1, 2, 3] },
    });
    a.notifId = pick(notif.value, "id", "result.id");
    ctx.check({
      id: "A-notification-ingest",
      ok: !!a.notifId,
      expected: "notification_ingest stores the notification (not skipped)",
      actual: a.notifId ?? (notif.ok ? json(notif.value) : errText(notif)),
    });
    const trust = await ctx.tryRpc("openhuman.approval_preauthorize_flow", {
      flow_id: a.flowTrustId,
      tool_names: [a.flowTrustTool],
    });
    a.flowTrustGranted = trust.ok;
    ctx.check({
      id: "A-approval-preauthorize",
      ok: trust.ok && pick(trust.value, "gate_installed", "result.gate_installed") !== false,
      expected: "approval_preauthorize_flow grants trust with the approval gate installed",
      actual: trust.ok ? json(trust.value) : errText(trust),
    });

    // Stream: a thread with 20 messages.
    const up = await ctx.tryRpc("openhuman.threads_upsert", {
      id: a.threadId,
      title: `storscen thread ${ctx.tag}`,
      created_at: new Date().toISOString(),
    });
    ctx.check({ id: "A-thread-upsert", ok: up.ok, expected: "threads_upsert creates the thread", actual: up.ok ? "ok" : errText(up) });
    let appended = 0;
    for (const m of a.messages) {
      const r = await ctx.tryRpc("openhuman.threads_message_append", {
        thread_id: a.threadId,
        message: { id: m.id, content: m.content, type: "text", extraMetadata: { storscen: ctx.tag }, sender: "user", createdAt: new Date().toISOString() },
      });
      if (r.ok) appended += 1;
      else ctx.state.appendError ??= errText(r);
    }
    a.appended = appended;
    ctx.check({
      id: "A-thread-append",
      ok: appended === a.messages.length,
      expected: `all ${a.messages.length} threads_message_append calls succeed`,
      actual: `${appended} succeeded${ctx.state.appendError ? `; first error ${ctx.state.appendError}` : ""}`,
    });

    // Blob: a voice note.
    const note = await ctx.tryRpc("openhuman.channel_web_voice_note_store", {
      note_id: a.noteId,
      audio_base64: a.noteBytes.toString("base64"),
      mime: "audio/webm",
    });
    a.noteStored = note.ok;
    ctx.check({ id: "A-blob-store", ok: note.ok, expected: "channel_web_voice_note_store stores the bytes", actual: note.ok ? json(note.value) : errText(note) });

    // Secret: a provider credential.
    const cred = await ctx.tryRpc("openhuman.auth_store_provider_credentials", {
      provider: a.provider,
      profile: "default",
      token: a.providerToken,
      fields: { region: "eu-fictional" },
      setActive: true,
    });
    ctx.check({ id: "A-secret-store", ok: cred.ok, expected: "auth_store_provider_credentials stores the credential", actual: cred.ok ? "ok" : errText(cred) });

    ctx.note("A-kv", "KV port not driven: no RPC writes tsd.kv without a bound memory engine (memory state is its only user; app state is a file on purpose). C lists the KV rows the run produced.");

    await readBackA(ctx, "A");
  },
};

// ---------------------------------------------------------------------------
// B. durability
// ---------------------------------------------------------------------------

const durability = {
  id: "B-durability",
  title: "Restart keeps everything; kill -9 mid-write leaves no torn records",
  needs: ["A-write-read"],
  drivers: ["sqlite", "file"],
  async run(ctx) {
    await ctx.restart();
    await readBackA(ctx, "B1");

    // B2: appends in flight when the core is SIGKILLed.
    const thread = `storscen-crash-${ctx.tag}`;
    await ctx.rpc("openhuman.threads_upsert", { id: thread, title: "crash", created_at: new Date().toISOString() });
    const body = (i) => `storscen-${ctx.tag}-crash${i} ` + "y".repeat(2048 + i);
    const acked = new Set();
    const attempted = new Set();
    let next = 0;
    let killed = false;
    const worker = async () => {
      while (!killed && next < 400) {
        const i = next++;
        const id = `crash-${ctx.tag}-${i}`;
        attempted.add(id);
        const r = await ctx.tryRpc("openhuman.threads_message_append", {
          thread_id: thread,
          message: { id, content: body(i), type: "text", extraMetadata: {}, sender: "user", createdAt: new Date().toISOString() },
        }, 10_000);
        if (r.ok) acked.add(id);
      }
    };
    const workers = Array.from({ length: 4 }, worker);
    while (acked.size < 40 && next < 400) await sleep(5);
    if (acked.size < 40) {
      // Without enough acknowledged appends there is no in-flight window to
      // test, and the loss and tear checks below would pass on nothing.
      await Promise.allSettled(workers);
      ctx.check({ id: "B2-in-flight-window", ok: false, expected: "at least 40 appends are acknowledged before the kill", actual: `${acked.size} acknowledged after ${attempted.size} attempts`, severity: "high" });
      return;
    }
    killed = true;
    await ctx.kill9();
    await Promise.allSettled(workers);
    ctx.result("crash", { attempted: attempted.size, acked_before_kill: acked.size });

    await ctx.start();
    const r = await ctx.tryRpc("openhuman.threads_messages_list", { thread_id: thread });
    const msgs = listOf(r.value, "messages");
    const ids = msgs.map((m) => m.id);
    const lost = [...acked].filter((id) => !ids.includes(id));
    const torn = msgs.filter((m) => {
      const i = Number(String(m.id).split("-").pop());
      return m.content !== body(i);
    });
    const dup = ids.length - new Set(ids).size;
    const phantom = ids.filter((id) => !attempted.has(id));
    ctx.check({ id: "B2-no-acked-loss", ok: r.ok && lost.length === 0, expected: "every append acked before the kill is present after reboot", actual: r.ok ? `${acked.size} acked, ${lost.length} lost${lost.length ? `: ${lost.slice(0, 5)}` : ""}` : errText(r), severity: "high" });
    ctx.check({ id: "B2-no-torn", ok: r.ok && torn.length === 0 && dup === 0 && phantom.length === 0, expected: "no torn, duplicated or phantom message after the kill", actual: `${msgs.length} present; torn ${torn.length}, duplicates ${dup}, phantom ${phantom.length}`, severity: "high" });

    await ctx.stop();
    if (ctx.driver === "sqlite") {
      for (const db of await ctx.dbFiles()) {
        const res = withDb(db, (d) => d.integrity());
        ctx.check({ id: `B2-integrity-${path.basename(path.dirname(db))}`, ok: res.length === 1 && res[0] === "ok", expected: "PRAGMA integrity_check = ok after the kill", actual: res.join("; ").slice(0, 300), evidence: db, severity: "high" });
      }
    } else {
      // The file driver's records are JSON / JSONL: every one must parse.
      const broken = [];
      async function walk(dir) {
        for (const e of await fsp.readdir(dir, { withFileTypes: true }).catch(() => [])) {
          const p = path.join(dir, e.name);
          if (e.isDirectory()) await walk(p);
          else if (p.endsWith(".json")) {
            try { JSON.parse(await fsp.readFile(p, "utf8")); } catch { broken.push(p); }
          } else if (p.endsWith(".jsonl")) {
            for (const line of (await fsp.readFile(p, "utf8")).split("\n").filter(Boolean))
              try { JSON.parse(line); } catch { broken.push(p); break; }
          }
        }
      }
      await walk(ctx.storageDir);
      ctx.check({ id: "B2-file-records-parse", ok: broken.length === 0, expected: "every JSON/JSONL record under the file store parses after the kill", actual: `${broken.length} broken${broken.length ? `: ${broken.slice(0, 3).map((p) => path.relative(ctx.storageDir, p))}` : ""}`, severity: "high" });
    }
    await ctx.start();
  },
};

// ---------------------------------------------------------------------------
// C. on-disk truth
// ---------------------------------------------------------------------------

const onDisk = {
  id: "C-on-disk",
  title: "Records sit in the expected scope and collection; secrets are not plaintext",
  needs: ["A-write-read"],
  drivers: ["sqlite", "file"],
  async run(ctx) {
    const a = ctx.state.a;
    await ctx.stop();
    const marker0 = a.messages[0].content.split(" ")[0];
    // Positive control first: the grep must find a plaintext value that IS stored
    // (the cron job's prompt, a plain document field on every driver).
    const control = await grepTree(ctx.home, a.cronPrompt);
    ctx.check({ id: "C-grep-control", ok: control.length > 0, expected: "the plaintext grep finds the stored cron prompt (positive control)", actual: `${control.length} files` });
    for (const [what, secret] of [["provider token", a.providerToken], ["session token", a.sessionToken]]) {
      const hits = await grepTree(ctx.home, secret);
      ctx.check({
        id: `C-no-plaintext-${what.replace(" ", "-")}`,
        ok: control.length > 0 && hits.length === 0,
        expected: `the ${what} appears nowhere under HOME in plaintext`,
        actual: control.length === 0 ? "control failed: grep proves nothing" : `${hits.length} files${hits.length ? `: ${hits.map((p) => path.relative(ctx.home, p)).join(", ")}` : ""}`,
        severity: "high",
      });
    }

    if (ctx.driver === "sqlite") {
      const dbs = await ctx.dbFiles();
      // Which database holds which secret: a session written before the
      // profile's backend is bound would sit in another file.
      for (const f of dbs)
        ctx.note(`C-secrets-${path.basename(path.dirname(f))}`, `${path.relative(ctx.oh, f)}: ${json(withDb(f, secretIds))}`);
      const own = dbs.filter((p) => p.includes(`${path.sep}users${path.sep}${a.sessionUser}${path.sep}`));
      ctx.check({ id: "C-db-location", ok: own.length === 1, expected: `one storage.db under users/${a.sessionUser}/ (layout v2 profile)`, actual: dbs.map((p) => path.relative(ctx.oh, p)).join(", ") || "none" });
      const file = own[0] ?? dbs[0];
      if (file) {
        withDb(file, (db) => {
          const where = (coll, needle) => db.docs(coll).filter((d) => d.body.includes(needle)).map((d) => d.scope);
          const expectLocal = (id, coll, needle, why) => {
            const scopes = where(coll, needle);
            ctx.check({ id, ok: scopes.length > 0 && scopes.every((s) => s === "local"), expected: `${coll} holds it in the acting scope \`local\` (${why})`, actual: scopes.length ? scopes.join(",") : `not in ${coll}` });
          };
          expectLocal("C-cron-scope", "cron.jobs", a.cronName, "cron/documents_port.rs: DocScope::acting");
          expectLocal("C-notification-scope", "notifications.integration", a.notifTitle, "domain stores use the tenant scope");
          expectLocal("C-approval-scope", "approvals.flow_trust", a.flowTrustId, "domain stores use the tenant scope");
          const stream = db.tables.streams
            ? db.all(`SELECT scope, name FROM "${db.tables.streams}" WHERE instr(body, ?) > 0`, marker0)
            : [];
          ctx.check({ id: "C-stream-scope", ok: stream.length > 0 && stream.every((s) => s.scope === "local"), expected: "the thread's messages are stream rows in scope `local`", actual: stream.length ? json(stream[0]) : "no stream row holds the marker" });
          const blob = db.blob(`%${a.noteId}%`);
          ctx.check({ id: "C-blob", ok: blob.length === 1 && blob[0].scope === "local" && Buffer.from(blob[0].bytes).equals(a.noteBytes), expected: "the voice note is one blob row in `local` with the exact bytes", actual: blob.length ? `${blob[0].scope} ${blob[0].key} ${blob[0].size} bytes` : "no blob row" });
          const secrets = db.docs("secrets");
          const leak = secrets.filter((s) => s.body.includes(a.providerToken) || s.body.includes(a.sessionToken));
          ctx.check({ id: "C-secret-docs", ok: secrets.length > 0 && leak.length === 0, expected: "secrets are encrypted documents (collection `secrets`), no plaintext in their bodies", actual: `${secrets.length} secret docs in ${[...new Set(secrets.map((s) => s.scope))].join(",") || "-"}; ${leak.length} with plaintext` , severity: "high" });
          ctx.result("census", db.census());
          ctx.result("secret-ids", secretIds(db));
          ctx.result("scopes", db.scopes());
          ctx.note("C-census", `scopes on disk: ${db.scopes().join(", ")}; KV rows: ${db.kvRows().length}; tables: ${json(db.tables)}`);
        });
      }
    } else {
      const docHits = await grepTree(ctx.storageDir, a.cronName);
      ctx.check({ id: "C-file-cron-scope", ok: docHits.some((p) => p.includes(`${path.sep}scopes${path.sep}local${path.sep}`)), expected: "the cron job is a JSON document under <dir>/scopes/local/", actual: docHits.map((p) => path.relative(ctx.storageDir, p)).join(", ") || "not found" });
      const blobHits = await filesWithBytes(ctx.storageDir, a.noteBytes);
      ctx.check({ id: "C-file-blob", ok: blobHits.length === 1 && blobHits[0].includes(`${path.sep}scopes${path.sep}local${path.sep}`), expected: "the voice note is one raw file under <dir>/scopes/local/ holding exactly the stored bytes", actual: blobHits.map((p) => path.relative(ctx.storageDir, p)).join(", ") || "no file holds the bytes" });
    }
    await ctx.start();
  },
};

// ---------------------------------------------------------------------------
// H. the memory driver keeps nothing across a restart (by design)
// ---------------------------------------------------------------------------

const memoryRestart = {
  id: "H-memory-restart",
  title: "memory driver: everything gone after a restart, as designed",
  needs: ["A-write-read"],
  drivers: ["memory"],
  async run(ctx) {
    const a = ctx.state.a;
    await ctx.restart();
    await ctx.signIn(a.sessionUser);
    // Positive control: a record written now IS listed, so an empty answer means gone.
    const probeName = `storscen-probe-${ctx.tag}`;
    const add = await ctx.tryRpc("openhuman.cron_add", { name: probeName, schedule: { kind: "at", at: FAR_FUTURE }, job_type: "agent", prompt: "probe" });
    const list = await ctx.tryRpc("openhuman.cron_list", {});
    const jobs = listOf(list.value, "jobs");
    const control = add.ok && jobs.some((j) => j.name === probeName);
    ctx.check({ id: "H-control", ok: control, expected: "a job added after the restart is listed (positive control)", actual: control ? "listed" : errText(add) || errText(list) });
    ctx.check({ id: "H-cron-gone", ok: control && !jobs.some((j) => j.id === a.cronId), expected: "the A cron job is gone after a restart", actual: control ? (jobs.some((j) => j.id === a.cronId) ? "still listed" : "gone") : "control failed" });
    const msgs = await ctx.tryRpc("openhuman.threads_messages_list", { thread_id: a.threadId });
    const n = listOf(msgs.value, "messages").length;
    ctx.check({ id: "H-stream-gone", ok: control && msgs.ok && n === 0, expected: "the A thread has no messages after a restart", actual: msgs.ok ? `${n} messages` : `error (${errText(msgs).slice(0, 120)})` });
  },
};

// ---------------------------------------------------------------------------
// I. concurrency
// ---------------------------------------------------------------------------

const concurrency = {
  id: "I-concurrency",
  title: "Concurrent writers: compare-and-swap has one winner; parallel appends lose nothing",
  needs: ["A-write-read"],
  drivers: ["sqlite"],
  async run(ctx) {
    if (ctx.has("openhuman.flows_create") && ctx.has("openhuman.flows_update")) {
      const graph = (await tinyflowsExpected()).catalog.flows[0].graph;
      const made = await ctx.tryRpc("openhuman.flows_create", { name: `storscen-race-${ctx.tag}`, graph });
      const flow = pick(made.value, "flow", "result.flow", "result") ?? made.value;
      const id = flow?.id;
      const version = flow?.updated_at ?? flow?.updatedAt;
      ctx.check({ id: "I-flow-create", ok: !!id && !!version, expected: "flows_create returns the flow with its id and updated_at", actual: id ? `${id} @ ${version}` : errText(made) || json(made.value).slice(0, 200) });
      if (id && version) {
        const N = 8;
        const res = await Promise.all(
          Array.from({ length: N }, (_, i) => ctx.tryRpc("openhuman.flows_update", { id, name: `storscen-race-${ctx.tag}-${i}`, expected_version: version })),
        );
        const conflict = (r) => json(r.ok ? r.value : r.error).includes("version_conflict");
        const winners = res.map((r, i) => (r.ok && !conflict(r) ? i : -1)).filter((i) => i >= 0);
        const conflicts = res.filter(conflict).length;
        ctx.result("cas", { writers: N, winners: winners.length, conflicts });
        ctx.check({ id: "I-cas-one-winner", ok: winners.length === 1 && conflicts === N - 1, expected: `exactly 1 of ${N} same-version updates wins; the other ${N - 1} get version_conflict`, actual: `${winners.length} won, ${conflicts} conflicts, ${N - winners.length - conflicts} other`, severity: "high" });
        const all = await ctx.tryRpc("openhuman.flows_list", {});
        const now = json(all.value).includes(`storscen-race-${ctx.tag}-${winners[0]}"`);
        ctx.check({ id: "I-cas-final-state", ok: winners.length === 1 && now, expected: "the stored flow carries the winner's name", actual: now ? "winner's name stored" : `not found (${errText(all).slice(0, 120)})` });
        const dbs = await ctx.dbFiles();
        const scopes = dbs.length
          ? withDb(dbs.find((p) => p.includes(ctx.state.a?.sessionUser ?? "")) ?? dbs[0], (db) =>
              db.all(`SELECT scope, coll FROM "${db.tables.docs}" WHERE id = ? OR instr(body, ?) > 0`, id, id))
          : [];
        const flowRows = scopes.filter((r) => r.coll.startsWith("flows"));
        ctx.check({ id: "I-flow-scope", ok: flowRows.length > 0 && flowRows.every((r) => r.scope === "local"), expected: "the flow is stored in the acting scope `local` (flows/catalog_storage.rs binds the module's callback to the acting scope)", actual: flowRows.map((r) => `${r.scope} ${r.coll}`).join("; ") || "no flow row found" });
        if (flowRows.some((r) => r.scope.includes("/module:")))
          ctx.note("I-flow-module-scope", "flow records sit in a module principal scope");
      }
    } else {
      ctx.note("I-cas-skipped", "flows_create / flows_update are not in this build's /schema; the compare-and-swap race was not run.");
    }

    const thread = `storscen-race-thread-${ctx.tag}`;
    await ctx.rpc("openhuman.threads_upsert", { id: thread, title: "race", created_at: new Date().toISOString() });
    const N = 50;
    const res = await Promise.all(
      Array.from({ length: N }, (_, i) =>
        ctx.tryRpc("openhuman.threads_message_append", { thread_id: thread, message: { id: `race-${i}`, content: `race ${i}`, type: "text", extraMetadata: {}, sender: "user", createdAt: new Date().toISOString() } })),
    );
    const ok = res.filter((r) => r.ok).length;
    const list = await ctx.tryRpc("openhuman.threads_messages_list", { thread_id: thread });
    const ids = listOf(list.value, "messages").map((m) => m.id);
    ctx.check({ id: "I-append-race", ok: ok === N && ids.length === N && new Set(ids).size === N, expected: `${N} parallel appends to one thread: all succeed, all ${N} listed once`, actual: `${ok} succeeded, ${ids.length} listed, ${new Set(ids).size} unique`, severity: "high" });
  },
};

export const SCENARIOS = [writeRead, durability, onDisk, memoryRestart, concurrency];
