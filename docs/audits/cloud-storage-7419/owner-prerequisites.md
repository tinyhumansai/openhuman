# Cloud storage owner prerequisites (#7419)

Checkpoint: 2026-10-12. This records owning repository evidence and outstanding
composition work. It does not establish OpenHuman desktop or cloud E2E
acceptance. Earlier local host work remains on the retained
`cloud-storage-7419` branch; this audit branch starts from canonical OpenHuman
`b1c053c4557e816b27b2c5f8eefa6eb1b516590f`.

## Canonical owner milestones

| Owner change                                                                                                                      | Available capability                                                                                                                         | Evidence and limits                                                                                                                                                                                                                                     |
| --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [TinyStorageDrivers #27](https://github.com/tinyhumansai/tinystoragedrivers/pull/27), merged                                      | Minimal document bus client and mechanical adapter over a captured native DocumentStore                                                      | Four actual compiled SDK callback proofs passed. The bridge preserves native CAS and exact callback admission; it accepts no caller-selected storage URL or collection. Local all-feature/default gates passed 705/663 tests respectively.              |
| [TinyBus #59](https://github.com/tinyhumansai/tinybus/pull/59) and [#60](https://github.com/tinyhumansai/tinybus/pull/60), merged | Exact callback revocation/drain and native module replacement with per-instance diagnostic ownership                                         | The replacement proof loads the same artifact as A and B after A stops and both names disappear. B's logging and sanitized panic use B's callback. #60's 13 CI jobs and CodeRabbit passed. This is not a host reset/drain proof.                        |
| [TinyAgents #413](https://github.com/tinyhumansai/tinyagents/pull/413), merged                                                    | Conversation persistence/provider prerequisite, metadata/prefix readers, trusted schema preparation and immutable validate-only construction | Seven schema controls passed after two genuine missing-declaration failures. Local workspace suites passed 5206 all-feature and 4811 default tests, with 36 ignored per graph. Native message writers and authoritative turn claims remain unsupported. |
| [TinyMemes #6](https://github.com/tinyhumansai/tinymemes/pull/6), merged                                                          | Bounded native delivered-marker CAS through the authenticated document callback                                                              | Actual compiled Memory/SQLite proof: 18 passes. Default/all-feature owner suites: 87 passes each. Hosted CI also passed the Bash forwarding regression. This does not implement learning, research, historical import or host composition.              |

Merged source is distinct from a released module. The latest observed TinyBus
release is v0.1.4, published before #60; no TinyMemes GitHub release was found.
The merged TinyBus tree is `40814a8279ff71b8fb411f9f29362c8ec0be51e2`, including
#59 and #60. #60's final head also corrects a test timeout beyond the earlier
local `5256d08c` candidate; the SDK diagnostic implementation is unchanged.

## Coherent runtime dependency prerequisite

Canonical TinyStorageDrivers still pins TinyBus `1f255765`. Selecting the newly
merged runtime in TinyMemes without a replacement therefore creates incompatible
Rust bus types at the adapter boundary.

[TinyStorageDrivers #28](https://github.com/tinyhumansai/tinystoragedrivers/pull/28)
updates only runtime pins, lock sources and a historical evidence note. The
reviewed clean head is `a981da30`. Metadata resolves one TinyBus at `40814a8` in
the driver workspace and one matching SDK in the compiled proof. No replacement,
URL alias or local runtime path supplies this graph.

Fresh actual callback proof: four passes. All-feature/default gates pass
705/663 tests, including the native Mongo fixture. Strict Clippy, builds, Rust
1.88, documentation and formatting pass. Restoring unrelated lock normalization
leaves only the intended source substitutions; final locked metadata, strict
Clippy, build and MSRV checks pass, and independent review finds no issues.
Publication is ready for review; this checkpoint does not claim its merge.

## Next owner capabilities under development

### Native TinyMemes content usage

The next contract accepts business inputs for slang usage, meme usage and meme
removal. Callers cannot supply a stored snapshot, expected business revision,
scope, storage selector or allegedly verified research output. The module must
read the captured native catalog and preserve complete records, history and
duplicate order while applying the existing owning reducer.

Seventeen actual compiled content cases cover Memory/SQLite
preservation, cold reopen, native CAS races, corruption, exhaustion, fencing,
native acknowledgement uncertainty and exact-instance admission. On the new
public #28/canonical TinyBus graph, all 18 existing marker controls pass. The
content baseline executes all 17 cases: two admission controls pass and 15
behavior cases fail against the deliberate unavailable stub, with no setup or
compiler failures. The owning CAS candidate `592b6db` subsequently passes all
35 actual compiled cases (17 content and 18 marker controls), with the baseline
assertions unchanged. Independent source and proof review finds no issues;
broader gates and coverage remain pending. This is not host migration acceptance.
The new graph removes TinyMemes' old replacement shim.

### Native conversation append

Documents support checked atomic batches and streams support conditional
append. They do not expose a cross-port atomic commit. The bounded proposed
protocol retains the existing stream layout and stores a separate durable
pending document while the committed visible prefix remains unchanged.
Final publication must verify the exact row, pending version and live
incarnation before a fenced checked document batch publishes metadata and
retires the reservation. Only writers recover pending work; reads stay
side-effect free.

Putting pending fields directly into schema-1 thread metadata would be unsafe:
older metadata writers ignore and drop unknown fields. A separate prepared
collection avoids that compatibility failure. Ordinary-ID uncertainty must
remain explicit when the original attempt cannot be identified; no hidden UUID
retry or whole-operation exactly-once guarantee follows. This is a reviewed
implementation plan, not an executed append capability or a turn-claim proof.

The owning local implementation branch now has an authentic baseline: two
prepared ExistingSchema Memory/SQLite cases compile successfully and both fail
at the first append with the expected typed Unsupported result. Later assertions
for replay, reopen and statistics are not reached by that baseline. The owning
append implementation and its GREEN evidence remain pending.

## Remaining host and migration work

OpenHuman still directly imports the TinyMemes implementation and persists local
catalogs and markers. A native marker call alongside that old factory would
still create directories and write sibling JSON files. The host needs released
bus contracts, actual compiled module admission, captured actor ports and
durable success before returning a substituted reply. The synchronous history
builder needs admitted asynchronous reads before constructing its temporary
observed marker snapshot.

Historical source ownership and root-retained import receipts, complete native
reset inventory and purge, conversation edit/tail/purge parity, authoritative
turn claims, and the two-core MongoDB/MinIO read-only E2E remain outstanding.
These owner prerequisites do not complete #7419 or its constituent migrations.
