//! Which operating mode this process serves.
//!
//! | [`Mode`] | Who the process serves |
//! | --- | --- |
//! | [`Mode::SingleUser`] | One user on their own machine — the desktop app, the CLI, embedders. Today's behaviour and the default. |
//! | [`Mode::Saas`] | Many users through a trusted gateway, one embedded agent per user, with closed defaults. |
//!
//! The mode is a property of the **process**, fixed the first time a SaaS core
//! boots ([`lock_mode`]) and never part of any user's `Config`: a mode a
//! config write or an RPC could flip would let a caller reopen everything SaaS
//! closes. Code that must behave differently asks [`is_saas`], which reads an
//! unset slot as `SingleUser`, so a process that never boots SaaS takes no new
//! branch.

use std::str::FromStr;
use std::sync::OnceLock;

/// The operating mode of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// One user per process: today's desktop, CLI and embed behaviour.
    #[default]
    SingleUser,
    /// Many users per process, each served as their own agent.
    Saas,
}

impl Mode {
    /// Stable tag for logs and CLI flags.
    pub fn tag(self) -> &'static str {
        match self {
            Mode::SingleUser => "single-user",
            Mode::Saas => "saas",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tag())
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "single-user" | "single_user" | "singleuser" | "single" => Ok(Mode::SingleUser),
            "saas" => Ok(Mode::Saas),
            other => Err(format!(
                "unknown mode `{other}` (expected `single-user` or `saas`)"
            )),
        }
    }
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// The mode this process runs in; `SingleUser` until a SaaS core boots.
pub fn current_mode() -> Mode {
    MODE.get().copied().unwrap_or_default()
}

/// Whether this process serves many users.
pub fn is_saas() -> bool {
    current_mode() == Mode::Saas
}

/// Whether the command line or `OPENHUMAN_MODE` asks for SaaS, read before
/// the CLI parses its arguments. The CLI skips its early keyring set-up for a
/// SaaS boot, which roots the keyring under the operator directory instead
/// (`saas::build`).
pub fn requested_in(args: &[String], env: Option<&str>) -> bool {
    let saas = |raw: &str| raw.trim().parse::<Mode>() == Ok(Mode::Saas);
    env.is_some_and(saas)
        || args.windows(2).any(|w| w[0] == "--mode" && saas(&w[1]))
        || args
            .iter()
            .any(|a| a.strip_prefix("--mode=").is_some_and(saas))
}

static SAAS_BOOT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Claim the process's one SaaS boot. A second claim fails, even while the
/// first boot is still building.
pub(crate) fn reserve_saas_boot() -> Result<(), String> {
    if SAAS_BOOT.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err("a SaaS core has already booted (or is booting) in this process".to_string());
    }
    Ok(())
}

/// Whether a core of `host_kind` may boot now. A process locked to SaaS boots
/// nothing but its own SaaS core; a SaaS core boots only through
/// `saas::build`, which runs the boot guard and locks the mode first, and only
/// once per process (`core_running`: a default context already exists).
pub(crate) fn admit_core(
    host_kind: crate::core::types::HostKind,
    core_running: bool,
) -> Result<(), String> {
    let saas_host = host_kind == crate::core::types::HostKind::Saas;
    match (is_saas(), saas_host) {
        (true, false) => Err(format!(
            "this process serves SaaS; refusing a {host_kind:?} core"
        )),
        (false, true) => Err("a SaaS core boots only through saas::build".to_string()),
        (true, true) if core_running => {
            Err("this process already serves its SaaS core".to_string())
        }
        _ => Ok(()),
    }
}

/// Fix the process mode. Locking the mode it already has is a no-op; locking
/// a different one fails, because one process never serves both shapes.
pub(crate) fn lock_mode(mode: Mode) -> Result<(), String> {
    lock_in(&MODE, mode)
}

fn lock_in(slot: &OnceLock<Mode>, mode: Mode) -> Result<(), String> {
    let locked = *slot.get_or_init(|| mode);
    if locked == mode {
        log::info!("[mode] process mode locked to {mode}");
        Ok(())
    } else {
        Err(format!(
            "this process is already running in {locked} mode; it cannot switch to {mode}"
        ))
    }
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
