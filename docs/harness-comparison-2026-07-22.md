# Agent harness resource-footprint comparison

Date: 2026-07-22
Method: web research against primary sources where they exist (GitHub repos and
issue trackers, official docs), with every weakly-sourced figure flagged. Our
own numbers come from the measured benchmarks in
[`library-benchmarking.md`](library-benchmarking.md) and
[`resource-profiling-session-2026-07-21.md`](resource-profiling-session-2026-07-21.md).

The single most important source-quality finding: **the only fully measured,
reproducible numbers in this comparison are ours.** Codex publishes binary
size but no RSS; ZeroClaw's numbers are vendor marketing with no third-party
verification; Claude Code's dramatic figures are leak bugs, not steady state;
Hermes' figure is self-reported documentation.

## Comparison table

| Harness | Language / runtime | Deployment shape | RAM idle | RAM under load | Startup | Binary / install | N-agent scaling | Source quality |
|---|---|---|---|---|---|---|---|---|
| **OpenHuman core** (ours) | Rust, embeddable library | Library or one RPC process; agents share the process | 44-51 MiB settled (default); 35-44 MiB slim | Cold turn +26-31 MiB (first-use); warm turn +0.5-1.9 MiB | ~100-140 ms cold turn; ~0 idle CPU | 116 MiB default / 81 MiB library-minimal / 60 MiB stripped | **In-process**: ~0.4 MiB/agent cold roster, ~1.8 MiB warm marginal | Measured, reproducible (this repo) |
| OpenAI Codex CLI (codex-rs) | Rust, single native binary | CLI process per session | no published RSS | no published RSS (qualitative claims only) | "milliseconds" (qualitative) | **80 MB** (macOS arm64, primary: issue #13091) | N independent processes | Binary size primary; RSS unpublished |
| Codex CLI (old Node/TS) | Node.js / V8 | CLI process per session | no published data | no published data | Node startup | npm + Node runtime | N processes | none published |
| ZeroClaw | Rust, static binary | CLI + optional daemon | **< 5 MB (self-reported, unverified)** | **no verified figure** (the oft-quoted "7.8-12 MiB" has no locatable primary source) | "< 10 ms" (self-reported) | 3.4 MB (one page says ~8.8 MB — internally inconsistent) | "multiple concurrently", no numbers | Marketing only; provenance suspect (SEO domain cluster) |
| OpenClaw (Clawdbot → Moltbot → OpenClaw) | TypeScript / Node.js | Local daemon + channel bridge | "> 1 GB" claimed only by competitor marketing | no neutral figure | slow (Node + heavy deps) | ~28 MB (per competitor comparison) | N processes | Rebrand history primary (TechCrunch/CNBC/Forbes); RAM figure biased |
| Claude Code | Node.js / V8 CLI | CLI process per session | ~500 MB claimed (weak SEO source) | documented **leak bugs**: 400-500 MB/min idle growth, multi-GB, extremes 14-93 GB | Node startup | npm + Node runtime | N processes | Leak bugs primary (issues #67433, #28731, #22188); baseline weak |
| Hermes Agent (Nous Research) | **Python 81% / TS 16%** (not Rust; it bundles the Rust-written `uv`) | CLI + gateway daemon; subagents are isolated subprocesses | no granular RSS; **4 GB RAM minimum** system req | "< 500 MB without a local LLM" (self-reported docs) | not published | Python 3.11 env | N subprocesses | Repo/languages primary; RAM self-reported |

## Per-harness notes

**OpenAI Codex CLI.** Confirmed Rust rewrite (~June 2025) shipping one
self-contained binary. The only hard number is 80 MB binary size on macOS
arm64, from OpenAI's own tracker (openai/codex#13091) — which proposes
feature-gating heavy dependencies to reach ~55-60 MB, directly analogous to
our Cargo domain gates. Memory claims are qualitative ("no unbounded Node heap
growth"). Scope: coding agent only — no persistent curated cross-session
memory core, no multi-agent orchestration, no channels, no workflow engine.

**ZeroClaw.** Rust single-binary positioned against OpenClaw. All numbers are
vendor self-reported (`/usr/bin/time -l` on their own build) with zero
third-party verification, promoted across a cluster of lookalike SEO domains.
The "7.8-12 MiB under load" figure previously cited in our docs could not be
found in any source and has been downgraded to unverified. ZeroClaw is a
separate project from OpenClaw, not a rebrand.

**OpenClaw lineage.** The Clawdbot → Moltbot → OpenClaw rebrand chain is
well-sourced (TechCrunch, CNBC, Forbes, Jan 2026). The ">1 GB RAM" figure
appears only in ZeroClaw's competitive marketing; plausible for a Node daemon
with browser automation, but there is no neutral benchmark.

**Claude Code.** Node/V8 CLI. No clean published idle baseline; ~500 MB comes
from a third-party SEO article and leak-report starting points. What is
well-documented (primary GitHub issues) is a family of off-heap RSS leak bugs:
400-500 MB/min growth while idle (#67433), 14 GB OOM (#28731), 93 GB heap
(#22188), idle CPU thrash (#18280). Those are bugs, not steady state — but
they are a cautionary tale about native-buffer discipline in long-running
Node agent processes.

**Hermes Agent.** The closest scope match to OpenHuman (SQLite + FTS5 + WAL
curated memory, parent/child subagent lineage, cron, unified
Telegram/Discord/Slack/Signal/WhatsApp/WeChat gateway) — and it is Python 81% /
TypeScript 16%, not Rust. Subagents run as isolated subprocesses, so it pays
its base footprint per agent. Self-reported "under 500 MB without a local
LLM", 4 GB RAM minimum.

## What this means for OpenHuman

**Today.** Against honest scope-matched peers we are clearly leaner: Hermes at
similar capability self-reports ~10x our settled RSS and requires 4 GB
minimum; Claude Code starts around a claimed ~500 MB with documented multi-GB
leaks; Codex's binary (80 MB) is larger than our stripped library-minimal
build (60 MB). The only harness claiming to be dramatically smaller —
ZeroClaw at "<5 MB" — is unverified marketing carrying far less capability.

**End-state.** The library-minimal + shared-services target (~15 MiB private
footprint + ~2 MiB per in-process agent) is not a stretch goal: today's
~42 MiB slim RSS already decomposes to 15.2 MiB private / 3.2 MiB live heap,
the rest being reclaimable executable text and allocator high-water. State
that with the RSS-vs-private-footprint caveat attached.

**Worth borrowing / leaning into:**

1. Feature-gating heavy deps is now industry practice (Codex #13091) —
   external validation of our domain-gate investment.
2. Rust + single self-contained binary is the market direction; the
   Node-based peers are the ones with RSS horror stories.
3. **In-process shared-services scaling is our differentiator.** Every
   scope-matched peer scales agents as N OS processes, paying the fixed base
   N times. Our ~0.4-1.8 MiB marginal per in-process agent is the entire
   basis of the 1000-agents-in-2-GB story; nobody else has it.
4. Internalize (not borrow) Claude Code's leak history: keep the warmed
   repeated-turn plateau benchmark as a standing regression gate.

## Sources

- OpenAI codex#13091 — 80 MB binary / feature-gating proposal
- devclass (2025-06) — Codex Rust rewrite announcement coverage
- anthropics/claude-code#67433, #28731, #22188, #18280 — leak/idle-CPU bugs
- zeroclaw.net; openclawconsult.com "lab" comparison (self-reported marketing)
- TechCrunch / Forbes (2026-01) — OpenClaw rebrand lineage
- github.com/nousresearch/hermes-agent — language split, architecture
- hermes-agent.nousresearch.com docs — memory features, footprint claim
