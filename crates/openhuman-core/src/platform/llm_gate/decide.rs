//! Product configuration and policy projection over pure host resource admission.

use crate::config::schema::{PauseReason, Policy, SchedulerGateConfig, SchedulerGateMode};
pub use crate::platform::resource_admission::Signals;
use crate::platform::resource_admission::{self, Admission, Limits, Mode};

fn admission_limits(cfg: &SchedulerGateConfig) -> Limits {
    Limits {
        mode: match cfg.mode {
            SchedulerGateMode::Auto => Mode::Auto,
            SchedulerGateMode::AlwaysOn => Mode::AlwaysOn,
            SchedulerGateMode::Off => Mode::Off,
        },
        battery_floor: cfg.battery_floor,
        cpu_busy_threshold_pct: cfg.cpu_busy_threshold_pct,
        cpu_severe_pct: cfg.cpu_severe_pct,
        require_ac_power: cfg.require_ac_power,
    }
}

pub fn decide(signals: &Signals, cfg: &SchedulerGateConfig) -> Policy {
    match resource_admission::decide(signals, &admission_limits(cfg)) {
        Admission::Aggressive => Policy::Aggressive,
        Admission::Normal => Policy::Normal,
        Admission::Throttled => Policy::Throttled,
        Admission::Paused { reason } => {
            let reason = match reason {
                resource_admission::PauseReason::UserDisabled => PauseReason::UserDisabled,
                resource_admission::PauseReason::OnBattery => {
                    log::debug!(
                        "[scheduler_gate] policy decision: paused on_battery (require_ac_power=true, on_ac={})",
                        signals.on_ac_power
                    );
                    PauseReason::OnBattery
                }
                resource_admission::PauseReason::CpuPressure => {
                    log::debug!(
                        "[scheduler_gate] policy decision: paused cpu_pressure (cpu={:.1}% >= severe={:.1}%)",
                        signals.cpu_usage_pct,
                        cfg.cpu_severe_pct.clamp(0.0, 100.0)
                    );
                    PauseReason::CpuPressure
                }
            };
            Policy::Paused { reason }
        }
    }
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
