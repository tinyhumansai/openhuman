# Loadable module boundary inventory

Status: migration incomplete. Completed adapters and the remaining linked
implementation packages are recorded below; the exception list is still nonempty.
The machine-readable dependency policy is
[`scripts/ci/module-boundaries.json`](../scripts/ci/module-boundaries.json).

Every owner below is the canonical upstream under
`https://github.com/tinyhumansai/`. Contract changes land there first. OpenHuman
host adapters switch only after compatible per-platform artifacts are released
and their digests pinned. Do not remove a dependency by removing its capability,
copying its implementation into a host, or accepting weaker isolation.

| Owner | Linked implementation packages | Contract | Host consumers | Required module work |
| --- | --- | --- | --- | --- |
| tinyjuice | none (module loaded through TinyBus) | tinyjuice-bus | core tools, compression, CCR REPL | Complete: PR #59 provides module-owned compression, CCR queries, HTML extraction, schemas and declarations; the host retains turn-bound model callbacks and artifact authorization |
| tinyconnectors | none | tinyconnectors-bus | core integrations, credentials and triggers | Module-backed argument preparation, action execution and trigger archives; host credential reconciliation and frozen tool restoration remain in OpenHuman |
| tinymcp | tinymcp | tinymcp-bus | core MCP registry, supervisor, CLI stdio/HTTP server | Supervisor notifications, server protocol operations and callbacks for approved host tools/resources/prompts |
| tinychannels | tinychannels, tinychannels-runtime (remaining runtime callers) | tinychannels-bus | core channels, TinyHumans host, CLI REPL | Contract is pure; v0.1.18 supplies complete Discord response vocabulary provider connection preparation and health observations. Discord discovery/permission checks, credential tests and connection validation/defaults and `doctor channels` health checks use confidential module calls. Remaining runtime callers need bus cutover with inbound/status callbacks and shutdown draining |
| tinyhosts | none | tinyhosts-bus | core hosting tools | Released Execute/Providers contract consumed through the module; workspace authorization, credentials and approval policy remain in OpenHuman |
| tinywallet | tinywallet-crypto, tinywallet-web3, tinywallet-x402 through remaining host imports | tinywallet-bus | core wallet/web3/x402 | Contract is pure at v0.13.0; native transfer preparation/execution use confidential module calls with host custody, exact approval binding and explicit close. Initialization, remaining validation, Web3 quotes/swaps, x402 payments, budgets and ledger still need host cutover |
| tinybox | tinybox-core, tinybox-jail, tinybox-docker | tinybox-bus | core sandbox and security, desktop gateways | Pre-core gateways use the released v0.1.18 bus client. Core sandbox execution and shell analysis still require cutover |
| tinycomputer | none | tinycomputer-bus | core voice, paste/focus and permission checks | Permission, focus, paste and Globe listener operations use the module; native thread and target rules remain covered |
| tinyvoice | None | tinyvoice-bus | core voice capture/hotkeys | Module-owned devices, recording and continuous capture, bounded audio reads/batches, explicit lease cleanup; computer module permission decisions |
| tinyruntime | tinyruntime-pyserver | tinyruntime-bus | core Python worker, optional TinyJuice ML | Excluded from this migration: TinyRuntime is being removed separately |
| tinydocs / tinymemory | none | tinydocs-bus | core memory converter and file sources | Bus DocumentConverter replaces OfficeConverter, including XLSX and existing PDF/DOCX/PPTX format coverage; ingestion metadata remains host-owned |
| tinysearch | none observed | tinysearch-bus | core module search proxy | Preserve existing bus adapter; enforce contract and host dependency graphs |

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

The shared `openhuman_rpc::embed::modules::ModuleClient` takes explicit runtime
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
| TinyBox pure contract, shell facts, reserved sandbox/process handles and acknowledged cleanup/shutdown | [tinybox#30](https://github.com/tinyhumansai/tinybox/pull/30) | 710 default and all-feature tests; 57 source files at least 90% coverage; compiled native-process artifact verification; independent lifecycle review accepted; supervised execution currently limited to Unix passthrough |
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

TinyBox currently refuses supervised execution on unsupported backends and platforms.
Docker supervision is implemented in PR 30 and still undergoing lifecycle review.
Namespace/SSH and native Windows supervision, streaming, file transfer, forwarding
and gateway operations remain required before migrating its host callers.

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

The host compiles TinyConnectors v0.14.0, TinyHosts v0.3.0, TinyJuice v0.7.1,
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

This cut removes the `tinyvoice` and `cpal` host exceptions, leaving nine
implementation exceptions for Channels, MCP, Box and Wallet. The resolved
voice-only normal/build graph falls from 323 packages to 318; this is a
dependency count, with no inferred binary size or build-time reduction.
