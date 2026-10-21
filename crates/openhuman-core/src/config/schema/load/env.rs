/// Read-only environment lookup used by [`crate::config::schema::Config::apply_env_overrides`].
/// The seam lets unit tests exercise the overlay without mutating the process
/// environment (which is racy under parallel tests and requires a shared
/// `TEST_ENV_LOCK`).
///
/// Production code uses [`ProcessEnv`], which delegates to `std::env`.
pub(crate) trait EnvLookup {
    /// Equivalent to `std::env::var(key).ok()`.
    fn get(&self, key: &str) -> Option<String>;

    /// Equivalent to `std::env::var_os(key).is_some()`. Used to distinguish
    /// "variable not present" from "variable set to empty" where it matters
    /// (see `OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES` below).
    fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Whether a present value was installed by the tracked dotenv load.
    fn is_from_dotenv(&self, _key: &str) -> bool {
        false
    }

    /// Looks up the first non-`None` value across `keys`, preserving the
    /// precedence used by the manual `or_else` chains throughout this
    /// module (e.g. `OPENHUMAN_FOO` wins over the bare `FOO` alias).
    fn get_any(&self, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| self.get(k))
    }
}

/// Default [`EnvLookup`] implementation backed by `std::env`.
pub(crate) struct ProcessEnv;

impl EnvLookup for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn contains(&self, key: &str) -> bool {
        std::env::var_os(key).is_some()
    }

    fn is_from_dotenv(&self, key: &str) -> bool {
        std::env::var_os(key).is_some() && super::dotenv_loaded_variable(key)
    }
}

/// Process env lookup that preserves every override except
/// `OPENHUMAN_WORKSPACE`.
pub(crate) struct ProcessEnvWithoutWorkspace;

impl EnvLookup for ProcessEnvWithoutWorkspace {
    fn get(&self, key: &str) -> Option<String> {
        if key == "OPENHUMAN_WORKSPACE" {
            None
        } else {
            ProcessEnv.get(key)
        }
    }

    fn contains(&self, key: &str) -> bool {
        if key == "OPENHUMAN_WORKSPACE" {
            false
        } else {
            ProcessEnv.contains(key)
        }
    }

    fn is_from_dotenv(&self, key: &str) -> bool {
        key != "OPENHUMAN_WORKSPACE" && ProcessEnv.is_from_dotenv(key)
    }
}

/// Environment lookup for callers that must preserve the persisted config
/// exactly while still loading it through the revision-aware source path.
pub(crate) struct NoEnv;

impl EnvLookup for NoEnv {
    fn get(&self, _key: &str) -> Option<String> {
        None
    }

    fn contains(&self, _key: &str) -> bool {
        false
    }
}

/// Parse a boolean env-var value. Accepts the usual truthy/falsy tokens
/// (`1/true/yes/on` and `0/false/no/off`, case-insensitive). Returns `None`
/// on unrecognised values and logs a warning so silent mis-spellings don't
/// invisibly leave the config unchanged.
pub(super) fn parse_env_bool(name: &str, raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => {
            tracing::warn!(
                env = %name,
                value = %raw,
                "invalid boolean env override ignored; expected 1/true/yes/on or 0/false/no/off"
            );
            None
        }
    }
}
