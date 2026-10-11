// The memory scenarios. Each one sets up, talks to the core (turns and RPCs),
// checks what came back and records failed checks as findings. Nothing is
// fixed during a run.
//
// `ctx` (built by run.mjs) carries: engine ("local" | "builtin"), core, turn(),
// rpc(), learn(), listAll(), waitItems(), persona, marker, ledger, findings,
// threads, logFile, setConfigToml() + restartCore() (local only), cortex
// (local only: the container handle), note().

import fsp from "node:fs/promises";
import path from "node:path";
import { randomUUID, sleep, waitFor, pick, countLogLines } from "./lib.mjs";

const isoDay = (offsetDays, tz) => {
  const d = new Date(Date.now() + offsetDays * 86_400_000);
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: tz,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(d);
};
const lc = (s) => String(s ?? "").toLowerCase();
const containsAny = (text, words) =>
  words.some((w) => lc(text).includes(lc(w)));

/** Give the timeout comparison five seconds more than the active default. */
export const extendedPreTurnWait = (defaultWait) => {
  if (
    !Number.isSafeInteger(defaultWait) ||
    defaultWait < 1 ||
    defaultWait > Number.MAX_SAFE_INTEGER - 5_000
  ) {
    throw new TypeError("pre-turn timeout must be a positive safe integer");
  }
  return defaultWait + 5_000;
};

// ---------------------------------------------------------------------------
// A. Storing
// ---------------------------------------------------------------------------

const storing = {
  id: "A-storing",
  title:
    "Storing: chats per agent, learning kinds, brain text, a file path, dated facts",
  engines: ["local", "builtin"],
  async run(ctx) {
    const { persona, check } = ctx;

    // A1: chats with two agents land under each agent.
    const t1 = ctx.newThread("A1-orchestrator");
    const r1 = await ctx.turn(
      t1,
      "Note for later: my weekend plan is to repaint the balcony railing.",
    );
    await ctx.setChatAgent("planner");
    const t2 = ctx.newThread("A1-planner");
    const r2 = await ctx.turn(
      t2,
      "Plan three steps for repainting a balcony railing.",
    );
    await ctx.setChatAgent("");
    check("A1-turns-completed", !r1.error && !r2.error, "both turns complete", {
      r1: r1.error,
      r2: r2.error,
    });
    for (const [thread, agent] of [
      [t1, "orchestrator"],
      [t2, "planner"],
    ]) {
      const items = await ctx.waitItems({ thread_id: thread }, 1, 60_000);
      const agents = [...new Set(items.map((h) => h.meta?.agent_id))];
      check(
        `A1-logged-${agent}`,
        items.length > 0,
        `the ${agent} chat is logged to memory`,
        { count: items.length },
      );
      if (items.length)
        check(
          `A1-agent-${agent}`,
          agents.includes(agent),
          `logged turns carry agent_id=${agent}`,
          { agents },
        );
    }

    // A2: each learning kind is stored, and its kind can be read back.
    for (const l of persona.learnings) {
      const id = await ctx.learn(l.text, l.kind);
      const got = await ctx.rpc("openhuman.memory_items_get", { ids: [id] });
      const hit = got?.items?.[0];
      check(
        `A2-learning-${l.kind}-stored`,
        !!hit,
        "the learning reads back by id",
        { id },
      );
      if (!hit) continue;
      const raw = JSON.stringify(hit);
      // The Hit shape has no learning-kind field; look for the kind anywhere.
      check(
        `A2-learning-${l.kind}-type-kept`,
        lc(raw).includes(`"${l.kind}"`),
        `the learning's kind (${l.kind}) is visible on the item`,
        hit,
        "medium",
        "READ",
      );
    }

    // A3: a brain text document.
    const doc = await ctx.rpc("openhuman.memory_brain_ingest", {
      text: persona.brain_doc.text,
      title: persona.brain_doc.title,
    });
    ctx.ledger.add(doc?.id);
    check("A3-brain-ingest", !!doc?.id, "a brain text document is stored", doc);

    // A4: a file whose path holds a username: only the basename may reach the engine.
    const file = path.join(ctx.fixtureCopy, persona.path_file.relative);
    const fdoc = await ctx.tryRpc("openhuman.memory_brain_ingest", {
      path: file,
    });
    if (!fdoc.ok) {
      check("A4-file-ingest", false, "a file by path is stored", fdoc.error);
    } else {
      ctx.ledger.add(fdoc.value?.id);
      const got = await ctx.rpc("openhuman.memory_items_get", {
        ids: [fdoc.value.id],
      });
      const meta = got?.items?.[0]?.meta ?? {};
      const leaked = JSON.stringify(meta).includes(persona.path_file.username);
      check(
        "A4-basename-only",
        !leaked,
        `no directory (and no username "${persona.path_file.username}") in the stored path`,
        { file_path: meta.file_path, folder: meta.folder },
        "high",
      );
    }

    // A5: dated facts (stored with the real dates they describe; C checks recall).
    ctx.dated = [];
    for (const [offset, d] of [
      [-1, persona.dated[0]],
      [-5, persona.dated[1]],
    ]) {
      const day = isoDay(offset, persona.time_zone);
      const id = await ctx.learn(d.turn, "fact", {
        observed_at: `${day}T10:00:00Z`,
      });
      ctx.dated.push({ day, id, text: d.turn });
    }

    // A6: a workflow that writes memory.
    ctx.note(
      "A6-workflow-memory",
      "not automated: creating and running a flow over RPC is out of this script's scope; checked by hand",
    );
  },
};

// ---------------------------------------------------------------------------
// B. Recall
// ---------------------------------------------------------------------------

const recall = {
  id: "B-recall",
  title:
    "Recall: cross-thread, negative control, a planted learning, brain citations, the pre-turn pack",
  engines: ["local", "builtin"],
  async run(ctx) {
    const { persona, check } = ctx;
    const quality = ctx.recallQualityChecked;

    // B1: told in one thread, asked in another.
    const tA = ctx.newThread("B1-tell");
    await ctx.turn(tA, persona.cross_thread.store);
    await ctx.waitItems({ thread_id: tA }, 1, 60_000);
    await sleep(ctx.settleMs);
    const tB = ctx.newThread("B1-ask");
    const b1 = await ctx.turn(tB, persona.cross_thread.probe);
    if (quality)
      ctx.qcheck(
        "B1-cross-thread",
        containsAny(b1.reply, persona.cross_thread.expect_any),
        "a fact told in another thread is recalled",
        b1.reply ?? b1.error,
      );

    // B2: nothing to recall, nothing invented.
    const b2 = await ctx.turn(
      ctx.newThread("B2-negative"),
      persona.negative_control.probe,
    );
    const admits =
      /(don.?t|do not|no) (know|have|record|information)|not (sure|aware)|haven.?t (told|mentioned)|no (record|memory|mention)/i.test(
        b2.reply ?? "",
      );
    ctx.qcheck(
      "B2-no-invention",
      admits,
      "admits it does not know the tortoise's name",
      b2.reply ?? b2.error,
      "high",
      "INFERRED",
    );

    // B3: a planted learning changes behaviour.
    await ctx.learn(persona.planted.text, "preference");
    await sleep(ctx.settleMs);
    const b3 = await ctx.turn(
      ctx.newThread("B3-planted"),
      persona.planted.probe,
    );
    const reply = b3.reply ?? "";
    const suggests =
      /peanut/i.test(reply) &&
      !/(no|without|avoid|skip|free of|allerg)[^.]{0,40}peanut|peanut[^.]{0,25}(free|allerg)/i.test(
        reply,
      );
    if (quality)
      ctx.qcheck(
        "B3-planted-learning",
        !suggests,
        "no peanut dish is suggested",
        reply || b3.error,
        "high",
        "INFERRED",
      );

    // B4: brain retrieval with citations.
    const rec = await ctx.tryRpc("openhuman.memory_recall", {
      question: persona.brain_doc.probe,
    });
    if (!rec.ok)
      check("B4-recall-rpc", false, "memory_recall answers", rec.error);
    else {
      const cites = rec.value?.citations ?? [];
      check(
        "B4-citations",
        cites.length > 0,
        "the answer cites stored items",
        rec.value,
      );
      if (quality)
        check(
          "B4-brain-answer",
          containsAny(rec.value?.answer, persona.brain_doc.expect_any),
          "the brain document's figure is in the answer",
          rec.value?.answer,
        );
    }

    // B5: the pre-turn pack at the default wait, then a larger one.
    const policy = await ctx.rpc("openhuman.memory_policy_get", {});
    const defaultWait = pick(policy, "recall.pre_turn_timeout_ms");
    const extendedWait = extendedPreTurnWait(defaultWait);
    const probes = [
      "What do I usually want in meeting summaries?",
      "Who is my manager?",
      "Where does my sister live?",
    ];
    const before = await countLogLines(ctx.logFile, /pre_turn timed out/);
    const tDef = ctx.newThread("B5-default");
    for (const p of probes) await ctx.turn(tDef, p);
    const atDefault =
      (await countLogLines(ctx.logFile, /pre_turn timed out/)) - before;
    await ctx.rpc("openhuman.memory_policy_set", {
      pre_turn_timeout_ms: extendedWait,
    });
    const tBig = ctx.newThread(`B5-${extendedWait}ms`);
    for (const p of probes) await ctx.turn(tBig, p);
    const atLarge =
      (await countLogLines(ctx.logFile, /pre_turn timed out/)) -
      before -
      atDefault;
    await ctx.rpc("openhuman.memory_policy_set", {
      pre_turn_timeout_ms: defaultWait,
    });
    ctx.results.pre_turn = {
      default_ms: defaultWait,
      extended_ms: extendedWait,
      timed_out_at_default: atDefault,
      timed_out_at_extended: atLarge,
      turns_each: probes.length,
    };
    check(
      "B5-pre-turn-default",
      atDefault === 0,
      `no pre-turn pack times out at the default wait (${defaultWait} ms)`,
      `${atDefault}/${probes.length} timed out at the default, ${atLarge}/${probes.length} at ${extendedWait} ms`,
      atDefault ? "high" : "low",
    );
  },
};

// ---------------------------------------------------------------------------
// C. Dates and time zone
// ---------------------------------------------------------------------------

const dates = {
  id: "C-dates",
  title:
    "Dates and time zone: user time zone, 'yesterday', a named date, reply language",
  engines: ["local", "builtin"],
  async run(ctx) {
    const { persona, check } = ctx;
    await ctx.rpc("openhuman.config_update_user_timezone", {
      timezone: persona.time_zone,
    });
    const tz = await ctx.rpc("openhuman.config_get_user_timezone", {});
    check(
      "C1-timezone",
      tz?.effective === persona.time_zone || tz?.timezone === persona.time_zone,
      `the time zone reads back as ${persona.time_zone}`,
      tz,
    );

    if (!ctx.dated?.length) {
      ctx.note("C2-skipped", "dated facts missing (A did not run); stored now");
      ctx.dated = [];
      for (const [offset, d] of [
        [-1, persona.dated[0]],
        [-5, persona.dated[1]],
      ]) {
        const day = isoDay(offset, persona.time_zone);
        ctx.dated.push({
          day,
          id: await ctx.learn(d.turn, "fact", {
            observed_at: `${day}T10:00:00Z`,
          }),
          text: d.turn,
        });
      }
    }
    await sleep(ctx.settleMs);
    if (ctx.recallQualityChecked) {
      const y = await ctx.turn(
        ctx.newThread("C2-yesterday"),
        "What did I do yesterday?",
      );
      ctx.qcheck(
        "C2-yesterday",
        containsAny(y.reply, ["vendor review", "northwind"]),
        "'yesterday' finds yesterday's fact",
        y.reply ?? y.error,
        "medium",
        "INFERRED",
      );
      const day = ctx.dated[1].day;
      const named = await ctx.turn(
        ctx.newThread("C3-named-date"),
        `What did I do on ${day}?`,
      );
      ctx.qcheck(
        "C3-named-date",
        containsAny(named.reply, ["dentist", "library"]),
        `a named date (${day}) finds that day's fact`,
        named.reply ?? named.error,
        "medium",
        "INFERRED",
      );
    }

    // C4: the reply follows the user's language.
    const es = await ctx.turn(
      ctx.newThread("C4-language"),
      "¿Cuál es la política de hoteles de Northwind en Berlín? Responde brevemente.",
    );
    const spanish =
      /\b(el|la|los|las|por|noche|hotel|euros?|de)\b/i.test(es.reply ?? "") &&
      /\b(noche|por|límite|tope|máximo|euros)\b/i.test(es.reply ?? "");
    ctx.qcheck(
      "C4-reply-language",
      spanish,
      "a Spanish question gets a Spanish reply",
      es.reply ?? es.error,
      "medium",
      "INFERRED",
    );
  },
};

// ---------------------------------------------------------------------------
// F. Labels and the observed actor (local reads the engine's own records)
// ---------------------------------------------------------------------------

const labels = {
  id: "F-labels",
  title:
    "Labels and actor: agent/thread on items, phone numbers plain, observed_actor only when on",
  engines: ["local"],
  async run(ctx) {
    const { persona, check } = ctx;
    // Phone numbers are stored as written (user decision 13:50).
    const id = await ctx.learn(
      `Priya's phone number is ${persona.contacts[0].phone}.`,
      "fact",
    );
    const got = await ctx.rpc("openhuman.memory_items_get", { ids: [id] });
    check(
      "F1-phone-plain",
      (got?.items?.[0]?.text ?? "").includes(persona.contacts[0].phone),
      "a phone number is stored in plain text",
      got?.items?.[0]?.text,
    );

    // observed_actor OFF (the default): logged turns carry no attribution.
    const tOff = ctx.newThread("F2-actor-off");
    await ctx.turn(
      tOff,
      "Remember that the balcony paint colour is sage green.",
    );
    const off = await ctx.waitItems({ thread_id: tOff }, 2, 60_000);
    check(
      "F2-thread-label",
      off.every((h) => h.meta?.thread_id === tOff),
      "every logged turn carries its thread_id",
      off.map((h) => h.meta?.thread_id),
    );
    check(
      "F2-agent-label",
      off.every((h) => !!h.meta?.agent_id),
      "every logged turn carries an agent_id",
      off.map((h) => h.meta?.agent_id),
    );
    // Attribution is written on the wire, not returned on read-back
    // (tinymemory #237/#238): judge it on the requests the core sent.
    const writesFor = (thread) =>
      (ctx.wire?.requests ?? []).filter(
        (r) =>
          /\/v1\/experience/.test(r.path ?? "") &&
          JSON.stringify(r.body ?? {}).includes(thread),
      );
    const offWrites = writesFor(tOff);
    if (!offWrites.length)
      ctx.note(
        "F3-no-wire",
        "no experience write for the OFF thread was captured on the wire",
      );
    else
      check(
        "F3-actor-off-wire",
        offWrites.every(
          (r) => !/"observed_actor"|"subject"/.test(JSON.stringify(r.body)),
        ),
        "with observed_actor off, no write carries observed_actor or subject (byte-identical)",
        offWrites.map((r) => Object.keys(r.body ?? {})),
      );

    // observed_actor ON: an assistant-turn write carries the agent as actor.
    await ctx.setConfigToml("memory", { observed_actor: true });
    await ctx.restartCore();
    const tOn = ctx.newThread("F4-actor-on");
    await ctx.turn(tOn, "Remember that the railing primer is grey.");
    await ctx.waitItems({ thread_id: tOn }, 2, 60_000);
    // The assistant turn is logged after the user turn: wait for its write.
    const onWrites = await waitFor(
      async () => {
        const w = writesFor(tOn);
        return w.some((r) => r.body?.content?.role === "assistant") ? w : null;
      },
      {
        timeoutMs: 60_000,
        intervalMs: 1000,
        what: "assistant-turn write on the wire",
      },
    ).catch(() => writesFor(tOn));
    ctx.results.observed_actor_wire = onWrites.map((r) => ({
      path: r.path,
      observed_actor: r.body?.observed_actor ?? null,
      subject: r.body?.subject ?? null,
    }));
    const attributed = onWrites.filter((r) =>
      /agent:/.test(JSON.stringify(r.body?.observed_actor ?? "")),
    );
    if (!onWrites.length)
      ctx.note(
        "F4-no-wire",
        "no experience write for the ON thread was captured on the wire",
      );
    else
      check(
        "F4-actor-on-wire",
        attributed.length > 0,
        "with observed_actor on, an assistant-turn write carries observed_actor agent:<id> on the wire",
        ctx.results.observed_actor_wire,
      );
    await ctx.setConfigToml("memory", { observed_actor: false });
    await ctx.restartCore();
  },
};

// ---------------------------------------------------------------------------
// G. Per-turn brain source cap
// ---------------------------------------------------------------------------

const sourceCap = {
  id: "G-source-cap",
  title:
    "Per-turn cap: more than four brain sources, at most four read, the named one included",
  engines: ["local", "builtin"],
  async run(ctx) {
    const { check } = ctx;
    const sources = ["notion", "gmail", "github", "files", "web"];
    for (const s of sources) {
      const r = await ctx.tryRpc("openhuman.memory_brain_ingest", {
        text: `Kestrel project note from ${s}: the launch checklist lives in ${s}.`,
        title: `Kestrel ${s} note`,
        source: s,
      });
      if (r.ok) ctx.ledger.add(r.value?.id);
      else
        ctx.note(
          `G-ingest-${s}`,
          `brain_ingest with source=${s} refused: ${JSON.stringify(r.error).slice(0, 200)}`,
        );
    }
    await sleep(ctx.settleMs);
    const pv = await ctx.tryRpc("openhuman.memory_pack_preview", {
      query: "Where is the Kestrel launch checklist in github?",
    });
    if (!pv.ok)
      return check(
        "G-pack-preview",
        false,
        "memory_pack_preview answers",
        pv.error,
      );
    const pack = pv.value?.pack ?? {};
    // Sources are on the section hits' namespaces (`refs` holds item ids).
    const namespaces = (pack.sections ?? []).flatMap((s) =>
      (s.hits ?? []).map((h) => String(h.meta?.namespace ?? "")),
    );
    const read = sources.filter((s) =>
      namespaces.some((ns) => ns.startsWith(`source:${s}`)),
    );
    ctx.results.source_cap = {
      sources_seen_in_pack: read,
      sections: (pack.sections ?? []).map((s) => s.title ?? s.name ?? s),
    };
    check(
      "G1-at-most-four",
      read.length <= 4,
      "at most four brain sources are read in one turn",
      read,
    );
    check(
      "G2-named-included",
      read.includes("github"),
      "the source named in the question is read",
      read,
    );
    const team = JSON.stringify(pack.sections ?? pack.markdown ?? "");
    check(
      "G3-no-team",
      !/team/i.test(team),
      "no Team section in the pack",
      pack.sections ?? null,
      "low",
    );
  },
};

// ---------------------------------------------------------------------------
// H. Forget
// ---------------------------------------------------------------------------

const forget = {
  id: "H-forget",
  title: "Forget: gone from list and recall; a forget of nothing says so",
  engines: ["local", "builtin"],
  async run(ctx) {
    const { check } = ctx;
    const id = await ctx.learn(
      "Jordan's locker number at the climbing gym is 4417.",
      "fact",
    );
    const f1 = await ctx.rpc("openhuman.memory_forget", { ids: [id] });
    check(
      "H1-forgotten-count",
      (f1?.forgotten ?? 0) >= 1,
      "forget reports the item removed",
      f1,
    );
    const got = await ctx.rpc("openhuman.memory_items_get", { ids: [id] });
    check(
      "H2-gone-from-get",
      !(got?.items ?? []).length,
      "the item no longer reads back",
      got,
    );
    const listed = await ctx.listAll({ tags_any: [ctx.marker] });
    check(
      "H3-gone-from-list",
      !listed.some((h) => h.id === id),
      "the item is gone from the list",
      { id },
    );
    if (ctx.recallQualityChecked) {
      const rec = await ctx.tryRpc("openhuman.memory_recall", {
        question: "What is my climbing gym locker number?",
      });
      check(
        "H4-gone-from-recall",
        !(rec.value?.answer ?? "").includes("4417"),
        "recall no longer returns it",
        rec.value ?? rec.error,
      );
    }
    const f2 = await ctx.tryRpc("openhuman.memory_forget", { ids: [id] });
    check(
      "H5-zero-forget-reported",
      f2.ok && f2.value?.forgotten === 0,
      "forgetting an already-forgotten id reports forgotten: 0 (not an error, not a false count)",
      f2.ok ? f2.value : f2.error,
      "low",
    );
  },
};

// ---------------------------------------------------------------------------
// I. QA re-tests
// ---------------------------------------------------------------------------

const qa = {
  id: "I-qa",
  title:
    "QA re-tests: belief builds, backfill state, list refresh, silent degradation",
  engines: ["local", "builtin"],
  async run(ctx) {
    const { check } = ctx;
    // I1: a belief build stores something (manual run bypasses the gate).
    // Local only: on the real account it would consolidate the account's own
    // memory, which a run must never do.
    if (ctx.engine === "builtin") {
      ctx.note(
        "I1-skipped",
        "belief builds are not run on the real account (they would consolidate its own memory)",
      );
    } else {
      const jobs = await ctx.tryRpc("openhuman.memory_jobs_run", {}, 300_000);
      ctx.results.jobs_run = jobs.ok ? jobs.value : { error: jobs.error };
      if (!jobs.ok)
        check("I1-jobs-run", false, "memory_jobs_run answers", jobs.error);
      else
        ctx.note(
          "I1-jobs-run",
          `memory_jobs_run: ${JSON.stringify(jobs.value).slice(0, 400)}`,
        );
    }
    // I2: past-conversations backfill state.
    const bf = await ctx.tryRpc(
      "openhuman.memory_conversations_backfill_status",
      {},
    );
    ctx.results.backfill = bf.ok ? bf.value : { error: bf.error };
    check(
      "I2-backfill-status",
      bf.ok,
      "the backfill status answers",
      bf.ok ? bf.value : bf.error,
      "low",
    );

    // I3: a new learning shows in the list at once (RPC level).
    const id = await ctx.learn(
      "Jordan's favourite tea is genmaicha.",
      "preference",
    );
    const listed = await ctx.listAll({
      kinds: ["learning"],
      tags_any: [ctx.marker],
    });
    check(
      "I3-list-refresh",
      listed.some((h) => h.id === id),
      "a learning shows in the learnings list straight after it is stored",
      { id, listed: listed.length },
    );

    // I4: silent degradation: the engine goes away mid-session (local only).
    if (ctx.engine === "local" && ctx.cortex) {
      await ctx.cortex.pause();
      const rec = await ctx.tryRpc(
        "openhuman.memory_recall",
        { question: "What tea do I like?" },
        60_000,
      );
      const learn = await ctx.tryRpc(
        "openhuman.memory_learn",
        {
          text: "unreachable engine probe",
          kind: "other",
          meta: ctx.markMeta(),
        },
        60_000,
      );
      const turn = await ctx.turn(
        ctx.newThread("I4-engine-down"),
        "What tea do I like?",
      );
      await ctx.cortex.resume();
      if (learn.ok) ctx.ledger.add(learn.value?.id);
      ctx.results.engine_down = {
        recall: rec.ok
          ? { answer: rec.value?.answer?.slice(0, 120) }
          : rec.error,
        learn: learn.ok ? "accepted" : learn.error,
        turn: turn.error ?? "completed",
      };
      check(
        "I4-recall-surfaces-error",
        !rec.ok,
        "recall with the engine down returns an error, not an empty answer",
        ctx.results.engine_down.recall,
      );
      check(
        "I4-learn-surfaces-error",
        !learn.ok,
        "a store with the engine down returns an error",
        ctx.results.engine_down.learn,
      );
      const warned = await countLogLines(
        ctx.logFile,
        /\[memory[^\]]*\].*(unavailable|degraded|connect|refused|error)/i,
      );
      ctx.note(
        "I4-turn-with-engine-down",
        `the chat turn ${turn.error ? `failed: ${turn.error}` : "completed"}; memory warnings in core.log so far: ${warned}`,
      );
    }
  },
};

// ---------------------------------------------------------------------------
// J. Hosted store speed (no real import)
// ---------------------------------------------------------------------------

const storeSpeed = {
  id: "J-store-speed",
  title:
    "Store speed on the hosted path: a synthetic batch, timed (no real import)",
  engines: ["builtin"],
  async run(ctx) {
    const N = 40;
    const batches = [];
    const errors = [];
    for (let b = 0; b < N / 10; b += 1) {
      const started = Date.now();
      for (let i = 0; i < 10; i += 1) {
        const r = await ctx.tryRpc("openhuman.memory_learn", {
          text: `memscen synthetic item ${b * 10 + i}: a short fictional note about the Kestrel project.`,
          kind: "other",
          meta: ctx.markMeta(),
        });
        if (r.ok) ctx.ledger.add(r.value?.id);
        else errors.push(JSON.stringify(r.error).slice(0, 200));
      }
      batches.push(Date.now() - started);
    }
    ctx.results.store_speed = {
      path: "memory_learn x10 per batch (no RPC exposes the import's store_many; this times the same hosted engine one item at a time)",
      batch_ms: batches,
      errors,
      unavailable_or_429: errors.filter((e) => /UNAVAILABLE|429|rate/i.test(e))
        .length,
    };
    ctx.check(
      "J1-no-store-errors",
      errors.length === 0,
      "a synthetic batch stores without errors",
      errors,
      "medium",
    );
  },
};

export const SCENARIOS = [
  storing,
  recall,
  dates,
  labels,
  sourceCap,
  forget,
  qa,
  storeSpeed,
];

/** Scenarios that run only on the local engine and are defined in their own modules. */
export function registerLocalOnly(...scenarios) {
  for (const s of scenarios)
    if (s && !SCENARIOS.some((x) => x.id === s.id)) SCENARIOS.push(s);
}

export { isoDay, containsAny, randomUUID, waitFor, fsp };
