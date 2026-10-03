use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePythonBackend {
    /// TokenJuice ML plain-text compressor ("Kompress", ModernBERT via torch).
    Kompress,
}

impl RuntimePythonBackend {
    pub fn id(self) -> &'static str {
        match self {
            Self::Kompress => "kompress",
        }
    }
}

pub fn enabled_backends(config: &Config) -> Vec<RuntimePythonBackend> {
    if !config.runtime_python.enabled {
        return Vec::new();
    }

    let mut backends = Vec::new();
    if config.tokenjuice.ml_compression_enabled {
        backends.push(RuntimePythonBackend::Kompress);
    }
    backends
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
