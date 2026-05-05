---
name: Bug
about: Used for bug reports
title: ""
type: Bug
assignees: ''

---

Use a concise sentence-case title that describes the broken behavior. Do not add `Bug` or bracket prefixes to the title.

## Summary

What failed, in one or two sentences (user-visible symptom or test failure).

## Problem

What happened vs what you expected, impact, and **steps to reproduce** (ordered, minimal). Include **version / platform** (app version, OS, desktop vs dev) if known.

## Solution (optional)

Suspected cause, workaround, or proposed fix. Skip if unknown.

## Acceptance criteria

- [ ] **Repro gone** — Bug no longer reproduces on the stated environment (or root cause documented if intentional).
- [ ] **Regression safety** — Unit, integration, or E2E coverage added or updated if this should not come back.
- [ ] **Diff coverage ≥ 80%** — the fix PR meets the changed-lines coverage gate (Vitest + cargo-llvm-cov, enforced by [`.github/workflows/coverage.yml`](../../.github/workflows/coverage.yml)).
- [ ] **…** — Other verify-before-close items.

## Related

Links to issues, PRs, logs, or prior discussion.
