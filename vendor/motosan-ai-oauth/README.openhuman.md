# OpenHuman patch

This directory contains the source of the official `motosan-ai-oauth` 0.2.1
crate from crates.io:

- Download: <https://static.crates.io/crates/motosan-ai-oauth/motosan-ai-oauth-0.2.1.crate>
- SHA-256: `244943168db97b6874d3a88f5fff7029ab002d84c9c572e3bfe6e8cd92af0bae`

To reproduce the source verification, download that archive and verify it with
`sha256sum`. This vendored copy deliberately retains only the provider enabled
by OpenHuman (`codex`); the unused Gemini and Anthropic provider modules are
omitted. In particular, the Gemini module distributed public installed-app
OAuth credentials which OpenHuman neither compiles nor uses.

The other source delta is in `Cargo.toml`: unused provider features are removed
and the crate's `reqwest` dependency sets `default-features = false`. The
released manifest enables both reqwest's default native-TLS backend and
`rustls-tls`, which brings OpenSSL into Linux builds. OpenHuman already chooses
its TLS backend by target: rustls on Linux/macOS and native TLS on Windows.

Remove this patch after an upstream release includes the same manifest change.
