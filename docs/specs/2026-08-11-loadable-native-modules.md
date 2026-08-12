# Loadable native modules

Status: Implemented (`documents` is the first consumer)

## Problem

A codec is not kernel work, and each one drags a tree of parsers into a binary
that mostly does something else. The document surface was the clearest case: the
`documents` gate carried `docx-rs`, `ppt-rs` and `pdf-extract` plus their font,
PostScript and XML tails — 39 crates, including four native C builds — to support
three agent tools.

Gating them helped the builds that turned them off and did nothing for the build
that ships, which turns them on. What was needed was a boundary that survives
compilation: the capability present, the dependencies absent.

## Goals

- Run a capability as a compiled artifact loaded at runtime, outside this binary.
- Verify what gets loaded, from a decision made at build time rather than by
  whatever a release page serves.
- Keep the tool surface, the tool schemas and the agent-facing errors unchanged.
- Leave the kernel profile untouched: a host embedding workflows must not acquire
  a dynamic-library loader.

## Non-goals

- Running untrusted third-party code. A module is first-party code that ships
  separately; anything untrusted belongs in a process.
- A module marketplace, or any path by which a server or a config file can name
  an artifact to load.
- Unloading. tinybus never unloads a library, and nothing here pretends otherwise.

## Behaviour

`openhuman::modules` owns a private in-process broker, a tinybus `ModuleHost`, and
a compiled-in registry. A registry entry names a module's id, the interfaces it
claims, its release tag, and one SHA-256 per published artifact.

On first use of a capability a module provides, `ops::ensure_loaded` resolves it —
already serving, then a configured local artifact, then the install directory,
then the tinybus module search path, then a verified download — and caches the
outcome for the process. Loaded, the module is an ordinary bus peer, and the core
calls it over a proxy.

Inbound payloads ride tinybus streams alongside the call. Outbound payloads are
held by the module and pulled in chunks, because a served object cannot open a
stream back to its caller.

Config (`[modules]`) controls whether modules load, whether this host may fetch
them, where they install, and whether a local build stands in for a release. It
cannot add a module.

## Invariants and constraints

- **A loaded module is trusted in-process code.** It shares the address space, the
  privileges and the crash domain. Deadlines, bounded queues and caught panics
  contain ordinary misbehaviour, not a segfault. The ABI, manifest and digest
  gates decide what is *admitted*, never what is *safe*.
- **The set of loadable modules is compiled in.** A registry that config or RPC
  could extend would be remote code execution with a download step.
- **Digests are the host's half of a two-sided check.** tinybus fetches the
  release's own checksum manifest, compares it with the pinned digest, hashes the
  download, and extracts only after. A release re-cut under the same tag stops
  matching rather than silently replacing what runs in-process.
- **Failure is terminal for the process.** tinybus never unloads, so a refused or
  faulted module cannot reach a different outcome without a restart. Failures are
  cached and say so.
- **Admission is permissive, not strict.** Strict mode also refuses a module whose
  rustc version differs from the host's, and released artifacts are built on
  whatever toolchain CI had while this crate pins its own — the real published
  artifact is refused that way. Everything protecting the address space is still
  enforced; only the toolchain string is relaxed.
- **A target triple is not enough.** A library built against glibc 2.39 fails to
  `dlopen` on a 2.35 host. Artifact selection is an ordered list of candidates,
  glibc-aware, and empty on a host no published artifact targets.
- **Modules cannot publish `DomainEvent`s.** `OnceBus::init_in_process` owns its
  broker privately, so modules run on a second one.
- **The loader stays out of the kernel profile.** `tinybus/modules` is forwarded
  from this crate's `modules` feature, never enabled on the dependency, because
  `tinybus` is always-on surface.

## Acceptance criteria

- The `documents` gate carries no codec: `docx-rs`, `ppt-rs`, `pdf-extract`,
  `lopdf`, `syntect`, `pulldown-cmark` and `xml-rs` are absent from both the
  `documents` and the product profiles, proven with `scripts/assert-shed.sh`.
- The kernel profile is unchanged against upstream: `scripts/kernel-floor.sh
  flows` reports the same packages, names and native builds.
- `modules.*` is unknown-method with the feature off, and both halves are pinned
  by tests.
- The three document tools produce openable artifacts through a loaded module,
  including a deck whose images cross as a stream.

## Testing notes

**The module bus belongs to whichever runtime creates it.** The core has one
runtime and never notices. Two `#[tokio::test]` functions each build their own,
and the second to call a loaded module finds a broker whose tasks died with the
first — the call hangs until a deadline above it fires, rather than failing.

Any test that drives a real module must therefore be the only one in its process.
The module-backed tool tests are `#[ignore]`d for that reason, not merely because
they need an artifact. Run them one at a time:

Linux, where the artifact is a `.so`. On macOS the built library is
`libtinydocs_module.dylib` and on Windows `tinydocs_module.dll`; substitute the
filename, and use the platform's own temporary directory rather than `/tmp`.

```sh
cargo build --release --package tinydocs-module \
  --manifest-path vendor/tinydocs/Cargo.toml
mkdir -p /tmp/oh-modules && chmod 700 /tmp/oh-modules
cp vendor/tinydocs/target/release/libtinydocs_module.so /tmp/oh-modules/

OPENHUMAN_MODULE_PATH=/tmp/oh-modules \
  cargo test -p openhuman --lib --features documents -- --ignored \
  implementations::document::tests::execute_happy_path
```

## Open questions

**A reply-stream seam in tinybus would delete the module's output store.** The
only reason a produced document is held at all is that `Interface::call` receives
no caller identity and no connection, so a served object cannot stream a reply.

**Per-interface method lists in `module_export!`** would let one module serve a
transfer interface and a format interface separately. Today the macro attaches its
method list to the first entry in `provides` and leaves the rest empty, so a
second fully-declared interface is not expressible.
