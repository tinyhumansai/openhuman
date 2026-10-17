//! Which turns get remixed: the `OPENHUMAN_TINYMEMES` flag and its A/B split.
//!
//! | value | behaviour |
//! | --- | --- |
//! | unset, `off`, `0`, `false` | disabled; no tinymemes work at all |
//! | `on`, `1`, `true` | every thread is in the treatment arm |
//! | `ab` | 50% of threads are treatment, the rest control |
//! | `ab:NN` | NN% of threads are treatment |
//!
//! Assignment is by a stable hash of the thread id, so a thread keeps its arm
//! for its whole life and across restarts. The flag is read on every turn, so
//! it can change without a restart.
//!
//! A build can ship with a different default: set `OPENHUMAN_TINYMEMES_DEFAULT`
//! (same values) in the environment of the `cargo build` that produces the
//! release, and it is baked in. The runtime `OPENHUMAN_TINYMEMES` still wins
//! whenever it is set, so a deployment can always be switched back off.

pub(crate) const FLAG_ENV: &str = "OPENHUMAN_TINYMEMES";

/// Default baked in at build time (`OPENHUMAN_TINYMEMES_DEFAULT` during
/// `cargo build`); `None` means off.
const BUILD_DEFAULT: Option<&str> = option_env!("OPENHUMAN_TINYMEMES_DEFAULT");

/// Parsed flag value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    On,
    Ab { percent: u8 },
}

/// The arm a thread is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arm {
    /// Feature off: nothing runs and nothing is logged per turn.
    Disabled,
    /// In an experiment, not remixed. Logged for the throughput comparison.
    Control,
    /// Remixed.
    Treatment,
}

impl Arm {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Arm::Disabled => "disabled",
            Arm::Control => "control",
            Arm::Treatment => "treatment",
        }
    }
}

pub(crate) fn mode_from(value: Option<&str>) -> Mode {
    let Some(raw) = value.map(|v| v.trim().to_ascii_lowercase()) else {
        return Mode::Off;
    };
    match raw.as_str() {
        "" | "off" | "0" | "false" | "no" => Mode::Off,
        "on" | "1" | "true" | "yes" => Mode::On,
        "ab" => Mode::Ab { percent: 50 },
        // Any all-digit percentage above 100 means 100, including one too
        // large for `u64`.
        other => match other.strip_prefix("ab:").map(str::trim) {
            Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
                Mode::Ab {
                    percent: digits.parse::<u64>().map_or(100, |n| n.min(100)) as u8,
                }
            }
            _ => {
                log::warn!("[tinymemes] unrecognised {FLAG_ENV} value; treating as off");
                Mode::Off
            }
        },
    }
}

pub(crate) fn mode() -> Mode {
    resolve_mode(std::env::var(FLAG_ENV).ok().as_deref(), BUILD_DEFAULT)
}

/// The runtime value when set, else the build-time default, else off.
pub(crate) fn resolve_mode(runtime: Option<&str>, build_default: Option<&str>) -> Mode {
    match runtime {
        Some(value) => mode_from(Some(value)),
        None => mode_from(build_default),
    }
}

/// Stable 0..100 bucket for a thread (FNV-1a, so it does not change between
/// builds or platforms the way `DefaultHasher` may).
pub(crate) fn bucket(thread_id: &str) -> u8 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in thread_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % 100) as u8
}

/// The arm for a thread under `config`: the `[components] memes` switch is the
/// master, and the flag's arm is honoured only while the component is on.
pub(crate) fn arm_for(config: &crate::config::Config, mode: Mode, thread_id: &str) -> Arm {
    if !config.components.memes {
        return Arm::Disabled;
    }
    assign(mode, thread_id)
}

pub(crate) fn assign(mode: Mode, thread_id: &str) -> Arm {
    match mode {
        Mode::Off => Arm::Disabled,
        Mode::On => Arm::Treatment,
        Mode::Ab { percent } => {
            if bucket(thread_id) < percent {
                Arm::Treatment
            } else {
                Arm::Control
            }
        }
    }
}

#[cfg(test)]
#[path = "arm_tests.rs"]
mod tests;
