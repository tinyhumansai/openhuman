# Private Rust validation inventory and measurements

This inventory records the test selections before changing execution for #8063.
Coverage replaces equivalent executions inside the private job. Feature sets are
not treated as equivalent merely because they select the same crate.

| Selection                                                         | Earlier execution                       | Private execution                                 |
| ----------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------- |
| `openhuman` product `--lib --bins`                                | coverage nextest, background unit build | coverage nextest, sequential                      |
| `openhuman-cli` product integration targets                       | coverage nextest `kind(test)`           | same, fails empty selection                       |
| `openhuman-embed`, `openhuman-tinyhumans` product, all targets    | coverage libtest                        | coverage nextest                                  |
| `openhuman-embed`, `openhuman-tinyhumans` default, all targets    | omitted as presumed duplicates          | separate coverage nextest configurations          |
| `openhuman-rpc`, `openhuman-tui` default, all targets             | coverage libtest                        | coverage nextest                                  |
| core no-default lib, existing gate filters                        | plain libtest                           | coverage nextest with equivalent substring unions |
| core no-default plus `modules,mcp,e2e-test-support`, gate filters | serial plain libtest                    | serial coverage nextest                           |
| Tauri own manifest/default features                               | coverage libtest                        | coverage nextest                                  |
| CLI storage E2E targets, SQLite only                              | plain libtest                           | coverage nextest, same targets                    |
| core storage domain filters excluding cron module tests           | plain libtest                           | coverage nextest, same inclusions/exclusions      |
| core storage cron bus/documents tests                             | serial plain libtest                    | serial coverage nextest                           |
| RPC `session-store,storage-sqlite`, `session_store::`             | plain libtest                           | coverage nextest                                  |

The storage E2E list and domain filter list remain declared in `lanes-plan.mjs`.
Product features remain in `scripts/ci/product-features.txt`; they are resolved
per package, preserving existing feature forwarding. Checksum-pinned native test
modules, coverage-presence checks and the 80% changed-line gate remain required.
Doctests, clippy, rustdoc, examples and uninstrumented link checks remain separate
because coverage/nextest does not replace them.

The Composio stack-budget regression has `#![cfg(not(coverage))]` and runs once
through uninstrumented nextest in `rust-lint`. Instrumented integration
expectations exclude that target and, outside Windows, the entirely
`#[cfg(windows)]` sandbox target. Present binaries containing only ignored tests
are checked against nextest's compiled inventory; all other missing or empty
expected targets fail validation.

## Full-workflow overlap audit

[CI Full](../.github/workflows/ci-full.yml) calls
[test-reusable.yml](../.github/workflows/test-reusable.yml), whose core job
already enumerates all **46 root Rust integration targets**. The
[Rust E2E runner](../scripts/test-rust-e2e.sh) selects **35 of those same target
names**: `tests/*_e2e.rs` plus `in_process_all`. Job names do not establish
that their test selections are distinct.

| Executions or mode                                                                     | Overlap and configuration differences                                                                                                                                                                                                                                                                                                                                            | Decision                                                                                                           |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| CI Full's direct Rust E2E job and its `e2e-desktop` call's nested `rust-e2e-linux` job | Both invoke `pnpm test:rust:e2e`, selecting the same 35 targets with product features and the Node mock. The direct job pins its container digest and sets line-table debug information; the reusable job uses a moving image tag and inherits its own workflow's `OPENHUMAN_NODE_ENABLED=0`. Caller workflow environment is not automatically inherited by a reusable workflow. | Retain: repeated selections, but runtime environments are not proven equivalent.                                   |
| Reusable core integrations and Rust E2E                                                | All 35 E2E target names occur in the core job's 46-target inventory. The core job selects `-p openhuman-cli` with product features plus `bin-tools`; E2E selects the workspace with product features and starts an ambient Node mock. Individual tests may also start their own mocks or clear inherited backend settings.                                                       | Retain: package, feature, mock and process setup differ; a shared target name does not prove equivalent execution. |
| `mcp_registry_e2e`                                                                     | The E2E runner explicitly adds `--ignored --test-threads=1`; ordinary integration execution skips these ignored cases.                                                                                                                                                                                                                                                           | Preserve this distinct test mode.                                                                                  |
| `tinymcp_storage_smoke_e2e`                                                            | The E2E runner adds `e2e-test-support` and serial execution.                                                                                                                                                                                                                                                                                                                     | Preserve this distinct feature configuration.                                                                      |
| `tinydocs_linked_e2e`                                                                  | The E2E runner adds non-product `linked-modules`.                                                                                                                                                                                                                                                                                                                                | Preserve this distinct feature configuration.                                                                      |
| Browser/desktop E2E and Windows secrets validation                                     | These exercise frontend or native host flows, Windows-specific code, or external-service configuration.                                                                                                                                                                                                                                                                          | Preserve these distinct contexts.                                                                                  |

These full/reusable workflows do not invoke the private coverage lane. No
execution above is removed on the assumption that private coverage replaces an
equivalent selection in the same validation run. Further deduplication would
require checking the resolved package features and normalizing the relevant
runtime settings first.

The current local callers of `test-reusable.yml` are `ci-full.yml` and
[test.yml](../.github/workflows/test.yml); the callers of `e2e-reusable.yml` are
`ci-full.yml` and [e2e.yml](../.github/workflows/e2e.yml). Comments describing
direct staging/production pretest callers are stale. The
[staging](../.github/workflows/release-staging.yml) and
[production](../.github/workflows/release-production.yml) workflows remain
untouched by this change.

## Historical warm baseline

[Run 39006804022](https://github.com/tinyhumansai/openhuman/actions/runs/39006804022),
head `a90ebba82e4bbfe4d454107620b52bf18092932a`, completed its EX63 job on
2026-10-17 from 05:53:35 to 06:22:35 UTC: **29m00s**, excluding approximately
18m15s queue time. This used parallel lanes and the previous sccache backend.

- Cache: 5,373 Rust hits, zero misses. This is a **warm** baseline, not a cold run.
- Core nextest: 10,168 unit tests and 447 integration tests passed; 10 and 19 skipped.
- Remaining core coverage libtest records: 945 passed across 89 result records.
- Tauri: 309 passed, one ignored. Gates-off: 488 passed. Storage: 2,549 passed.
- Coverage: 1,554 eligible files present; changed-line report 86% over 125 measured
  changed lines (17 violations). Four LCOV reports were archived.
- Core coverage check: 20m23s; unit execution 546.719s; integration 316.683s.
- Cache save time is unavailable for this old backend.

These counts are executions, not deduplicated test identities. The historical
commit and runner allocation differ from the implementation benchmark, so this
is context rather than a controlled speed-up claim.

## Measurement protocol

The initial diagnostic at `0b93ecf6bc1012fa06886c87b30ee0eebbd7e5f0`
finished with a failing aggregate result in 117m49s: lanes took 108m24s and cache
saving took 8m36s. Its resource limit changed mid-run, so these timings are not
a controlled cold measurement. All eight main Rust configurations passed their
12,225 test executions; all 23 offline examples passed. The diagnostic exposed
missing vendor tag history, the frontend lint heap limit, an unsupported
`llvm-cov show-env --features` option, and conflicting auxiliary coverage flags.
The seven auxiliary coverage configurations failed before running tests and
must not be counted as covered merely because an LCOV report was produced.

A second diagnostic at `e35e5c0240b61bc8cffaf857f8f826c4276ce8b6`
failed after 107m03s when the 160 GiB scratch disk filled during the ordinary
Tauri link. The subsequent coverage build and status writes reported `ENOSPC`;
storage suites, Pester and cache saving were not reached. The 12,714 recorded
executions across ten instrumented configurations and the plain Composio test
passed, but this was a failed workflow and no warm run followed it.

Its closed artifacts showed per-process auxiliary coverage profiles and large
ordinary debug sections. The corrected private plan bounds every coverage
process to eight merging profile slots, removes only the completed release
example graph, and uses `line-tables-only` for ordinary dev/test builds. Coverage
keeps its existing debug level zero. Actual Cargo 1.96.1 checks confirmed dev and
test file-and-line backtraces and assertions, the coverage override, and unchanged
release artifacts. Release output includes reflinks, so its allocated size does
not establish how much physical space cleanup recovers. These fixes require a
new complete cold/warm pair; neither diagnostic establishes final performance.

The private workflow budget is now 180 minutes, including a bounded 15-minute
cache save. The earlier 120-minute budget had almost expired before those seven
configurations could execute; the provisional five-minute save limit was below
the observed 8m36s save time. Final measurements must use the corrected source
and unchanged resources throughout each run.

Run the complete selected lane plan twice at the same source revision and image,
using fresh target outputs. Preserve package-manager caches, and identify whether
the signed remote Rust cache already contains those compiler keys. Do not call a
run cold merely because its target directory is new. Record Tinymold hit/miss and
save time, test counts, coverage presence, linkage proof, each check's result and
elapsed time. Preserve failed runs too. After merging, repeat through trusted-main
workflow routing, since pre-merge runs can execute the old workflow.

## Complete Linux manual validation

Both runs at `3a01ec0c54a54f30a14ab92cd5fdbf241d6e9316` independently passed
with identical selected test identities. The [measurement record](ci-private-measurements-8063.json)
contains exact counters, resource limits and archive digests.
Both used the same published image, frozen injected rootfs/kernel, four vCPUs,
13 GiB RAM, 160 GiB scratch, 32 GiB guest swap and the production 58 GiB shared
host limit. The measured contract used a 180-minute full-entry budget and
195-minute guest deadline; there were no additional per-unit CPU or memory caps.

| Metric                                  |               Cold |               Warm |
| --------------------------------------- | -----------------: | -----------------: |
| Validation lane interval                |        8,631.081 s |        4,134.076 s |
| Full manual entry, including cache save | 8,869 s (2h27m49s) | 4,172 s (1h09m32s) |
| Whole VM lifecycle                      |        8,877.017 s |        4,180.850 s |
| Rust test executions                    |             15,572 |             15,572 |
| Rust failures/errors                    |                  0 |                  0 |
| Local compiler-cache hits               |              2,286 |              2,717 |
| Remote compiler-cache hits              |                  0 |              3,801 |
| Compiler-cache misses                   |              3,830 |                 33 |
| Compiler-cache hit rate                 |              37.4% |              99.5% |
| Compiler-cache errors                   |                  0 |                  0 |
| Cache-save duration/result              |     206 s / exit 0 |       1 s / exit 0 |

| Configuration                              | Cold executions | Warm executions |
| ------------------------------------------ | --------------: | --------------: |
| `openhuman-lib-bins-nextest-`              |          10,168 |          10,168 |
| `openhuman-embed`                          |             372 |             372 |
| `openhuman-rpc`                            |             154 |             154 |
| `openhuman-tinyhumans`                     |             328 |             328 |
| `openhuman-embed-default-`                 |             337 |             337 |
| `openhuman-tinyhumans-default-`            |             328 |             328 |
| `openhuman-tui`                            |              91 |              91 |
| `openhuman-cli-integration-tests-nextest-` |             447 |             447 |
| `composio-stack-uninstrumented`            |               1 |               1 |
| `gate-contract-tests`                      |             432 |             432 |
| `gate-contract-tests-features`             |              56 |              56 |
| `tauri`                                    |             309 |             309 |
| `storage-e2e`                              |              43 |              43 |
| `storage-lib-tests`                        |           2,486 |           2,486 |
| `cron-module-tests`                        |              13 |              13 |
| `storage-session-store-tests`              |               7 |               7 |

Independent pair verification parsed the XML and individual identities against
count JSON and the compiled CLI inventory: 42 expected CLI targets, 36 executed
binaries and six present ignored-only binaries. All 16 configurations ran once;
all 50 active checks passed sequentially, including ten dependency edges. Nine
LCOV reports were nonempty, all 1,554 eligible source files were present, and
actual ordinary and instrumented ELF files contained mold metadata. The retained
changed-line gate exited zero with zero measured product lines; its percentage
is not a measurement of these CI scripts. All 23 offline examples and Pester
also passed.

The manual full-entry clock starts before cloning a local root Git bundle and
includes fresh public vendor clones using the pinned shared checkout script,
pinned action downloads, preflight, cache setup, all forced-area lanes,
coverage/link checks and cache save. It excludes host preparation, VM boot and
shutdown, GitHub step dispatch, the path-filter action and artifact upload. The
whole-VM clock includes boot, entry smoke and shutdown. GitHub uses an
authenticated root checkout of the PR merge ref instead of this local bundle.
These values therefore describe the manual run rather than a GitHub job.

Cold means a new empty remote ActionCache namespace, fresh guest, target outputs
and local Tinymold store. Production package caches are retained, and the shared
CAS may already contain deduplicated blobs. The warm repeat retained that same
namespace and package-cache disk. This is compiler-key cold, with observed
production co-tenancy. Neighbors changed during the cold run and transient host
memory reclaim occurred; periodic unit PSI avg10 samples reached 60.91%, with
zero host OOM events. The warm observed unit PSI avg10 maximum was 4.56%.
Historical and cold/warm elapsed differences do not isolate a cache-only causal
effect.

The completed cold/warm scratch had 2,133,032,960 / 2,155,548,672 bytes
(about 1.99 / 2.01 GiB) free. Optional live disk polling stopped early in cold
and did not cover warm; these are closed footprints rather than peak headroom.
Release example files were absent after scoped cleanup. Logical ordinary ELF sizes fell
from 922,632,896 to 538,239,416 bytes for approvals and from 926,900,368 to
536,418,496 for Composio relative to the disk-full diagnostic. Reflinks make
allocated subtree sizes upper bounds, so those figures do not establish physical
reclaimed bytes. The remaining scratch margin is narrow.

### Preflight guard added after measurement

After the measured revision, preflight gained a required LLVM-tool check and
`CARGO_LLVM_COV_SETUP=no`. The paired results above retain their exact measured
revision; the lane selections, build settings and compiler-cache policy are
unchanged. Separate published-image verification reproduced the old guard
accepting a missing LLVM-tool path, then confirmed the new guard rejects it
before publishing environment. The real installed tools passed. Seven focused
contract tests and the complete script suite passed: 772 tests, three platform
skips, zero failures. No component installation or image modification was used
for this guard verification.

### Trusted-base rollout

[The earlier GitHub run](https://github.com/tinyhumansai/openhuman/actions/runs/39066797765)
used main's 120-minute workflow and cancelled during Rust lint after core coverage
passed. [Budget prerequisite #8067](https://github.com/tinyhumansai/openhuman/pull/8067)
raises that finite limit to 180 minutes, matching the measured contract and the
already configured 195-minute supervisor deadline. A fresh synchronize or
ready-for-review event must follow its merge:
[reruns retain the original event revision](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs).
Verification of the new checkout/Tinymold action steps through trusted-main
routing remains pending their rollout.
