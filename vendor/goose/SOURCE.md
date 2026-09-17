# Vendored Goose source

This directory contains the unmodified `goose-agent` and
`goose-provider-types` crates from the Apache-2.0 licensed Goose repository:

- upstream: <https://github.com/block/goose>
- commit: `53672c3f14bbf83959cea3e6fe0132a2e8b800af`
- source paths: `crates/goose-agent`, `crates/goose-provider-types`
- imported: 2026-09-15

The root `Cargo.toml` in this directory is an OpenHuman packaging adaptation.
It narrows the upstream workspace to these two crates while retaining the
upstream dependency versions. `rmcp` is fixed to `3.3.0`, the version selected
by the pinned upstream `Cargo.lock`, rather than accepting any later 3.x
release. The crate source, tests, changelogs, and README files are copied
without modification. The upstream Apache 2.0 license is in `LICENSE`.
