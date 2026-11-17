// E. Layout v3 and the move into it (LOCAL ONLY: never on the real account).
//
// Runs last on the local engine: everything the earlier scenarios stored is in
// the legacy layout by then and moves with it.

import fsp from "node:fs/promises";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { sleep, waitFor, pick } from "./lib.mjs";

/** The v1 store the importer reads (DDL as in memory/import_tests.rs LEGACY_DDL). */
const LEGACY_DDL = `
CREATE TABLE memory_docs (
  document_id TEXT PRIMARY KEY, namespace TEXT NOT NULL, key TEXT NOT NULL, title TEXT NOT NULL,
  content TEXT NOT NULL, source_type TEXT NOT NULL, priority TEXT NOT NULL, tags_json TEXT NOT NULL,
  metadata_json TEXT NOT NULL, category TEXT NOT NULL, session_id TEXT, created_at REAL NOT NULL,
  updated_at REAL NOT NULL, markdown_rel_path TEXT NOT NULL, UNIQUE(namespace, key));
CREATE TABLE episodic_log (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, timestamp REAL NOT NULL,
  role TEXT NOT NULL, content TEXT NOT NULL, lesson TEXT);
CREATE TABLE user_profile (facet_id TEXT PRIMARY KEY, facet_type TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0.5, evidence_count INTEGER NOT NULL DEFAULT 1, source_segment_ids TEXT,
  first_seen_at REAL NOT NULL, last_seen_at REAL NOT NULL);
`;

const V1_DOCS = [
  [
    "v1-d1",
    "notes",
    "garden",
    "Garden plan",
    "Plant the winter kale on the east bed in November.",
  ],
  [
    "v1-d2",
    "notes",
    "boat",
    "Boat club",
    "The Lakeside boat club meets on the first Saturday of the month.",
  ],
  [
    "v1-d3",
    "learning:style",
    "tone",
    "Tone",
    "Keep answers short and skip the pleasantries.",
  ],
];

/** Write a small v1 store into `<workspace>/memory/memory.db`. */
async function writeV1Store(workspace) {
  const dir = path.join(workspace, "memory");
  await fsp.mkdir(dir, { recursive: true });
  const db = new DatabaseSync(path.join(dir, "memory.db"));
  db.exec(LEGACY_DDL);
  const now = Date.now() / 1000 - 86_400 * 30;
  const doc = db.prepare(
    "INSERT INTO memory_docs VALUES (?, ?, ?, ?, ?, 'chat', 'normal', '[]', '{}', 'core', NULL, ?, ?, ?)",
  );
  for (const [id, ns, key, title, content] of V1_DOCS)
    doc.run(id, ns, key, title, content, now, now, `${key}.md`);
  const ep = db.prepare(
    "INSERT INTO episodic_log (session_id, timestamp, role, content, lesson) VALUES ('v1-s1', ?, ?, ?, NULL)",
  );
  ep.run(now, "user", "Can you remind me when the boat club meets?");
  ep.run(
    now + 5,
    "assistant",
    "The Lakeside boat club meets on the first Saturday of the month.",
  );
  db.prepare(
    "INSERT INTO user_profile VALUES ('v1-f1', 'preference', 'tone', 'terse', 0.9, 1, NULL, ?, ?)",
  ).run(now, now);
  db.close();
  return { docs: V1_DOCS.length, conversations: 2, learnings: 1 };
}

export const migration = {
  id: "E-migration",
  title:
    "Layout v3 and the move: import first, then organize; resumable; nothing lost; chats pooled",
  engines: ["local"],
  last: true,
  // A user-started move is gated on background work, which the key-holding
  // core keeps off; this one runs on a core that never saw the key.
  noAccount: true,
  async run(ctx) {
    const { check } = ctx;
    const status = () => ctx.rpc("openhuman.memory_migration_status", {});

    // Seed the legacy layout with a little more, and a question to re-ask.
    await ctx.learn("Jordan's bike is a teal Brompton named Kestrel.", "fact");
    const t = ctx.newThread("E-legacy-chat");
    await ctx.turn(t, "Note that the Kestrel launch is on the 14th.");
    await ctx.waitItems({ thread_id: t }, 1, 60_000);
    const question = "What colour is my bike?";
    const before = await ctx.tryRpc("openhuman.memory_recall", { question });

    const { workspace, config } = await ctx.activeWorkspace();
    check(
      "E0-starts-legacy",
      (pick(config, "memory.layout") ?? "legacy") === "legacy",
      "the run starts in the legacy layout",
      pick(config, "memory.layout"),
    );

    const v1 = await writeV1Store(workspace);
    const scan = await ctx.rpc("openhuman.memory_import_scan", {});
    check(
      "E1-import-found",
      scan?.found === true,
      "the v1 store is found",
      scan,
    );
    const mscan = await ctx.rpc("openhuman.memory_migration_scan", {});
    check(
      "E1-move-needed",
      mscan?.needed === true,
      "legacy memory is offered for the move",
      mscan,
    );

    const beforeItems = await ctx.listAll({});
    const beforeTexts = new Set(beforeItems.map((h) => h.text));
    ctx.results.migration = { before_count: beforeItems.length, v1 };

    // One banner flow: import with consent; organizing starts on its own.
    await ctx.rpc("openhuman.memory_import_start", { consent: true });
    const imported = await waitFor(
      async () => {
        const s = await ctx.rpc("openhuman.memory_import_status", {});
        return ["done", "error"].includes(pick(s, "state.phase")) ? s : null;
      },
      { timeoutMs: 600_000, intervalMs: 1000, what: "import finished" },
    );
    check(
      "E2-import-done",
      pick(imported, "state.phase") === "done",
      "the import finishes",
      imported,
    );

    // On a self-hosted CortexDB the legacy tree counts as shared: the move
    // waits for the takeover dialog's consent (by design). Give it, as the
    // user confirming the dialog would.
    const gate = await status();
    if (mscan?.shared) {
      check(
        "E2b-waits-for-consent",
        !gate.running && pick(gate, "state.phase") !== "cleaned",
        "a shared tree is not moved without consent",
        gate,
        "high",
      );
      await ctx.rpc("openhuman.memory_migration_start", { takeover: true });
    }
    // Organizing starts on its own; try to stop the core mid-move.
    let restarted = false;
    const deadline = Date.now() + 600_000;
    let last;
    while (Date.now() < deadline) {
      last = await status();
      const phase = pick(last, "state.phase");
      if (!restarted && last.running && phase === "copying") {
        await ctx.restartCore();
        restarted = true;
        const after = await status();
        ctx.results.migration.after_restart = after;
        if (!after.running)
          await ctx.rpc("openhuman.memory_migration_start", {});
        continue;
      }
      if (phase === "cleaned" && !last.running) break;
      await sleep(250);
    }
    check(
      "E3-organized",
      pick(last, "state.phase") === "cleaned",
      "organizing finishes (cleaned)",
      last,
      "high",
    );
    if (!restarted)
      ctx.note(
        "E4-restart",
        "the move finished before it could be caught mid-copy; resume-after-restart was not exercised this run",
      );
    else
      check(
        "E4-resumed",
        pick(last, "state.phase") === "cleaned",
        "the move finishes after a core restart mid-copy",
        ctx.results.migration.after_restart,
      );

    const { config: after } = await ctx.activeWorkspace();
    check(
      "E5-layout-v3",
      pick(after, "memory.layout") === "v3",
      "the layout setting is now v3",
      pick(after, "memory.layout"),
      "high",
    );

    // Nothing lost: every item text from before is still there.
    const afterItems = await ctx.listAll({});
    const afterTexts = new Set(afterItems.map((h) => h.text));
    const lost = [...beforeTexts].filter((x) => !afterTexts.has(x));
    ctx.results.migration.after_count = afterItems.length;
    ctx.results.migration.failures = pick(last, "state.failures") ?? [];
    check(
      "E6-nothing-lost",
      lost.length === 0,
      "every item from before the move is in the per-user tree",
      { lost: lost.slice(0, 10), lost_count: lost.length },
      "high",
    );
    const v1Seen = V1_DOCS.filter(([, , , , content]) =>
      afterItems.some((h) => (h.text ?? "").includes(content.slice(0, 30))),
    );
    check(
      "E7-v1-imported",
      v1Seen.length === V1_DOCS.length,
      "the v1 store's documents are in memory",
      { seen: v1Seen.map((d) => d[0]) },
    );

    // Chats pooled at ws:main.
    const convs = afterItems.filter((h) => h.kind === "conversation");
    const unpooled = convs.filter(
      (h) => !String(h.meta?.namespace ?? "").startsWith("ws:main"),
    );
    check(
      "E8-chats-pooled",
      convs.length > 0 && unpooled.length === 0,
      "every conversation sits at the pooled chat node (ws:main)",
      {
        conversations: convs.length,
        elsewhere: unpooled.slice(0, 5).map((h) => h.meta?.namespace),
      },
    );

    // Recall answers unchanged.
    const afterRecall = await ctx.tryRpc("openhuman.memory_recall", {
      question,
    });
    if (ctx.recallQualityChecked)
      ctx.qcheck(
        "E9-recall-unchanged",
        /teal/i.test(afterRecall.value?.answer ?? "") ===
          /teal/i.test(before.value?.answer ?? ""),
        "the same question recalls the same fact after the move",
        { before: before.value?.answer, after: afterRecall.value?.answer },
        "medium",
        "INFERRED",
      );

    ctx.note(
      "E10-not-automated",
      "workflow memory staying out of chat (sandbox) and the unconfirmed-import no-erase guard are covered by unit tests, not driven here",
    );
  },
};
