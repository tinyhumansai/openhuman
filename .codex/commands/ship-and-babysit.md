---
name: ship-and-babysit
description: Commit local changes, push the branch to the user's fork, open or reuse a PR against tinyhumansai/openhuman:main, then babysit CI and CodeRabbit feedback until the PR is green and clean.
---

# Ship And Babysit

Canonical long-form workflow also lives at:

- [`.agents/agents/ship-and-babysit.md`](../../.agents/agents/ship-and-babysit.md)

Use this command when you want an end-to-end ship flow for `tinyhumansai/openhuman`:

1. Commit relevant local changes with a conventional commit.
2. Push the current non-`main` branch to `origin` (the user's fork).
3. Open or reuse a PR against `tinyhumansai/openhuman:main`.
4. Poll CI and CodeRabbit feedback.
5. Fix actionable issues, commit, and push follow-ups.
6. Stop only when the PR is green and clean.

Guardrails:

- Never push to `upstream`.
- Never push directly to `main`.
- Never resolve a review thread without either fixing it or replying with a reasoned dismissal.
- Do not merge the PR.
