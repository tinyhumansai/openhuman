---
name: ship-and-babysit
description: Commit local changes, push to the user's fork, open or reuse a PR against tinyhumansai/openhuman:main, then babysit CI and CodeRabbit feedback until the PR is green and clean.
---

# Ship And Babysit

Canonical long-form workflow lives at:

- [`.codex/skills/ship-and-babysit/SKILL.md`](../skills/ship-and-babysit/SKILL.md)
- [`.agents/agents/ship-and-babysit.md`](../../.agents/agents/ship-and-babysit.md)

Use this command when you want an end-to-end ship flow for `tinyhumansai/openhuman`:

1. Commit relevant local changes with a conventional commit.
2. Push the current non-`main` branch to `origin` (the user's fork).
3. Open or reuse a PR against `tinyhumansai/openhuman:main`.
4. Poll CI and CodeRabbit feedback.
5. Fix actionable issues, commit, and push follow-ups.
6. Stop only when the PR is green and clean.

Feature testing rule:

- Feature work must ship with matching E2E coverage.
- Core, domain, persistence, CLI, and JSON-RPC features need mocked Rust E2E coverage in `tests/*_e2e.rs`, commonly `tests/json_rpc_e2e.rs`.
- Frontend user flows need mocked Playwright E2E coverage in `app/test/e2e/specs/*.spec.ts`.
- Mock backend calls all the way through with the repo mock backend; do not hit real backend services or third-party APIs in E2E.

Guardrails:

- Never push to `upstream`.
- Never push directly to `main`.
- Never resolve a review thread without either fixing it or replying with a reasoned dismissal.
- Do not merge the PR.
- Fill `.github/PULL_REQUEST_TEMPLATE.md` exactly. Every checklist item must be checked; use `- [x] N/A: <reason>` when an item does not apply.
