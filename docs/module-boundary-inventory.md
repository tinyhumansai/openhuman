# Loadable module boundary inventory

Status: the Channels, MCP, Wallet and TinyBox host cutovers are merged, and the
final scoped local acceptance passed. The module-boundary gate reports zero
violations, zero exceptions and no pending contracts; TinyRuntime remains
explicitly excluded for separate removal work. The machine-readable dependency policy is
[`scripts/ci/module-boundaries.json`](../scripts/ci/module-boundaries.json).

Every owner below is the canonical upstream under
`https://github.com/tinyhumansai/`. Contract changes land there first. OpenHuman
host adapters switch only after compatible per-platform artifacts are released
and their digests pinned. Do not remove a dependency by removing its capability,
copying its implementation into a host, or accepting weaker isolation.

| Owner | Linked implementation packages | Contract | Host consumers | Required module work |
| --- | --- | --- | --- | --- |
| tinyjuice | none (module loaded through TinyBus) | tinyjuice-bus | core tools, compression, CCR REPL | Complete: PR #59 provides module-owned compression, CCR queries, HTML extraction, schemas and declarations; handle/marker parsing (`tinyjuice_bus::handle`) and `juice_*` tool-name constants (`tinyjuice_bus::tools`) live in the bus since contract 2.2; the host retains turn-bound model callbacks and artifact authorization |
| tinyconnectors | none | tinyconnectors-bus | core integrations, credentials and triggers | Module-backed argument preparation, action execution, trigger archives and the curated toolkit catalogs (`ListCuratedCatalog`, `GetVocabulary`, held as a process snapshot) and per-toolkit scope preferences (`GetUserScopes`, `SetUserScopes`; a legacy workspace file is migrated into the module once), live tool-contract fetch/cache (`GetToolContract`, `ListToolContracts`), the confirmed-Read output probe (`ProbeToolOutput`) and the backend/direct connected-integrations walk (`ListConnectedIntegrations`), with the synchronous slug helpers, the task-source configuration types (`ProviderSlug`, `FilterSpec`, `SourceTarget`, `FetchReason`) and the host-owned offered-toolkit allow/deny policy (`[composio] offered_toolkits_allow` / `offered_toolkits_deny`, parsed by `ComposioConfig`) passed to the module in its load blob by `module_config` and enforced by the module's listings (contract 1.21; a module loaded from the search path gets no host blob, so it offers every toolkit); tool visibility asks `ListVisibleTools` and listings are scope-filtered by the module; the host keeps the overview cache, mode switch, prompt mapping and `contract_gate`; host credential reconciliation and frozen tool restoration remain in OpenHuman |
| tinymcp | none in normal/build host dependencies | tinymcp-bus | core MCP registry, supervisor, CLI stdio/HTTP server | Merged host adapters use the released v0.16.1 module (contract 1.23; configured-client receipt machine and client cache are `tinymcp_bus::client_lifecycle`, host keeps only the bus-calling driver; identifier validation, the registry-detail JSON shape and the not-connected explanation are `tinymcp_bus` helpers, so the host's `registry/helpers.rs` is gone; bus members the host never calls are kept for other hosts and documented in `tinymcp_bus::names`) for registry, supervisor, client and server protocol operations, with approved host callbacks kept in OpenHuman. The 62-member artifact and local focused, lifecycle, registry, native and build gates pass; frozen recorded tool declarations are restored independently of the current catalog |
| tinychannels | tinychannels, tinychannels-runtime, tinychannels-module (test-only) | tinychannels-bus | core channels, TinyHumans host, CLI REPL | Contract 16 (operation 1.17) is pure; scoped providers, inbound/identity/outbound preparation, remote commands, runtime limits, typing and progressive delivery execute through confidential module calls. Host-owned policy, credentials, persistence and callbacks remain in OpenHuman. Both host implementation exceptions are removed; the 254-test focused suite and released-artifact runtime fixture pass |
| tinyhosts | none | tinyhosts-bus | core hosting tools | Released Execute/Providers contract consumed through the module (v0.4.0: tool-to-operation mapping, argument shaping, outcome matching and summaries live in `tinyhosts-bus::tools`); workspace authorization, credentials and approval policy remain in OpenHuman |
| tinymemes | none | tinymemes-bus | core `tinymemes` reply remix | Released Remix contract (v1.1) consumed through the module, which owns the reading, rating, rewrite, slang and meme indexes, the remixed-id ring (behind its `IndexStore` port), the `TINYMEMES_*` env overrides, the LLM-as-Jev fallback and the background slang and meme research. The host serves the Complete, Evaluate, Search, LoadIndex, SaveIndex, Deliver and End callbacks (OpenHuman inference, managed Jev, web search, documents on the storage port), each carrying the remix's correlation id so remixes for different workspaces run concurrently, and keeps the A/B arm, flag and delivered-history policy. `tinymemes-bus` is synchronous and I/O-free (no tinybus or tinystoragedrivers-bus); the typed clients are in `tinymemes-client`, which the host does not link |
| tinywallet | none in normal/build host dependencies | tinywallet-bus | core wallet/web3/x402 | Complete: v0.20.0 contract 1.14 adds mnemonic word-count validation to module-owned mnemonic derivation, native transactions, Web3 quotes/execution, ERC-20 encoding, and x402 payment parsing/budgets/ledger. OpenHuman retains encrypted-key custody, authenticated transport, explicit approval, and lifecycle orchestration |
| tinybox | tinybox-core (test-only) | tinybox-bus | core sandbox and security, desktop gateways | Host cutover uses TinyBox v0.1.28 (contract 1.18); the host calls members only through `tinybox_bus::names` constants and uses `tinybox_bus::CommandClass` directly (no host copy or re-export). It covers for sandbox/resource lifecycle, collected and supervised execution, jailed and native shell execution (unsandboxed, cron and the `shell` tool run through `StartNativeShellExec`; core has no shell selection or spawn of its own), standalone Docker policy validation (`ValidateDockerPolicy`), Docker control, local grants (`ResolveLocalGrants`; the host re-applies `is_always_forbidden` to the reply), file transfer and port forwarding. Host authorization and the security floor remain in OpenHuman |
| tinycomputer | none | tinycomputer-bus | core voice, paste/focus and permission checks | Permission, focus, paste and Globe listener operations use the module; native thread and target rules remain covered |
| tinyvoice | None | tinyvoice-bus | core voice capture/hotkeys | Module-owned devices, recording and continuous capture, bounded audio reads/batches, explicit lease cleanup; computer module permission decisions |
| tinyruntime | tinyruntime-pyserver | tinyruntime-bus | core Python worker, optional TinyJuice ML | Excluded from this migration: TinyRuntime is being removed separately |
| tinydocs / tinymemory | none | tinydocs-bus | core memory converter and file sources | Bus DocumentConverter replaces OfficeConverter, including XLSX and existing PDF/DOCX/PPTX format coverage; ingestion metadata remains host-owned; `[components].documents = false` refuses `ensure_loaded("tinydocs")` and the `documents` tool group, makes the converter answer `COMPONENT_DISABLED:documents`, and degrades attachment ingest to a note |
| tinysearch | none observed | tinysearch-bus (v0.5.0, contract 2.2) | core module search proxy; error classification (`errors::Category`), role helpers and route/presentation config types live in tinysearch-bus; render text/markdown and provider labels live in `tinysearch_bus::{render,labels}`, core keeps the UI-metadata projection (`search/render.rs`) | Preserve existing bus adapter; enforce contract and host dependency graphs |

## Gate behavior

`pnpm rust:module-boundaries` checks the resolved normal and build dependency
closure of core, CLI, TUI and the excluded desktop workspace with all features.
Dev-only dependencies are excluded, including the test-only TinyWallet key
implementation. Package IDs, not dependency aliases, determine graph identity.
Target-specific dependencies are retained by Cargo metadata without a platform
filter. A new implementation package within an inventoried owner namespace
fails unless it is a registered contract.

Contracts are resolved separately in generated probe workspaces, once with
default features and once with every declared feature. Only serialization,
schema and error-derive closures are approved. This catches optional runtime or
implementation imports independently of host feature selection. Probes do not
compile native module code, and their manifests and lockfiles are kept under
`target/module-boundaries/` in the checkout.

TinyRuntime is explicitly outside this migration, as requested, and is recorded
in `excludedSubsystems` instead of the temporary exception list. Its removal
is separate work. It still cannot enter another audited contract's approved
serialization closure.

Exceptions are exact package/scope pairs with reasons. Host exceptions still
traverse their descendants. Contract exceptions stop at the first unsafe edge:
its implementation closure is not approved, and that edge must be removed with
the owning migration. Channels and Wallet now have pure released contracts.
An unused exception fails, prompting removal instead
of leaving future regressions silently exempted.

`pnpm rust:module-boundaries:complete` also fails while any exception or pending
contract exists. Passing the transitional gate means no unlisted boundary
violation was observed; it does not mean this plan is complete.

## Remaining acceptance work

`openhuman_rpc::embed::modules::client(config)` builds a
`tinybus::module::ModuleClient` from explicit runtime
configuration and uses the same process-wide loader before or after core
startup. It has no linked fallback. Resolution reports and client errors use a
closed reason vocabulary and registry metadata. Cached resolution failures and
native terminal unavailability are deduplicated; independent failed invocations
each produce an event. Cached callers revisit reporting if resolution finished
before a Sentry client was bound. An unfinished loader wait produces no terminal
event. Reports clear request scope and exclude raw loader paths and remote fault
prose. Existing domain adapters still need to migrate through this reporting path.

Upstream operations and artifacts, gateway integration, frozen tool restoration,
lifecycle regressions and coverage of all migrated adapter failures are still
required. No capability migration has landed in this change. The minimal/default/product/individual-feature build matrix,
platform compilation, runtime bus fixtures and shed measurements remain
separate acceptance checks. Dependency counts alone do not demonstrate a
build-time or binary-size improvement.

## Upstream migration work

Owner changes are independently reviewable; host dependencies and artifact pins
remain unchanged until compatible upstream releases are available.

| Change | Canonical PR | Local verification |
| --- | --- | --- |
| Async HTML extraction seam, deadlines and generic tool-metadata sanitization in TinyTools | [tinytools#60](https://github.com/tinyhumansai/tinytools/pull/60) | 1,183 workspace tests and six doctests; clippy/build; independent sanitizer review accepted; all 113 source files at least 90% coverage |
| TinyJuice typed CCR/content queries, HTML extraction, pure schemas and declarations | [tinyjuice#59](https://github.com/tinyhumansai/tinyjuice/pull/59) | 739 all-feature tests and doctests; compiled-artifact E2E; module input/limit guards |
| Complete TinyDocs Markdown conversion for memory ingestion | [tinydocs#31](https://github.com/tinyhumansai/tinydocs/pull/31) | 183 tests; dynamic artifact E2E; per-file coverage at least 90% |
| TinyHosts pure vocabulary and authorized source preparation with captured deployment bytes | [tinyhosts#21](https://github.com/tinyhumansai/tinyhosts/pull/21) | 210 all-feature and 179 default tests including doctests; compiled snapshot and large legacy deployment probes; pure contract audit; all 19 implementation files at least 90% coverage; independent preparation review accepted |
| TinyComputer native permissions, confidential focus/paste and reliable Globe leases/read/shutdown | [tinycomputer#87](https://github.com/tinyhumansai/tinycomputer/pull/87) | 1,372 all-feature and 1,367 default tests; 90-member compiled artifact verification; independent ownership and event-loss review accepted; macOS cross-check and 233-file coverage gate pass; physical macOS helper not exercised on Linux |
| TinyConnectors argument preparation, task filtering, structured provider errors and leased archives | [tinyconnectors#46](https://github.com/tinyhumansai/tinyconnectors/pull/46) | 423 tests; dynamic artifact calls; user archive lifecycle fixtures; pure contract audit; per-file coverage at least 90% |
| TinyChannels contract vocabulary separated from provider, relay, pairing and runtime behavior | [tinychannels#56](https://github.com/tinyhumansai/tinychannels/pull/56) | 1,174 default and 1,180 all-feature tests; independent contract audit and relocation review accepted; bus files at least 98.65% coverage; legacy provider/worker coverage gaps disclosed |
| TinyBox pure contract, shell facts, reserved sandbox/process handles and acknowledged cleanup/shutdown | [tinybox#30](https://github.com/tinyhumansai/tinybox/pull/30) | 710 default and all-feature tests; 57 source files at least 90% coverage; compiled native-process artifact verification; independent lifecycle review accepted; at PR #30, supervised execution was limited to Unix passthrough |
| TinyWallet pure contracts, address validation and stateless EVM construction with exact approval facts | [tinywallet#56](https://github.com/tinyhumansai/tinywallet/pull/56) | 745 all-feature and 601 default tests; four-chain signing and native/ERC-20/contract construction through the compiled artifact; pure contract audit; all 95 source files at least 90% coverage; independent construction review accepted |
| TinyWallet module-owned Web3 and x402 operations with scoped service handles | [tinywallet#62](https://github.com/tinyhumansai/tinywallet/pull/62) | Independent verification at ec22874: 60 module tests, every source file at least 90% coverage, and Rust 1.88 native runners execute two signing tests plus one lifecycle test. Hosted coverage passes; signed-payment budget accounting remains under review repair before merge and compatible release |
| TinyVoice device enumeration, recording/continuous capture, reserved hotkey leases, replayable batches and acknowledged shutdown | [tinyvoice#23](https://github.com/tinyhumansai/tinyvoice/pull/23) | Contract 1.4; 32-member compiled artifact; 88 module tests; independent hotkey, Windows owner and Xvfb cleanup fixtures; pure contract audit, own-module formatting and Windows GNU test cross-check; formatted module source coverage 1,592/1,743 (91.34%), with the existing physical-device exclusion; no physical Windows/MSVC or macOS input validation |
| TinyRuntime pure JSONL worker vocabulary, persistent cache recipes, optional provider preparation and module-owned lifecycle | [tinyruntime#29](https://github.com/tinyhumansai/tinyruntime/pull/29) | 244 router unit tests, 6 default / 7 all-feature public API tests, 64 contract tests, 35 pyserver tests and doctests; 13-member compiled artifact verifies old-provider compatibility, preparation, cache reuse, adoption and rebuilding; per-file coverage gate passes; independent cleanup, cache and provider-bridge reviews accepted |
| TinyMCP pure shared vocabulary, supervisor observations, server callbacks and bounded text/argument/tool rendering operations | [tinymcp#54](https://github.com/tinyhumansai/tinymcp/pull/54) | 1,471 all-feature and 1,359 default tests; dynamic 44-member artifact verification; pure contract audit; all 93 source files at least 90% coverage; independent lifecycle, vocabulary and library-feature reviews accepted; external regressions cover module-disabled and host feature combinations |

The TinyDocs operations need a new published module artifact.
TinyHosts preserves existing member arities and adds authorized preparation inside
Execute; consuming that operation requires a new published artifact. No local build digest
has been used as a release pin, and these PRs do not yet remove any host exception.

TinyChannels preserves its serialized vocabulary and moves behavioral APIs to
implementation crates, using compatibility extension traits where needed. Its
relay/pairing/delivery operations now have owning-module implementations in PR 56,
including bounded queues, replayable results and shutdown reconciliation. The
OpenHuman adapters remain outstanding.
Published v0.1.13 contains the earlier implementation-bearing contract, so that
release does not unblock this contract cut.

TinyBox v0.1.24 supplies the host's sandbox operations through the released
module, including Docker and namespace/SSH backend selection, supervised process
output, jailed and native shell execution, local grant resolution, file transfer
and port forwarding. Availability still depends on the backend and platform
facts returned by the module; OpenHuman retains command authorization and its
security floor.

TinyRuntime is now excluded from the active migration because it is being removed
separately. The previously published owning PR remains recorded above; the frozen
Python-provider work has not been published or integrated. The boundary inventory
still records existing dependencies until the separate removal reaches OpenHuman.

TinyHosts’ bounded directory preparation currently returns up to 4 MiB of source.
Existing larger Launch/Deploy requests keep their transport budget. Streaming or
module-owned prepared artifacts remain required for larger host directory inputs.

TinyComputer’s reliable Globe reads retain one bounded snapshot until acknowledgment.
Native overflow, malformed events and unexpected EOF mark continuity loss. Hosts
must await its terminal shutdown before unloading. TinyVoice owns native Linux and
Windows hotkey listeners and accepts sequenced generic facts for macOS. Its reads
retain the previous batch across continuity loss, allowing a valid acknowledgment
to retrieve the inactive reset snapshot. Shared activation types live in the bus
contract and are re-exported by the implementation. OpenHuman still needs the
Computer-to-Voice adapter and compatible released artifacts before switching.

TinyDocs PR 35 moves PNG/JPEG header interpretation into the module and adds
bounded typed image inspection. Its parser regressions and rebuilt artifact
verification pass. The source API change requires a minor package release;
host presentation and document ingestion still await the approved release.

## Current delivery gates

The module PR babysitters repair CI and review findings and merge only after
checks, approval, resolved feedback and mergeability all pass. TinyRuntime is
excluded from that work. CI success does not clear an old changes-requested
review, and resolved threads do not establish independent approval.

TinyConnectors PR 46 is merged; its canonical main CI passed. The minor release
[run 38084368799](https://github.com/tinyhumansai/tinyconnectors/actions/runs/38084368799)
is building compatible platform artifacts. Host adoption remains gated on
published packages and verified digests. No host dependency cut is claimed here.

## Released connector integration (2026-10-11)

The `module-connectors-host` branch consumes TinyConnectors v0.14.0, source
`8b35ca5`, contract 1.13. All 16 archive bytes were hashed and matched both
`checksum.toml` and GitHub asset digests before replacing registry pins. The
host calls the module for preparation, calendar defaults, task-window filtering,
provider classification and opaque archive operations. It releases archive
leases on scope changes, sign-out and runtime shutdown; cancellation during
open retains the handle for cleanup. Frozen recorded tool declarations are
retained by the existing session adapter.

The normal/build graph audit removes the two connector implementation
exceptions and reports 32 remaining temporary exceptions and two pending
contracts. The measured flows profile is 335 packages / 313 names / two native
builds, compared with the combined foundation baseline of 340 / 318 / two.
Validation passes: 354 Composio tests against the released native artifact and
local provider mocks, 176 module/Sentry tests, ten recorded-tool tests, the
no-loader library check, the contract/host graph audit, crate-chain, feature
forwarding and Rust layout checks. A new Sentry regression first captured two
events; the fixed adapter emits one sanitized event and preserves existing
provider messages. The broad native suite first exposed changed authentication
text and an uninitialized-history read; both existing regressions now pass.
This does not claim completion of the full migration.

TinyWallet #56 and TinyHosts #21 merged with green hosted checks and approval.
Their release workflows exposed two packaging failures: Wallet still selected
an implementation crate with an unpublished sibling dependency, and Hosts
shared native build-script caches across Ubuntu images. Owner follow-ups
[Wallet #60](https://github.com/tinyhumansai/tinywallet/pull/60) and
[Hosts #25](https://github.com/tinyhumansai/tinyhosts/pull/25) correct the release
workflows with regressions; both follow-ups have merged. Hosts v0.3.0 completed
in run 38087761940, and all 16 archive bytes match the checksum file and GitHub
asset digests. Its host adapter is isolated in `module-tinyhosts-host`.

Wallet v0.9.0 committed source `69938a6` but its original run could not push the
tag. The exact release commit was tagged successfully through the owner remote.
Recovery run 38088958171 skipped the bundle job after the skipped versioning
ancestor; [Wallet #61](https://github.com/tinyhumansai/tinywallet/pull/61) adds the
missing recovery condition and a regression. Run 38089224879 uses the release-tag
workflow to build the existing tag and completed successfully. All eleven
supported-platform archives and the checksum file match their GitHub asset
digests and checksum entries. The older tagged workflow does not produce the
five additional distro bundles; the next complete wallet-operations release
must use the current distro-inclusive workflow before host adoption. Wallet #61
merged with 35 passing hosted checks and approval.

Wallet #62 adds module-owned Web3 quotes/execution and x402 budgets/payments.
Its ordinary hosted coverage gate still needs additional service lifecycle
fixtures. The host's native wallet also calls `WalletEngine` for balances,
network and asset catalogues, transfer preparation/execution, and transaction
status, receipt and lookup operations. Those need a further owning-module
contract and implementation change before the wallet implementation dependency
cut; releasing #62 alone will not complete that migration.

The current base also contains unrelated source/artifact pin drift for computer,
search, docs, runtime/providers, MCP and Box. The module-pin gate reports this;
it is not treated as a passing check or hidden by new exceptions. The stale
Wallet and Channels pin exemptions were removed because those base pins now
agree. TinyRuntime changes remain excluded from this migration.

TinyJuice [#59](https://github.com/tinyhumansai/tinyjuice/pull/59) merged as
`c3288ed1e5f30e52d22f634f8f00ac66322e67d9` after incorporating the latest
canonical base. Fresh locked all-feature tests passed (739 tests and doctests),
and a separately run compiled-artifact TinyBus test passed. Both final hosted
Rust jobs passed, with approval and zero unresolved threads. Minor release run
38091415272 succeeded. The eleven v0.7.0 platform archives have been downloaded
and verified against both published checksums and GitHub asset digests. The
source pin is `b89632a8f7b96580a9f6718d185eb60b14a41a8a`; HTML and query
adapters now use the module. The isolated `module-tinyjuice-host` checkout owns
the turn-bound summary callback registry using the existing callback DTO.
All 66 Juice tests pass with the released native library configured through
OpenHuman's pinned loader, including cached-handle queries, supplied-content
queries, HTML extraction and resumed frozen tool declarations. Resume retains
recorded executors when current REPL flags are off, preserves the recorded
schemas verbatim, and does not add live declarations to a frozen subset or
empty snapshot. Disabled module loading still fails the retained executor
explicitly. The library checks pass both without the loader and with the
repository-pinned Rust compiler.

The TinyJuice cut removes its host implementation exception. The transitional
audit now reports zero violations, zero stale exceptions, 30 remaining temporary
exceptions and two pending contracts. Its 22 checker tests pass. The measured
flows profile contains 328 packages, 306 names and two native builds, seven
fewer packages and names than the connector integration profile. This does not
establish a build-time or binary-size improvement.

The TinyRuntime scope exclusion removes its temporary exception from this
branch's acceptance count (31 remaining exceptions before the next module
cut). This is an inventory scope change, not a dependency reduction or a
TinyRuntime implementation change.

## Released cutover integration (2026-10-11)

The `module-released-cutover` branch integrates the prepared foundation,
Connectors, Hosts, Juice, and document adapters onto OpenHuman main
`9f46749f173263487116b0fb8bf3ddefccb3b5b2`. Search is already on that base.
The existing TinyRuntime removal is preserved; runtime worker migration and
further bus error-reporting work remain outside this integration.

The host compiles TinyConnectors v0.14.0, TinyHosts v0.4.0, TinyJuice v0.7.1,
and TinyDocs v0.2.0 contracts and executes their affected operations through
released modules. The module boundary audit reports zero violations and zero
stale exceptions, with 27 temporary exceptions and one pending contract.
The migration is therefore still incomplete. The flows dependency simulator
measures 326 packages, 304 unique crate names, and two native builds
(`libsqlite3-sys`, `ring`), compared with current main's 337 packages and
315 names. Package counts do not establish build-time or binary-size savings.

The full pin gate still reports pre-existing Computer and MCP source/artifact
drift. Their pending host cutovers must reconcile these pins; no new exception
hides this mismatch. All eleven Juice v0.7.1 archive bytes were independently
verified against the host registry, release checksum manifest, and GitHub asset
digests. Local Docs and Hosts archives also match the registry SHA-256 pins.

## Computer accessibility cutover (2026-10-11)

The Computer host cutover pins TinyComputer v0.11.0 at source
`8f50df63e46455a86f2b57f295f86ab8569df99e`, compiling contract 2.12.
All sixteen release archives were downloaded and verified against both the
published checksum manifest and GitHub asset digests. The published Linux
artifact loads through the real TinyBus host. The Linux registry now covers
permission probes as well as the existing macOS and Windows surfaces.

OpenHuman retains its one-shot and continuous microphone permission policies,
focus capture for later target validation, and push/tap activation policy.
Permission queries, permission requests, confidential focus/paste/validation,
and the native Globe listener execute inside the module. An owned listener
task recovers uncertain start replies and retains its admission guard until
cleanup succeeds. Acknowledged batches and a bounded host queue preserve
ordering; overflow resets activation until a physical release is observed.

The shipped and voice feature graphs shed `tinycomputer-accessibility`,
`arboard`, and `enigo`. The boundary audit has zero violations and zero stale
exceptions, with 26 temporary exceptions and one pending contract. The flows
profile measures 325 packages and 303 names; the all-feature profile measures
513 packages and 471 names, nine fewer of each than the preceding cutover.
Both profiles still contain the two native builds `libsqlite3-sys` and `ring`.
These counts do not measure binary size or build time. TinyVoice capture and
non-macOS hotkeys remain a separate host migration.

Linux packaging explicitly stages `libxdo.so.3` and `libasound.so.2` for loaded
modules in both AppImage layouts, checking the target ELF architecture before
copying. Final artifact validation requires these libraries independently of
the host binary's NEEDED entries. Debian declares the ALSA runtime alternatives
as well as libxdo3. Packaging fixtures cover missing libraries, invalid source
ELFs, repeated staging, and both launcher layouts.

Verification passed 51 focused Rust tests, including the published native
artifact and loader-disabled errors. Globe fixtures cover startup cancellation,
a lost start reply, retried cleanup, ordering, and overflow recovery. The
AppImage regression first failed on the obsolete host libxdo NEEDED requirement
and passes after explicit module runtime staging. The full pin audit now reports
only the existing MCP source/artifact drift, pending its separate cutover.

## Module-owned channel health checks

TinyChannels v0.1.18 contains the merged upstream health operation
([tinychannels#67](https://github.com/tinyhumansai/tinychannels/pull/67)).
`doctor channels` sends hydrated configuration and the host proxy policy over
a confidential bus call. The module builds providers and checks each with a
ten-second budget without starting listeners; OpenHuman formats the returned
observations. An unavailable module fails the requested check explicitly.

The desktop lockfile also catches up the already-merged Wallet v0.13.0 pin.
Its pure contract removes the two stale contract exceptions; eleven host
implementation exceptions remain. Runtime channel dispatch and relay delivery
still need migration.

TinyVoice capture uses released v0.1.16 (contract 1.5), including owner-computed
recording summaries and cancellation that waits for native finish cleanup. The
host holds opaque reservations, reads WAV output in
256 KiB slices, polls at most two continuous chunks per request, and releases
leases on cancellation, sign-out and shutdown. Failed cleanup stays registered
for retry. Microphone authorization remains a TinyComputer call; the host does
not link TinyVoice or CPAL. The existing hotkey adapter remains module-backed.

TinyVoice v0.3.0 (contract 1.10) owns wake-word sequencing (`SequenceUtterance`),
the pre-transcription recording gates (`GateRecording`) and the hotkey recording
state machine (`SessionOpen` / `SessionAdvance` / `SessionStatus` /
`SessionClose`). The always-on adapter and `voice/server` call them by contract
constant; the host keeps only native recording setup, transcription, text
delivery and the Globe composition, and reports each fact to the session and
performs the one action it answers with (openhuman#7587).

This cut removes the `tinyvoice` and `cpal` host exceptions, leaving nine
implementation exceptions for Channels, MCP, Box and Wallet. The resolved
voice-only normal/build graph falls from 323 packages to 318; this is a
dependency count, with no inferred binary size or build-time reduction.

## Scoped channel runtime prerequisites

Upstream [tinychannels#68](https://github.com/tinyhumansai/tinychannels/pull/68)
moves listener reconnect supervision into the module.
[tinychannels#69](https://github.com/tinyhumansai/tinychannels/pull/69) adds
inert runtime reservations and private provider/callback objects. Runtime
activation and cleanup remain owned by the module when a caller cancels.
Each private object retains its original callback destination through shutdown;
stopping one runtime cannot close another identity's provider with the same name.

Released v0.1.20 supplies contract revision 10 and operation version 1.11.
All eleven archives and the checksum manifest were verified against downloaded
bytes, GitHub asset digests and asset sizes. The released Linux artifact passed
a runtime-isolation and cleanup fixture using OpenHuman's current TinyBus kernel.
The owner workspace passed 1,306 tests and two actual compiled-artifact fixtures.

These are upstream prerequisites. The host provider, relay and delivery adapters
still need migration; both Channels implementation exceptions remain until
those callers and their dependency graphs are cut over.


## Channel message preparation and hosted ingress

The host now pins verified TinyChannels v0.1.26 (contract 16, operation 1.17).
All eleven published archives and their checksum manifest were checked against
actual downloaded bytes, GitHub digests, and sizes. Published Linux module
fixtures pass with both the owning loader and OpenHuman's shipped TinyBus kernel.
They exercise confidential inbound/identity preparation and isolated runtime
shutdown.

Rich outbound normalization and idempotency preparation execute through
`PrepareOutboundIntent`; hosted inbound conversations obtain their existing
persisted identifiers through `PrepareChannelIdentity`. Disabled or unavailable
modules fail the affected operation. Channel session cancellation and product
configuration remain host-owned. Unreachable host-side segmented delivery was
removed without changing the active direct-delivery path.

Focused host validation passes 225 channel tests, eight controller adapter tests,
and fourteen module registry tests. Boundary enforcement reports zero violations
and zero stale exceptions, with nine temporary implementation exceptions still
present across Channels, MCP, Box, and Wallet. Both Channels implementation
exceptions remain pending provider, relay, remote-command, and progressive
runtime cutover.

Hosted progressive replies now run through a private runtime's `ExecuteProgressive`
handle. OpenHuman supplies ordered turn facts and captured managed transport;
the module owns delivery choreography and idempotency. A released-artifact host
fixture passes callback isolation, acknowledged stop, and final-send replay,
alongside four lease validation and cancellation tests. Native providers, relay,
remote-command host adapters and context helpers are still being migrated;
both Channels implementation exceptions remain.

Follow-up ([#7559](https://github.com/tinyhumansai/openhuman/issues/7559)): the
host per-sender history cache now stores the bus `ChatMessage` and calls
`tinychannels_bus::context::{clear_sender_history, compact_sender_history}`
directly; `TranscriptMessage` is built only at the harness boundary in the
message processor. `channels/context.rs` keeps just the host-only
`ChannelRuntimeContext` and the module-call helpers.
`spawn_scoped_typing_task` (`channels/runtime/dispatch/helpers.rs`) stays as a
minimal host adapter: it needs `tokio::spawn`/cancellation, which the
synchronous bus contract cannot hold, and it only drives the module's
`begin_typing_refresh`/`end_typing_refresh`/`stop_typing` members. The
`[components] channels = false` toggle is pinned in `tests/components_e2e.rs`.


## Full Channels host cutover validation

The native-provider, relay, remote-command, typing, approval presentation and
progressive-delivery adapters now use the minimal bus contract. Provider
implementations and compatibility wrappers are available only to unit tests.
The root contract fixture links the owning module in dev-dependencies and
communicates with it over TinyBus; shipped hosts never link that fixture.

The dependency checker reports zero violations, zero stale exceptions, four
remaining temporary exceptions (MCP and Wallet), and no pending contracts.
`scripts/assert-shed.sh channels tinychannels tinychannels-runtime
tinychannels-module` passes. The dependency simulator reports 308 resolved
packages / 284 package names for `--no-default-features --features channels`,
compared with 346 packages / 318 names in the primary base checkout. These
are dependency counts, not measured build-time or binary-size improvements.

The focused host tests and build are queued for final validation. Earlier
225-test channel results above describe the earlier adapter revision and do
not establish acceptance of this full cutover.


## TinyWallet wallet, Web3, and x402 host cutover

TinyWallet v0.20.0 (contract 1.14) is pinned at released source
`f596ed90dc5de8d6cc7d0a5c375b6e555d0283c2`. Wallet setup, mnemonic validation
and derivation, transaction construction and signing, Web3 quote execution,
ERC-20 calldata encoding, and x402 payment parsing, budgets and ledger now run
through the compiled module. OpenHuman keeps encrypted-key custody, authenticated
HTTP/RPC transport, explicit quote/payment approval and user-scoped lifecycle
management. The host has no normal/build dependency on
`tinywallet-crypto`, `tinywallet-web3` or `tinywallet-x402`; the wallet module
implementation remains only a test fixture dependency.

The v0.20.0 release provides 17 platform archives. Independent local verification
matched every downloaded archive against its GitHub digest and the embedded
`modules.toml` hash; the released Ubuntu artifact also passed the TinyBus
operation and signing E2E. The module-boundary audit reports zero violations
and no remaining Wallet implementation exceptions. TinyRuntime remains excluded
from this migration.

## Current delivery status

Channels host PR [#7502](https://github.com/tinyhumansai/openhuman/pull/7502) and MCP host PR [#7503](https://github.com/tinyhumansai/openhuman/pull/7503) are merged. Wallet PR [#7489](https://github.com/tinyhumansai/openhuman/pull/7489), TinyBox PR [#7469](https://github.com/tinyhumansai/openhuman/pull/7469), and the MCP lifecycle follow-up [#7515](https://github.com/tinyhumansai/openhuman/pull/7515) are also merged.

Final scoped acceptance passed on Linux: the MCP focused suite passed 165 tests, including all 22 lifecycle tests; registry tests passed 14/14; Channels passed 254 focused tests and a released-artifact runtime fixture; TinyBox passed its released-artifact fixture; MCP's released-artifact registry E2E passed 6/6; thread-store tests passed 22/22; and the embed lifecycle test passed. Default and product CLI builds, minimal and loader-disabled checks, the dependency-boundary audit, crate-chain check and feature-forwarding check also passed. These results do not claim runtime testing on every supported platform.

TinyRuntime is excluded as requested. Broader bus error reporting and Sentry coverage are deferred to separate work; the existing reporting path remains in place.

## TinyMCP compatible release pin

The registry pins TinyMCP v0.14.3, built from source
`10b03d1ab00ecbd08bd41d5e83a5c350cbd7e357`. All 17 platform archives match the
published checksum manifest, GitHub asset digests and sizes. The downloaded
Ubuntu artifact loads through OpenHuman's TinyBus revision and exposes the
expected 62-member interface (module descriptor contract 1.21). Registry
metadata includes the Arch ARM archive and the renamed Arch archive basenames.

The final Linux host focused tests and product builds passed; release
verification was also checked separately against all 17 published archives.
Actual-source HTTP/session regressions passed 11 tests covering host
credential-header removal, terminal callback draining, lost-reply retry and
rejected DELETE preservation.

## TinyFlows generic flow ops (library, v0.8.15)

`flows/ops` no longer carries graph logic that is generic over a
`WorkflowGraph`. Trigger labels and warnings (`tinyflows::triggers`), graph
validation and migration (`tinyflows::authoring`), the split-out schema check
(`tinyflows::wiring`), validation error mapping, the approval manifest walk and
trust split, and run lifecycle rules (`tinyflows_catalog::{validation,
approval_manifest, run_management}`) are called from the library directly. The
trail-off fallback is `tinyflows_copilot::fallback`; the host keeps only the
adapter that correlates harness history into its `ToolResult`. The host keeps
approval classification (`SecurityPolicy`), Composio schema lookup, and the
store, checkpoint, bus and cron effects. TinyFlows is a plain library gitlink,
so there is no registry record or `module-boundaries.json` entry.

### TinyFlows builder tool belt and observability mappers (#7554, #7557, v0.8.16)

The workflow-builder tools (`revise`, `edit`, `validate`, `dry_run`, `create`,
`duplicate`, `save`, `resume_flow_run`, `cancel_flow_run`, the flow and run
reads, `list_agent_definitions`, `list_node_kinds`, `get_node_kind_contract`
and `suggest_workflows`) are `tinytools::Tool`s in `tinyflows_copilot::tools`
over the ports `DraftStore`, `FlowStore`, `BuilderGates`, `AgentDirectory`,
`NodeKindSource` and `SuggestionStore`. `flows/builder_host.rs` is the one
implementation of those ports (`FlowsHost`, plus `OverlayNodeKinds` for the
host node-kind notes) and `flows/registration.rs` constructs the tools over it.
Tool names, descriptions, parameter schemas, permissions and the
`external_effect` flag are pinned byte-for-byte by
`flows/registration_tests/flow_tools.snap.json`, recorded before the move.
`edit_op_shape`, the node-kind line renderers
(`tinyflows::catalog::render_node_kinds_{line,required}`) and
`agent_prompt::resolve_completion_model` are upstream; the dedup exactly-once
settle contract was already `tinyflows::nodes::control_flow::dedup_settle`, so
only the per-flow lock and the run-finished subscriber remain in
`flows/bus/dedup_commit.rs`. `observability.rs` and `langfuse_export.rs` call
`tinyflows::observe::{step_status_str, to_exporter_observations}`.

Stays host: `search_tool_catalog`, `get_tool_contract`,
`get_tool_output_sample`, `list_connectable_toolkits` and
`list_flow_connections` (bound to the live Composio catalog, its probe cache and
the connected accounts). `tinyflows-copilot` is `publish = false` because it
names `tinytools` by git rev; OpenHuman's `[patch]` redirects it to the
tinyagents copy, so there is one `Tool` trait.

### TinyFlows builder gate stack (#7551)

`flows/ops/builder_gates.rs` no longer orders the author-time gates. `tinyflows::gates::builder_failures` runs compatibility, saved-workflow, portable authoring and binding gates, then the host stages, then the mock-run argument preflight. The host implements `BuilderGateHost`: `ToolPolicy` (workspace-relative upload paths), `AgentReferences`, `Connections` and `ToolContracts`, plus the saved-workflow lookup. `validate_binding_resolvability` and `validate_required_arg_resolvability` are deleted. Provider connectivity remains an advisory proposal field, not a stage.

### TinyFlows schedule runtime (#7560)

The cron scheduler runtime is `tinyflows_schedule::runtime::Scheduler` (v0.8.15): the poll pass, bounded concurrency, process-wide `RunClaim`s, retry with backoff, rescheduling, single-flight skips and the `cron_job_policies` fold into `CronJob::policy`. `parse_human_delay`, `default_delivery`, `check_origin_requirements` and the delivery-validation rules are `tinyflows_schedule::validation`; the host adds only the channel `allowed_users` lookup.

Deleted from `crates/openhuman-core/src/cron/`: `scheduler/{dispatch,slot,retry}.rs`, the persistence half of `scheduler/run_record.rs`, `policy.rs` and its tests, the core `parse_human_delay`, `validate_delivery`, `check_origin_requirements` and `default_delivery`, and the `DomainEvent::FlowScheduleTick` round trip (`EVENTS_VERSION` 1.11.0). The tests of the moved behaviour (claims, concurrency, single-flight, retry budget, one-shot completion) live in `tinyflows-schedule`.

The host keeps the ports in `scheduler/{host,executor,run_delivery}.rs`: `JobStore` over `CronDocuments` (with operator limits), `JobExecutor` (shell and agent attempts, failure classification, the retries-exhausted report), `RunDelivery` (delivery, completion and health events) and `TriggerSink` (the flow's schedule trigger through `FlowTriggerSubscriber::dispatch_schedule_tick`, and `system:<name>` jobs). `platform::llm_gate` (moved out), system jobs, seeding and the `cron.*` RPC stay. TinyFlows is a plain library gitlink, so there is no registry record or `module-boundaries.json` entry.

Behaviour notes: a job's policy is read from the job document (an unreadable job refuses Run Now with a credential-free message, `ops::load_job_for_run`); `CronJobCompleted` and the scheduler health signal are published from delivery, before the runtime writes the completion; `CronJobTriggered` is no longer published for `flow` rows.

### `[components].flows` toggle (#7564)

`[components] flows = false` (or `OPENHUMAN_COMPONENT_FLOWS=0`) is host-only:
the flows code stays compiled in. On the next core start it narrows
`DomainSet::flows` (no flow trigger or run-digest subscribers, no boot
reconcile), turns the `workflows` tool group off, and answers every `flows.*`
RPC with `COMPONENT_DISABLED:flows`. `about_app` reports the `flows` and
`workflow_runs` capability domains as disabled. TinyFlows is a plain library
gitlink with no registry record, so `modules.list` has no row for it.

Relation to `cron.enabled`: independent switches. With `flows = false` the
cron scheduler keeps running agent, shell and system jobs, but `flow` schedule
rows are answered as skipped (`COMPONENT_DISABLED:flows`, not retried) and no
flow run starts. `cron.enabled = false` stops the whole scheduler, flow
schedule rows included, regardless of `flows`.

### TinyFlows schedule bus cutover (#7397)

Supersedes the section above for who runs the scheduler. The `tinyflows` module (registry record `records_flows.rs`, contract `tinyflows_bus::schedule` v3) owns the scheduler runtime, claims, retry timing, completion, the cron job and run store (`CronDocuments`) and the schedule maths (`NextRun`, `ValidateSchedule`, `RunsCloserThan`, `NormalizeExpression`, `ParseHumanDelay`, `ValidateDelivery`, `DefaultDelivery`, `CheckOriginRequirements`, `CheckPatch`, `TruncateCronOutput`). Core links only `tinyflows-bus` (serde and chrono): `cargo tree -p openhuman -e normal -i tinyflows-schedule` is empty, and `tinyflows-drivers` is optional behind `flows` for the catalog alone.

The host keeps four adapters in `crates/openhuman-core/src/cron/`: `client.rs` (typed calls and `ScheduleStore` requests), `documents_port.rs` (the scope's document store served to the module), `bus_host.rs` (the `ScheduleHost` callbacks `Execute`, `Failed`, `Deliver`, `ScheduleTrigger`, bound per scope) and `handlers.rs` (the job-kind registry that replaced the hard-coded shell/agent/flow split and absorbed the system-job handlers). Deleted: `claims.rs`, `scheduler/host.rs`, `scheduler/run_record.rs` and the `tinyflows_schedule` re-export facade. Temporary exception: `tinyflows-bus` carries `chrono` (record timestamps), recorded in `scripts/ci/module-boundaries.json`.
