#!/usr/bin/env bash
# Guard — the removal simulator still agrees with cargo.
#
# scripts/dep-sim.py projects the effect of cutting a dependency cohort,
# and every gating decision in the kernelization program is sized with
# it. It parses `cargo tree` precisely so it cannot drift from cargo's
# own feature resolution — an earlier version walked `cargo metadata`'s
# resolve graph instead and over-reported by 36 crates, counting
# dev-dependencies and unenabled target-specific edges.
#
# This asserts the calibration still holds. If it fails, every
# projection built on the simulator is suspect until it is fixed.
#
# The number is the Linux name count of the `flows` profile, so it moves
# whenever scripts/kernel-floor.limits does and belongs in the same PR.
# 264 -> 265 on 2026-08-21: the tinymemory #76/#77 bump adds exactly one
# name, `tinymemory-bus`. 265 -> 267 on 2026-08-22: the MCP extraction
# adds `tinymcp` and `tinymcp-bus`. 267 -> 268 on 2026-08-22: language
# runtimes moved behind the `tinyruntime` TinyBus module, adding
# `tinyruntime-bus`. 268 -> 269 on 2026-08-23: the TinyJuice wire
# contract moved into `tinyjuice-bus`, which cannot be gated because
# `inference::tokenjuice` compiles in every build. 269 -> 270 on
# 2026-08-29: `tinytools` becomes the dependency behind the `Tool`
# trait/types, which cannot be gated because `tools/` is kernel
# surface — zero transitive additions, native build count unchanged
# at 2. 270 -> 273 on 2026-08-30: the `tinyflows` crate's catalog/
# storage/copilot surface split into three sibling path-crates in the
# SAME vendored workspace (`tinyflows-catalog`, `tinyflows-sqlite`,
# `tinyflows-copilot`) — one name each, no new external crate, native
# build count unchanged at 2. 273 -> 275 on 2026-08-31: the compatible
# dependency refresh recorded in `kernel-floor.limits` raised the
# resolved Linux name count while keeping the native build count at 2.
# 269 -> 270 on 2026-09-13: the core/wrapper split adds the required
# `openhuman-rpc` workspace crate; its five dependencies were already
# in the profile, so this is one package/name and no native build.
# 270 -> 271 on 2026-09-18: `tinytools-agent` becomes the shared
# provider-neutral tool-call protocol crate; it adds one Rust crate
# and no native build dependency.
# 286 -> 283 on 2026-09-29: the merged dependency graph now resolves three
# fewer unique crate names in the flows profile; keep calibration aligned
# with the lowered kernel floor.
# See the kernel-floor history for prior raises. The current
# macOS graph resolves three more names than CI Linux; this calibrates against
# the Linux target used by CI.
#
# This number MUST move in lockstep with scripts/kernel-floor.limits —
# it is a second source of truth for the same Linux name count and is
# not derived from that file. Bump both in the same PR that raises
# (or lowers) the floor.
# 271 -> 275 on 2026-09-18: the inference migration replaces the
# monolithic `tinyinference` package with four additional resolved
# packages in the flows profile; native build count remains 2.
# 275 -> 279 on 2026-09-20: the required TinyAgents runtime/session/
# graph split adds four unique crate names without native dependencies.
# 279 -> 277 on 2026-09-20: the merged fresh-turn and cost-routing
# dependency refresh sheds two of those resolved names again.
# 277 -> 280 on 2026-09-22: TinyAgents 2.1.2 moves its required
# runtime/session/graph split into the harness path; it adds three
# crate names and no native build dependency.
# 280 -> 282 on 2026-09-25: TinyChannels 0.1.3 resolves HMAC 0.13 and
# activates digest 0.11's ctutils/cmov tail, adding two names but no native
# build dependency. See the matching kernel-floor history entry.
# 286 -> 283 on 2026-09-29: the updated locked dependency graph resolves
# three fewer unique names in the Linux flows profile; no native build
# dependencies changed. See kernel-floor.limits.
# 285 -> 286 on 2026-09-27: multi-provider search uses the required
# tinysearch-bus contract in always-on config and policy code; one name,
# no native dependency. See kernel-floor.limits.
# 286 -> 283 on 2026-09-29: current vendored dependency resolution sheds
# three package names without changing the native build dependency count.
# 282 -> 285 on 2026-09-26: the required TinyBrowser integration adds
# tinybrowser-bus, tinybrowser-control, and tinyjevclient. Native count stays 2.
# 286 -> 283 on 2026-09-29: the current flows graph sheds three resolved
# packages and crate names without changing native builds. See kernel-floor.limits.
# 283 -> 282 on 2026-09-29: the JSON-RPC split removes openhuman-rpc from the
# core graph after the TinyMCP v0.3.5 update; native count remains 2.
# 282 -> 283 on 2026-09-30: TinyChannels supplies the always-on CLI channel
# and shared runtime helpers through `tinychannels-runtime`; the required
# workspace package adds one name, no dependency tail or native build. See PR #6782.
# 299 -> 304 on 2026-10-01: the current merged upstream module pins resolve
# five additional crate names in the flows profile. The measured graph has 321
# packages / 304 names and still 2 native builds; the kernel-floor history
# records the upstream update and its measured increase.
#
# Called by ci-lite.yml's feature-gate smoke lane and by the lane runner, so the
# expected count lives here once (plus scripts/kernel-floor.limits).
set -euo pipefail

cd "$(dirname "$0")/../.."

EXPECTED_NAMES=302

exec python3 scripts/dep-sim.py --cut-nothing --expect-names "${EXPECTED_NAMES}"
