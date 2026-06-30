---
description: >-
  How OpenHuman learns your communication style, identity, tooling, vetoes, and
  goals from everyday use, then surfaces them as ambient defaults in every reply.
icon: brain
---

# Personalization & Self-Learning

OpenHuman gets to know you the way a good assistant would: not by asking you to fill in a settings form, but by paying attention. As you chat, connect accounts, and correct it, it quietly collects evidence about how you like to work, scores that evidence for stability, and promotes the durable signals into your **`PROFILE.md`** and into the system prompt of every future turn.

Nothing is locked in from a single message. A preference has to keep showing up before it earns a place in your profile, and anything that fades stops being injected. You stay in control: the profile is a plain Markdown file you can edit, and you can pin or forget any learned fact.

***

## What gets learned

Learning is organized into six **facet classes**. Each class has its own decay rate and its own budget so one noisy category can't crowd out the others.

| Class | What it captures | Examples |
| --- | --- | --- |
| **Style** | How you like replies written | `verbosity=terse`, `format=bullets`, `emoji=skip` |
| **Identity** | Stable facts about you | `name=Alice`, `timezone=PST`, `role=engineer` |
| **Tooling** | Your developer toolchain | `package_manager=pnpm`, `editor=neovim`, `lang=rust` |
| **Veto** | Things you've explicitly rejected | `avoid em dashes`, `no nested bullets` |
| **Goal** | Active goals and ongoing projects | free-form goal sentences |
| **Channel** | Your preferred place to talk | `primary=desktop-chat` |

Recurring people, topics, and past threads are **not** stored here — those live in the [memory tree](obsidian-wiki/memory-tree.md) and are pulled in per-turn by `memory_recall`.

***

## The learning pipeline

Evidence flows through four stages: **capture → score → render → inject**.

```text
 your activity                 candidate buffer            stability detector
 ────────────                  ────────────────            ──────────────────
 chat turns        ──┐
 corrections       ──┤
 email signatures  ──┼──→  LearningCandidate ──→  rebuild every 30 min
 connected accounts──┤      (class, key, value,    + event-driven (~60s
 LinkedIn (opt-in) ──┘       cue family, evidence)   after new data)
                                                          │
                                                          │  score each (class, key)
                                                          │  resolve value conflicts
                                                          │  apply per-class budgets
                                                          ▼
                                                   user_profile facets
                                                   (Active / Provisional /
                                                    Candidate / Dropped)
                                                          │
                                          CacheRebuilt ───┤
                                                          ▼
                                  ┌───────────────────────┴───────────────┐
                                  ▼                                        ▼
                            PROFILE.md                            system prompt
                       (managed blocks)                     ("Your standing preferences")
```

**Capture.** Producers watch your activity and push a `LearningCandidate` into a bounded buffer. Each candidate records the `(class, key, value)` it asserts, a pointer back to the evidence, and a **cue family** describing how strong the signal is — `Explicit` (you said it outright), `Structural` (from account data or a file), `Behavioral` (inferred from how you act), or `Recurrence` (a statistical pattern).

**Score.** A background **stability detector** rebuilds the cache roughly every 30 minutes, and sooner when new email or documents arrive. It aggregates every candidate for a given fact and computes a stability score: stronger cue families count for more, recent evidence counts for more than stale evidence (each class has a half-life), and an explicit statement from you doubles the weight.

| Class | Evidence half-life |
| --- | --- |
| Identity | 90 days |
| Veto | 60 days |
| Tooling / Goal | 30 days |
| Style | 14 days |
| Channel | 7 days |

The score decides each fact's lifecycle state:

| State | Meaning |
| --- | --- |
| **Active** | Strong enough to render in `PROFILE.md` and inject into the prompt |
| **Provisional** | Stored and tracked, but not yet shown |
| **Candidate** | Still gathering evidence |
| **Dropped** | Faded below the floor; removed |

When two values compete for the same fact (e.g. `verbosity=terse` vs `verbosity=detailed`), the higher-stability value wins. The loser is dropped, and if it ever becomes true again it re-earns its place naturally.

***

## Where it's stored: `PROFILE.md`

The learned profile is materialized into **`PROFILE.md`** in your workspace — a real, editable Markdown file. Each facet class owns a managed block (`## Style`, `## Identity`, `## Tooling`, `## Vetoes`, `## Goals`), and only **Active** facets are written, sorted by stability. Pinned entries are marked `*(pinned)*`.

The renderer only touches its own managed blocks. Anything you write by hand outside those blocks — and the separate `## Connected Accounts` block owned by the integrations layer — is left untouched. Empty classes show a `*(no entries yet)*` placeholder rather than disappearing.

> **Per-session freeze.** `PROFILE.md` is folded into the agent's system prompt at the start of a session and held stable for that session's lifetime, which keeps prompt caching fast. Edits you make mid-session are picked up on the next rebuild and the next session — not retroactively in the current one.

***

## How it surfaces in replies

On every turn the agent reads the Active facets and injects them as a compact **"Your standing preferences"** section in the system prompt, alongside a standing instruction to call `memory_recall` before answering anything that leans on past sessions. The result is that the agent defaults to your verbosity, your tools, and your vetoes without being reminded — and reaches into memory for the specifics.

***

## Optional LinkedIn enrichment

During onboarding you can let OpenHuman bootstrap your identity from LinkedIn. The flow searches your connected Gmail for a `linkedin.com/in/...` profile URL, and (when available) scrapes the public profile, then compresses what it finds into `PROFILE.md` via the `learning_save_profile` step. It runs once, as a fire-and-forget pass — entirely opt-in, and skipped cleanly if no profile is found.

***

## Reviewing and controlling what's learned

Everything learned is inspectable and reversible:

- **Edit `PROFILE.md` directly.** It's your file. Correct, add, or delete anything; the next rebuild respects your edits.
- **The Brain page** (raised center button in the bottom bar, `/brain`) is the home for memory and intelligence — the knowledge graph, goals, sources, and sync status all live here.
- **Pin** a fact to lock it Active and shield it from decay, or **forget** a fact to drop it and block it from coming back. Under the hood these are the `learning_pin_facet`, `learning_unpin_facet`, and `learning_forget_facet` operations over the `openhuman.learning_*` RPC surface, alongside `learning_list_facets` and `learning_rebuild_cache`.

***

## See also

* [Memory Tree](obsidian-wiki/memory-tree.md), where recurring people, topics, and threads live and are recalled per-turn.
* [Goals & To-dos](goals-and-todos.md), the goal-tracking surface that pairs with learned `goal/*` facets.
* [Subconscious Loop](subconscious.md), the background engine that keeps thinking about your workspace between turns.
