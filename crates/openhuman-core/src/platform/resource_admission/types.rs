//! Pure resource-admission inputs and results, independent of product config.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    AlwaysOn,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub mode: Mode,
    pub battery_floor: f32,
    pub cpu_busy_threshold_pct: f32,
    pub cpu_severe_pct: f32,
    pub require_ac_power: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    UserDisabled,
    OnBattery,
    CpuPressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Aggressive,
    Normal,
    Throttled,
    Paused { reason: PauseReason },
}

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
