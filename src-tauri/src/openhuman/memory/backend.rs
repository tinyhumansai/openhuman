#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MemoryBackendKind {
    Sqlite,
    Lucid,
    Postgres,
    Markdown,
    None,
    Unknown,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct MemoryBackendProfile {
    pub key: &'static str,
    pub label: &'static str,
    pub auto_save_default: bool,
    pub uses_sqlite_hygiene: bool,
    pub sqlite_based: bool,
    pub optional_dependency: bool,
}

const SQLITE_PROFILE: MemoryBackendProfile = MemoryBackendProfile {
    key: "sqlite",
    label: "SQLite with Vector Search (recommended) — fast, hybrid search, embeddings",
    auto_save_default: true,
    uses_sqlite_hygiene: true,
    sqlite_based: true,
    optional_dependency: false,
};

const LUCID_PROFILE: MemoryBackendProfile = MemoryBackendProfile {
    key: "lucid",
    label: "Lucid Memory bridge — sync with local lucid-memory service, keep SQLite fallback",
    auto_save_default: true,
    uses_sqlite_hygiene: true,
    sqlite_based: true,
    optional_dependency: true,
};

const MARKDOWN_PROFILE: MemoryBackendProfile = MemoryBackendProfile {
    key: "markdown",
    label: "Markdown Files — simple, human-readable, no dependencies",
    auto_save_default: true,
    uses_sqlite_hygiene: false,
    sqlite_based: false,
    optional_dependency: false,
};

const POSTGRES_PROFILE: MemoryBackendProfile = MemoryBackendProfile {
    key: "postgres",
    label: "PostgreSQL — remote durable storage via [storage.provider.config]",
    auto_save_default: true,
    uses_sqlite_hygiene: false,
    sqlite_based: false,
    optional_dependency: false,
};

const NONE_PROFILE: MemoryBackendProfile = MemoryBackendProfile {
    key: "none",
    label: "None — disable persistent memory",
    auto_save_default: false,
    uses_sqlite_hygiene: false,
    sqlite_based: false,
    optional_dependency: false,
};

const CUSTOM_PROFILE: MemoryBackendProfile = MemoryBackendProfile {
    key: "custom",
    label: "Custom backend — extension point",
    auto_save_default: true,
    uses_sqlite_hygiene: false,
    sqlite_based: false,
    optional_dependency: false,
};

const SELECTABLE_MEMORY_BACKENDS: [MemoryBackendProfile; 4] = [
    SQLITE_PROFILE,
    LUCID_PROFILE,
    MARKDOWN_PROFILE,
    NONE_PROFILE,
];

pub fn selectable_memory_backends() -> &'static [MemoryBackendProfile] {
    &SELECTABLE_MEMORY_BACKENDS
}

pub fn default_memory_backend_key() -> &'static str {
    SQLITE_PROFILE.key
}

pub fn classify_memory_backend(backend: &str) -> MemoryBackendKind {
    match backend {
        "sqlite" => MemoryBackendKind::Sqlite,
        "lucid" => MemoryBackendKind::Lucid,
        "postgres" => MemoryBackendKind::Postgres,
        "markdown" => MemoryBackendKind::Markdown,
        "none" => MemoryBackendKind::None,
        _ => MemoryBackendKind::Unknown,
    }
}

pub fn memory_backend_profile(backend: &str) -> MemoryBackendProfile {
    match classify_memory_backend(backend) {
        MemoryBackendKind::Sqlite => SQLITE_PROFILE,
        MemoryBackendKind::Lucid => LUCID_PROFILE,
        MemoryBackendKind::Postgres => POSTGRES_PROFILE,
        MemoryBackendKind::Markdown => MARKDOWN_PROFILE,
        MemoryBackendKind::None => NONE_PROFILE,
        MemoryBackendKind::Unknown => CUSTOM_PROFILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_known_backends() {
        assert_eq!(classify_memory_backend("sqlite"), MemoryBackendKind::Sqlite);
        assert_eq!(classify_memory_backend("lucid"), MemoryBackendKind::Lucid);
        assert_eq!(
            classify_memory_backend("postgres"),
            MemoryBackendKind::Postgres
        );
        assert_eq!(
            classify_memory_backend("markdown"),
            MemoryBackendKind::Markdown
        );
        assert_eq!(classify_memory_backend("none"), MemoryBackendKind::None);
    }

    #[test]
    fn classify_unknown_backend() {
        assert_eq!(classify_memory_backend("redis"), MemoryBackendKind::Unknown);
    }

    #[test]
    fn selectable_backends_are_ordered_for_onboarding() {
        let backends = selectable_memory_backends();
        assert_eq!(backends.len(), 4);
        assert_eq!(backends[0].key, "sqlite");
        assert_eq!(backends[1].key, "lucid");
        assert_eq!(backends[2].key, "markdown");
        assert_eq!(backends[3].key, "none");
    }

    #[test]
    fn lucid_profile_is_sqlite_based_optional_backend() {
        let profile = memory_backend_profile("lucid");
        assert!(profile.sqlite_based);
        assert!(profile.optional_dependency);
        assert!(profile.uses_sqlite_hygiene);
    }

    #[test]
    fn unknown_profile_preserves_extensibility_defaults() {
        let profile = memory_backend_profile("custom-memory");
        assert_eq!(profile.key, "custom");
        assert!(profile.auto_save_default);
        assert!(!profile.uses_sqlite_hygiene);
    }
}
