//! Test-only declaration of the Cargo-built `test-mcp-stub` as an
//! operator-trusted stdio server.
//!
//! Registry-installed servers (`mcp.json`, the registry RPCs) are never
//! operator-trusted, so tinymcp starts them inside a TinyBox jail. With
//! tinymcp 0.20.0 the host declares best-effort enforcement, which macOS
//! Seatbelt satisfies, so the tests on the registry path start the stub jailed
//! there and do not need this helper.
//!
//! Linux does not: `mcp::host_sandbox` asks for network access (`allow_net`)
//! together with process isolation, and tinybox v0.2.0's
//! `native_jail::prepare` refuses that by design, since a bubblewrap jail
//! sharing the host network namespace reports `Process`, not `Kernel`. The
//! Registry stdio MCP servers need network, and TinyBox refuses a
//! network-sharing jail for untrusted code until it has isolated networking
//! (https://github.com/tinyhumansai/tinybox/issues/109).
//!
//! The trusted stub is still needed only where a test is not on the registry
//! path: it declares the Cargo-built stub in the operator-owned `config.toml`
//! (`[[mcp_client.servers]]`, `operator_trusted = true`), which tinymcp starts
//! as an ordinary child process without a jail.

#![allow(dead_code)]

/// A `config.toml` fragment declaring the stub as an operator-trusted stdio
/// server called `name`, to append to a test config.
pub fn trusted_stub_toml(name: &str, stub: &str) -> String {
    format!(
        "\n[[mcp_client.servers]]\nname = {name:?}\ncommand = {stub:?}\noperator_trusted = true\n"
    )
}
