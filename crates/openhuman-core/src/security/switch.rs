//! The one place that decides whether TinySecurity-delegated checks run.
//!
//! OpenHuman delegates its security checks to the TinySecurity module: the
//! prompt screen, action classification, approvals, path checks, the action
//! budget, egress privacy checks, redaction and the audit trail. When the
//! module is **enabled** those checks fail closed: a module that is not loaded,
//! errors or times out refuses. When security is **disabled** they are
//! permissive instead: every check allows and redaction passes content through.
//!
//! Security is disabled for one of two reasons ([`DisabledReason`]):
//!
//! - the build has no `security-module` feature, so there is no module to ask
//!   ([`DisabledReason::FeatureAbsent`]);
//! - the booted configuration says `[security] mode = "off"`
//!   ([`DisabledReason::ConfiguredOff`]).
//!
//! Many sites (redaction, URL classification, the policy's path checks) have no
//! configuration in hand, so the booted mode is recorded process-wide when the
//! core boots ([`apply_boot_config`]). OpenHuman runs one `Runtime` per process,
//! so one process-wide mode is sound. A site that does hold a configuration asks
//! [`disabled_for`], which also honours that configuration's own `[security]`.
//!
//! Some things stay on when security is disabled, because they are not module
//! checks: `[privacy] mode = "local_only"` still refuses external egress and
//! shell commands that may reach the network; the
//! approval gate is unchanged, so a call a channel, flow or origin requires a
//! person to approve still waits for one (and is refused where no approval
//! engine exists), although the module-driven approval prompts are skipped
//! upstream; and the encrypted secret store, keychain and device pairing still
//! need the module's cryptography.

use std::sync::atomic::{AtomicBool, Ordering};

/// Why TinySecurity-delegated checks are not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledReason {
    /// The host was built without the `security-module` cargo feature.
    FeatureAbsent,
    /// The configuration says `[security] mode = "off"`.
    ConfiguredOff,
}

impl DisabledReason {
    /// Why, in words for a log line.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::FeatureAbsent => "built without the security-module feature",
            Self::ConfiguredOff => "[security] mode = \"off\"",
        }
    }
}

/// The booted configuration's `[security] mode = "off"`, process-wide. Written by
/// [`apply_boot_config`]; read by [`disabled`].
#[cfg(not(test))]
static CONFIGURED_OFF: AtomicBool = AtomicBool::new(false);

// Tests keep the booted mode per thread, so one test turning security off can
// never reach a test running on another thread.
#[cfg(test)]
thread_local! {
    static TEST_CONFIGURED_OFF: AtomicBool = const { AtomicBool::new(false) };
}

fn configured_off() -> bool {
    #[cfg(not(test))]
    {
        CONFIGURED_OFF.load(Ordering::Relaxed)
    }
    #[cfg(test)]
    {
        TEST_CONFIGURED_OFF.with(|flag| flag.load(Ordering::Relaxed))
    }
}

fn set_configured_off(off: bool) {
    #[cfg(not(test))]
    CONFIGURED_OFF.store(off, Ordering::Relaxed);
    #[cfg(test)]
    TEST_CONFIGURED_OFF.with(|flag| flag.store(off, Ordering::Relaxed));
}

/// Whether TinySecurity-delegated checks are disabled for this process, and why.
/// `None` means they run and fail closed.
#[must_use]
pub fn disabled() -> Option<DisabledReason> {
    if cfg!(not(feature = "security-module")) {
        Some(DisabledReason::FeatureAbsent)
    } else if configured_off() {
        Some(DisabledReason::ConfiguredOff)
    } else {
        None
    }
}

/// [`disabled`], also honouring `config`'s own `[security]` table. Use this
/// where the caller holds the configuration it acts under.
#[must_use]
pub fn disabled_for(config: Option<&crate::config::Config>) -> Option<DisabledReason> {
    disabled().or_else(|| {
        config
            .is_some_and(|config| !config.security.enforces())
            .then_some(DisabledReason::ConfiguredOff)
    })
}

/// Whether `config` alone, in this build, has security disabled: the feature
/// is absent or the config says `off`. Unlike [`disabled_for`] it ignores the
/// process's booted mode, for describing a configuration that has not booted.
#[must_use]
pub fn disabled_by_config(config: &crate::config::Config) -> Option<DisabledReason> {
    if cfg!(not(feature = "security-module")) {
        Some(DisabledReason::FeatureAbsent)
    } else if !config.security.enforces() {
        Some(DisabledReason::ConfiguredOff)
    } else {
        None
    }
}

/// Record the booted configuration's `[security]` mode for the process and say
/// once whether checks are off. Called when the core boots with a resolved
/// configuration (never for an inspection boot), and by an embedding host whose
/// builder sets `[security]` over a configuration the core discovered itself.
/// Logs one line when something is off: security as a whole, or only the
/// prompt screen.
pub fn apply_boot_config(config: &crate::config::Config) {
    set_configured_off(!config.security.enforces());
    if let Some(reason) = disabled() {
        tracing::info!(
            "[security] TinySecurity checks disabled ({}): tool calls, paths, budget and egress are allowed without the module, and tool output, diagnostics and log URLs are not redacted",
            reason.describe()
        );
    } else if !config.prompt_screen.enforces() {
        tracing::info!(
            "[prompt_injection] prompt screen disabled by configuration ([prompt_screen] mode = \"off\"); input is not checked by TinySecurity"
        );
    }
}

/// Turn security off for the current test thread until the guard drops.
#[cfg(test)]
pub(crate) fn off_for_this_thread() -> ThreadOffGuard {
    let previous = configured_off();
    set_configured_off(true);
    ThreadOffGuard { previous }
}

/// Restores the thread's booted mode on drop.
#[cfg(test)]
pub(crate) struct ThreadOffGuard {
    previous: bool,
}

#[cfg(test)]
impl Drop for ThreadOffGuard {
    fn drop(&mut self) {
        set_configured_off(self.previous);
    }
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
