use crate::openhuman::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePythonBackend {
    Spacy,
    /// TokenJuice ML plain-text compressor ("Kompress", ModernBERT via torch).
    Kompress,
}

impl RuntimePythonBackend {
    pub fn id(self) -> &'static str {
        match self {
            Self::Spacy => "spacy",
            Self::Kompress => "kompress",
        }
    }
}

pub fn enabled_backends(config: &Config) -> Vec<RuntimePythonBackend> {
    if !config.runtime_python.enabled {
        return Vec::new();
    }

    let mut backends = Vec::new();
    if config.memory_tree.spacy_enabled {
        backends.push(RuntimePythonBackend::Spacy);
    }
    if config.tokenjuice.ml_compression_enabled {
        backends.push(RuntimePythonBackend::Kompress);
    }
    backends
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_respects_runtime_and_spacy_flags() {
        let mut config = Config::default();
        config.runtime_python.enabled = false;
        config.memory_tree.spacy_enabled = true;
        assert!(enabled_backends(&config).is_empty());

        config.runtime_python.enabled = true;
        config.memory_tree.spacy_enabled = false;
        assert!(enabled_backends(&config).is_empty());

        config.memory_tree.spacy_enabled = true;
        assert_eq!(enabled_backends(&config), vec![RuntimePythonBackend::Spacy]);
    }

    #[test]
    fn registry_includes_kompress_when_enabled() {
        let mut config = Config::default();
        config.runtime_python.enabled = true;
        config.memory_tree.spacy_enabled = false;
        config.tokenjuice.ml_compression_enabled = true;
        assert_eq!(
            enabled_backends(&config),
            vec![RuntimePythonBackend::Kompress]
        );

        config.memory_tree.spacy_enabled = true;
        assert_eq!(
            enabled_backends(&config),
            vec![RuntimePythonBackend::Spacy, RuntimePythonBackend::Kompress]
        );

        // Master runtime switch still gates everything.
        config.runtime_python.enabled = false;
        assert!(enabled_backends(&config).is_empty());
    }
}
