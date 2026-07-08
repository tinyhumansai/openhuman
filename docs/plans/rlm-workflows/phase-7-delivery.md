# Phase 7 — Delivery: two PRs, one gigantic + one focused

## 7.1 TinyAgents PR (first)

- Repo: `tinyhumansai/tinyagents`, branch `feat/repl-host-embedding`
  (created off `357bcc8`, the commit the submodule currently pins).
- Content: Phase 2 (cancel flag, live EventSink call events, embedding
  docs, tests). Small focused commits per tinyagents conventions.
- Until it merges, openhuman development proceeds against the local
  submodule checkout (the `[patch]` already points at
  `vendor/tinyagents`), so nothing blocks.

## 7.2 OpenHuman PR (the gigantic one)

- Repo: `tinyhumansai/openhuman`, branch `feat/rlm-language-workflows` off
  `upstream/main`; push to `origin` (fork `senamakel/openhuman`), PR
  `--head senamakel:feat/rlm-language-workflows` against upstream.
- One PR containing, as a series of small coherent commits:
  1. `docs(plans)`: this plan folder.
  2. `chore(deps)`: enable tinyagents `repl` feature.
  3. `feat(rlm)`: types + policy mapping.
  4. `feat(rlm)`: capability bridge.
  5. `feat(rlm)`: session manager.
  6. `feat(rlm)`: ops (eval, cancel, events).
  7. `feat(rlm)`: the `rlm` tool + registration + prompt/about_app docs.
  8. `fix/feat(rlm)`: hardening pass (error taxonomy, layered timeouts).
  9. `test(rlm)`: the Phase 6 suite.
  10. `chore(vendor)`: bump tinyagents submodule pointer to the merged
      Phase 2 commit (after the tinyagents PR lands; if it hasn't merged
      yet, the openhuman PR pins the branch head and notes the dependency
      in the PR body).
- PR body: architecture summary (from README), the two-repo dependency,
  commands run, coverage numbers, and explicit callout that tests were
  authored in the final phase per the brief.

## 7.3 Merge order & risk

1. tinyagents PR merges → retag/pin.
2. openhuman PR updates submodule pointer commit, CI full lane re-runs.
3. Rollback story: the feature is dark unless the tool registers
   (`OPENHUMAN_RLM=0` kill switch + not registered on readonly tier);
   reverting the tool-registration commit disables the surface without
   touching the domain.

## 7.4 Follow-ups (explicitly out of v1)

- Graph execution from scripts (`graph_run` executing compiled graphs) —
  blocked on tinyagents implementing super-step execution behind the REPL.
- RPC/CLI exposure of RLM sessions (controller schemas) + a dedicated UI
  timeline card for cells.
- Durable session persistence across core restarts.
- Streaming partial stdout from a running cell.
