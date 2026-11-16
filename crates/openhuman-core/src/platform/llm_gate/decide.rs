//! The gate's decision: host signals + `[scheduler_gate]` config → [`Policy`].
//!
//! Pure: no sampling, I/O, waiting or global state. [`super::signals`] samples
//! the host, [`super::throttle`] caches the result and runs the cooperative
//! wait.

use crate::config::schema::{SchedulerGateConfig, SchedulerGateMode};

/// One snapshot of the host signals the gate decides on.
#[derive(Debug, Clone, Copy)]
pub struct Signals {
    pub on_ac_power: bool,
    /// 0.0..=1.0, or `None` when no battery sensor is present (most servers).
    pub battery_charge: Option<f32>,
    /// Recent global CPU usage, 0..100.
    pub cpu_usage_pct: f32,
    pub server_mode: bool,
}

/// Why the gate is currently paused. Carried by [`Policy::Paused`] so
/// downstream consumers (UI, logging, observability) can surface a
/// specific user-facing reason instead of a generic "paused" label.
///
/// `UserDisabled` covers the `SchedulerGateMode::Off` path, `OnBattery` and
/// `CpuPressure` are the power-aware reasons (#1073), and `Unknown` is the
/// safe fallback for callers that don't have specific context yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// User explicitly turned the gate off in config.
    UserDisabled,
    /// Host on battery and gate's power-aware mode kicked in (#1073).
    OnBattery,
    /// CPU pressure exceeded the gate threshold (#1073).
    CpuPressure,
    /// No active app session — background AI work is suspended until the
    /// user signs in again. Trumps every other signal: while signed out
    /// the host should do *no* LLM-bound work, period. Set by
    /// `gate::set_signed_out(true)` from the credentials lifecycle and
    /// from 401-detection sites.
    SignedOut,
    /// Pause reason not yet classified — placeholder while #1073 is in flight.
    Unknown,
}

impl PauseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserDisabled => "user_disabled",
            Self::OnBattery => "on_battery",
            Self::CpuPressure => "cpu_pressure",
            Self::SignedOut => "signed_out",
            Self::Unknown => "unknown",
        }
    }
}

/// Background-AI scheduling tier. See module docs in `mod.rs` for semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Aggressive,
    Normal,
    Throttled,
    /// Gate paused. The `reason` is rendered to users in the memory-sync
    /// status UI (#1136) and recorded in observability.
    Paused {
        reason: PauseReason,
    },
}

impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aggressive => "aggressive",
            Self::Normal => "normal",
            Self::Throttled => "throttled",
            Self::Paused { .. } => "paused",
        }
    }

    /// `Some(reason)` when paused, `None` otherwise. Convenience for
    /// callers that only need the reason and don't want to pattern-match
    /// the whole enum (UI badges, log line construction).
    pub fn pause_reason(self) -> Option<PauseReason> {
        match self {
            Self::Paused { reason } => Some(reason),
            _ => None,
        }
    }
}

/// Compute the current [`Policy`] from sampled signals + user config.
///
/// Order of evaluation matters — explicit user overrides win first, then
/// deployment mode, then dynamic host signals. The signed-out override is
/// applied above this, in `gate`.
pub fn decide(signals: &Signals, cfg: &SchedulerGateConfig) -> Policy {
    match cfg.mode {
        SchedulerGateMode::Off => {
            return Policy::Paused {
                reason: PauseReason::UserDisabled,
            }
        }
        SchedulerGateMode::AlwaysOn => return Policy::Aggressive,
        SchedulerGateMode::Auto => {}
    }

    if signals.server_mode {
        return Policy::Aggressive;
    }

    // Clamp config-supplied thresholds so a malformed config.toml (e.g.
    // `battery_floor = 1.5` or a negative cpu threshold) can't silently
    // disable / force throttling — the field is `f32` and serde won't
    // reject out-of-domain values for us.
    let battery_floor = cfg.battery_floor.clamp(0.0, 1.0);
    let cpu_threshold = cfg.cpu_busy_threshold_pct.clamp(0.0, 100.0);
    let cpu_severe = cfg.cpu_severe_pct.clamp(0.0, 100.0);

    // ── Pause checks come BEFORE the throttle gate — these are the
    //    "stand down completely" signals. Hierarchy:
    //      1. user policy (`require_ac_power` on battery)
    //      2. host on fire (CPU severely pegged)

    // (1) Power-aware stand-down. Only consult `on_ac_power` when the
    //     user explicitly opts in — many desktops report `false` here
    //     because they have no battery + no AC sensor, and we don't
    //     want to silently disable background work for them.
    if cfg.require_ac_power && !signals.on_ac_power {
        log::debug!(
            "[scheduler_gate] policy decision: paused on_battery (require_ac_power=true, on_ac={})",
            signals.on_ac_power
        );
        return Policy::Paused {
            reason: PauseReason::OnBattery,
        };
    }

    // (2) Hard CPU ceiling — at >= cpu_severe_pct the host is unusable;
    //     a Throttled 30s backoff is not enough, hold every job.
    if signals.cpu_usage_pct >= cpu_severe {
        log::debug!(
            "[scheduler_gate] policy decision: paused cpu_pressure (cpu={:.1}% >= severe={:.1}%)",
            signals.cpu_usage_pct,
            cpu_severe
        );
        return Policy::Paused {
            reason: PauseReason::CpuPressure,
        };
    }

    let battery_ok = signals.on_ac_power
        || signals
            .battery_charge
            .map(|c| c >= battery_floor)
            .unwrap_or(true); // no battery present == treat as plugged in

    let cpu_ok = signals.cpu_usage_pct <= cpu_threshold;

    if battery_ok && cpu_ok {
        Policy::Normal
    } else {
        Policy::Throttled
    }
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
