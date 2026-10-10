//! Compile-time capabilities of the core, independent of facade feature forwarding.

/// Every Cargo feature gate and whether this core artifact includes it.
pub fn compiled_features() -> std::collections::BTreeMap<String, bool> {
    [
        ("storage-sqlite", cfg!(feature = "storage-sqlite")),
        ("storage-mongodb", cfg!(feature = "storage-mongodb")),
        ("storage-file", cfg!(feature = "storage-file")),
        ("default", cfg!(feature = "default")),
        ("http-server", cfg!(feature = "http-server")),
        ("inference", cfg!(feature = "inference")),
        ("documents", cfg!(feature = "documents")),
        ("hosting", cfg!(feature = "hosting")),
        ("tinymemes", cfg!(feature = "tinymemes")),
        ("modules", cfg!(feature = "modules")),
        ("voice", cfg!(feature = "voice")),
        ("web3", cfg!(feature = "web3")),
        ("media", cfg!(feature = "media")),
        ("flows", cfg!(feature = "flows")),
        ("skills", cfg!(feature = "skills")),
        ("mcp", cfg!(feature = "mcp")),
        ("crash-reporting", cfg!(feature = "crash-reporting")),
        ("channels", cfg!(feature = "channels")),
        ("whatsapp-web", cfg!(feature = "whatsapp-web")),
        ("e2e-test-support", cfg!(feature = "e2e-test-support")),
        ("rss-bench", cfg!(feature = "rss-bench")),
        ("file-logging", cfg!(feature = "file-logging")),
        ("scheduler-gate", cfg!(feature = "scheduler-gate")),
    ]
    .into_iter()
    .map(|(name, enabled)| (name.to_owned(), enabled))
    .collect()
}
