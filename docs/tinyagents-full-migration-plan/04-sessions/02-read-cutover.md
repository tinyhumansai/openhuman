# 04.2 — Read-side shadow, cutover, retirement

Import phases 2–4 from `../../tinyagents-session-migration-design.md`.

## Steps

1. **Shadow (phase 2):** add a store-backed reader implementing the same
   surface as `transcript.rs` (`read_transcript`, history load in
   `session_io.rs`, `.md` render). Behind a flag, read BOTH and log
   divergence (grep-friendly `[session_import]` prefix); run the 11-fixture
   matrix + live dogfood.
2. **Cutover (phase 3):** run `openhuman.session_import_run` automatically
   once at boot (idempotent marker `migrations/session_import_v1` already
   exists), flip default reads to the store, keep legacy JSONL as
   write-through backup for one release.
3. **Retire (phase 4):** stop legacy writes; delete the legacy stack.
4. Sub-agent transcript stems (`__` chains) and worker-thread mirrors must
   resolve through store descriptors — verify against
   `subagent_runner` mirroring and `agent_orchestration/subagent_sessions/`
   (that store also folds into descriptors here).
5. Parser retirement gate: keep `agent/dispatcher.rs` and
   `harness/parse.rs` until no live provider path needs XML/P-format prompt
   dialects, native text-fallback parsing, TinyAgents text-mode response
   parsing, or sub-agent checkpoint cleanup.

## Deletions (phase 4)

- `session/transcript.rs` (1347) + `transcript_tests.rs`.
- `session/migration.rs` (373) + tests — old date-folder migration, still
  invoked at startup through the `migrate_session_layout_if_needed` re-export
  until the read-side cutover has a replacement startup gate.
- `session/turn/session_io.rs` (391) — replaced by store reader/writer.
- `session_db/{ops,store,schemas,types}.rs` generic session parts (keep
  `run_ledger/` until 05).
- `agent_orchestration/subagent_sessions/` (~650).
- `src/openhuman/session_import/` (whole domain + RPC controller from
  `core/all.rs`) once a marker-versioned release has shipped.

## Acceptance

- All fixtures byte/render-identical between readers before flip.
- Old UI session lists still answer by OpenHuman session key (compat
  projection over descriptors).
