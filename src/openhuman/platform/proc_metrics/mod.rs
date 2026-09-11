//! Cross-platform process memory sampling.
//!
//! Reads the current process's resident-memory breakdown from
//! `/proc/self/smaps_rollup` + `/proc/self/status` and aggregates repeated
//! samples into a [`RosterResult`] / [`BenchReport`]. Written for the
//! `rss-bench` benchmark harness (#5046), which measures the steady-state RSS
//! of an embedded `openhuman_core` agent roster against the 20–30 MiB budget,
//! but [`sample_self`] is a general capability: any caller wanting this
//! process's RSS / peak-RSS figures can use it. Linux additionally reports PSS
//! and private-page breakdowns; macOS leaves those Linux-only fields at zero.
//!
//! The parsers ([`parse_status`], [`parse_smaps_rollup`]) are OS-agnostic and
//! take `&str`, so they are unit-tested without a live `/proc`. [`sample_self`]
//! supports Linux and macOS and returns a structured error elsewhere — it never
//! fabricates a reading.

use serde::{Deserialize, Serialize};

pub mod tree;
pub use tree::{sample_tree, ChildSample, TreeSample};

/// Product budget for the embedded roster, in KiB (#5046). Target the agent
/// roster should land under.
pub const RSS_BUDGET_KIB: u64 = 20 * 1024;
/// Hard cap for the embedded roster, in KiB (#5046). Steady-state RSS above
/// this fails the (eventually blocking) CI gate.
pub const RSS_HARD_CAP_KIB: u64 = 30 * 1024;

/// One resident-memory sample of a single process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcSample {
    /// Resident set size (`/proc/self/status` `VmRSS`).
    pub rss_kib: u64,
    /// Proportional set size (`/proc/self/smaps_rollup` `Pss`).
    pub pss_kib: u64,
    /// Private clean pages (`smaps_rollup` `Private_Clean`).
    pub private_clean_kib: u64,
    /// Private dirty pages (`smaps_rollup` `Private_Dirty`).
    pub private_dirty_kib: u64,
    /// Peak resident set size (`status` `VmHWM`).
    pub vm_hwm_kib: u64,
    /// Live thread count (`status` `Threads`).
    pub threads: u64,
    /// On-disk size of the running executable, in bytes.
    pub binary_size_bytes: u64,
    /// Cumulative user-mode CPU time, in milliseconds. Linux: `/proc/self/stat`
    /// `utime` (ticks → ms). macOS: `proc_pid_rusage` `ri_user_time` (ns → ms).
    /// Defaulted so pre-existing JSON consumers keep deserializing.
    #[serde(default)]
    pub cpu_user_ms: u64,
    /// Cumulative system-mode CPU time, in milliseconds. Linux: `/proc/self/stat`
    /// `stime`. macOS: `proc_pid_rusage` `ri_system_time`.
    #[serde(default)]
    pub cpu_system_ms: u64,
    /// Open file-descriptor count. Linux: entries in `/proc/self/fd`. macOS:
    /// `proc_pidinfo(PROC_PIDLISTFDS)` buffer size / `proc_fdinfo` size. `None`
    /// when the platform lookup is unavailable rather than a misleading zero.
    #[serde(default)]
    pub open_fds: Option<u64>,
}

/// Fields extracted from `/proc/<pid>/status`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusFields {
    pub vm_rss_kib: u64,
    pub vm_hwm_kib: u64,
    pub threads: u64,
}

/// Fields extracted from `/proc/<pid>/smaps_rollup`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SmapsRollupFields {
    pub pss_kib: u64,
    pub private_clean_kib: u64,
    pub private_dirty_kib: u64,
}

/// First whitespace-separated integer in a `/proc` value tail
/// (e.g. `"\t  1234 kB"` → `1234`). Zero when absent or unparsable.
fn first_u64(rest: &str) -> u64 {
    rest.split_whitespace()
        .next()
        .and_then(|token| token.parse().ok())
        .unwrap_or(0)
}

/// Parse the `VmRSS` / `VmHWM` / `Threads` lines out of `/proc/<pid>/status`.
/// Missing keys stay zero. OS-agnostic — feed it the file contents.
pub fn parse_status(contents: &str) -> StatusFields {
    let mut fields = StatusFields::default();
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            fields.vm_rss_kib = first_u64(rest);
        } else if let Some(rest) = line.strip_prefix("VmHWM:") {
            fields.vm_hwm_kib = first_u64(rest);
        } else if let Some(rest) = line.strip_prefix("Threads:") {
            fields.threads = first_u64(rest);
        }
    }
    fields
}

/// Parse the `Pss` / `Private_Clean` / `Private_Dirty` lines out of
/// `/proc/<pid>/smaps_rollup` (the pre-summed variant of `smaps`). Missing keys
/// stay zero. `strip_prefix` with the trailing colon avoids matching the
/// `Pss_Anon:` / `Pss_Dirty:` breakdown lines.
pub fn parse_smaps_rollup(contents: &str) -> SmapsRollupFields {
    let mut fields = SmapsRollupFields::default();
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("Pss:") {
            fields.pss_kib = first_u64(rest);
        } else if let Some(rest) = line.strip_prefix("Private_Clean:") {
            fields.private_clean_kib = first_u64(rest);
        } else if let Some(rest) = line.strip_prefix("Private_Dirty:") {
            fields.private_dirty_kib = first_u64(rest);
        }
    }
    fields
}

/// User/system CPU jiffies parsed from `/proc/<pid>/stat`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatCpuFields {
    pub utime_ticks: u64,
    pub stime_ticks: u64,
}

/// Parse `utime` (field 14) and `stime` (field 15) out of `/proc/<pid>/stat`.
/// The `comm` field (2) is parenthesised and may itself contain spaces or
/// parens, so we split on the **last** `')'` and index the remaining
/// whitespace-separated fields: after `)`, token 0 is `state`, so `utime` is
/// token 11 and `stime` token 12. Missing/short input yields zero.
pub fn parse_proc_stat_cpu(contents: &str) -> StatCpuFields {
    let Some(after) = contents.rsplit_once(')').map(|(_, tail)| tail) else {
        return StatCpuFields::default();
    };
    let fields: Vec<&str> = after.split_whitespace().collect();
    // token 0 == state; utime == token 11, stime == token 12.
    let utime_ticks = fields.get(11).and_then(|t| t.parse().ok()).unwrap_or(0);
    let stime_ticks = fields.get(12).and_then(|t| t.parse().ok()).unwrap_or(0);
    StatCpuFields {
        utime_ticks,
        stime_ticks,
    }
}

/// Convert CPU clock ticks to milliseconds given `CLK_TCK` (ticks per second).
/// A zero or negative tick rate yields zero rather than dividing by zero.
pub fn cpu_ticks_to_ms(ticks: u64, clk_tck: i64) -> u64 {
    if clk_tck <= 0 {
        return 0;
    }
    ticks.saturating_mul(1000) / clk_tck as u64
}

/// Count entries in `/proc/<pid>/fd` (excluding `.`/`..`, which `read_dir`
/// already omits). `None` when the directory can't be read.
#[cfg(target_os = "linux")]
fn count_open_fds() -> Option<u64> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    Some(entries.filter(|e| e.is_ok()).count() as u64)
}

/// Sample this process's resident memory. Linux-only.
#[cfg(target_os = "linux")]
pub fn sample_self() -> anyhow::Result<ProcSample> {
    use anyhow::Context;
    let status = std::fs::read_to_string("/proc/self/status").context("read /proc/self/status")?;
    let smaps = std::fs::read_to_string("/proc/self/smaps_rollup")
        .context("read /proc/self/smaps_rollup")?;
    let status = parse_status(&status);
    let smaps = parse_smaps_rollup(&smaps);
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let cpu = parse_proc_stat_cpu(&stat);
    // SAFETY: `sysconf` is a pure lookup; `_SC_CLK_TCK` is a valid selector.
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let binary_size_bytes = std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|meta| meta.len())
        .unwrap_or(0);
    Ok(ProcSample {
        rss_kib: status.vm_rss_kib,
        pss_kib: smaps.pss_kib,
        private_clean_kib: smaps.private_clean_kib,
        private_dirty_kib: smaps.private_dirty_kib,
        vm_hwm_kib: status.vm_hwm_kib,
        threads: status.threads,
        binary_size_bytes,
        cpu_user_ms: cpu_ticks_to_ms(cpu.utime_ticks, clk_tck),
        cpu_system_ms: cpu_ticks_to_ms(cpu.stime_ticks, clk_tck),
        open_fds: count_open_fds(),
    })
}

/// Sample this process's resident memory on macOS via `proc_pid_rusage`.
///
/// `ri_resident_size` and `ru_maxrss` are bytes on Darwin. PSS and private-page
/// accounting have no direct macOS equivalent, so those Linux-specific fields
/// remain zero rather than being populated with a misleading substitute.
#[cfg(target_os = "macos")]
pub fn sample_self() -> anyhow::Result<ProcSample> {
    use std::mem::{size_of, MaybeUninit};

    let pid = std::process::id() as libc::c_int;
    let mut usage = MaybeUninit::<libc::rusage_info_v2>::uninit();
    // SAFETY: `usage` points to writable storage of the exact structure size
    // required by `RUSAGE_INFO_V2`; the kernel initializes it on success.
    let result = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if result != 0 {
        return Err(Into::into(std::io::Error::last_os_error()));
    }
    // SAFETY: `proc_pid_rusage` returned success and initialized `usage`.
    let usage = unsafe { usage.assume_init() };

    let mut peak = MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `peak` is valid writable storage for `getrusage`.
    let peak_result = unsafe { libc::getrusage(libc::RUSAGE_SELF, peak.as_mut_ptr()) };
    let vm_hwm_kib = if peak_result == 0 {
        // SAFETY: `getrusage` returned success and initialized `peak`.
        (unsafe { peak.assume_init() }.ru_maxrss as u64) / 1024
    } else {
        0
    };

    let mut task = MaybeUninit::<libc::proc_taskinfo>::uninit();
    // SAFETY: `task` points to writable storage and its size is passed to
    // `proc_pidinfo`, which initializes it when the returned byte count matches.
    let task_bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            task.as_mut_ptr().cast(),
            size_of::<libc::proc_taskinfo>() as libc::c_int,
        )
    };
    let threads = if task_bytes == size_of::<libc::proc_taskinfo>() as libc::c_int {
        // SAFETY: the kernel returned the full structure size.
        unsafe { task.assume_init() }.pti_threadnum.max(0) as u64
    } else {
        0
    };

    // `proc_pidinfo(PROC_PIDLISTFDS, .., NULL, 0)` returns the byte size of the
    // fd table; dividing by `proc_fdinfo` size gives the descriptor count.
    // SAFETY: passing a null buffer with zero length is the documented
    // size-probe form of `proc_pidinfo`.
    let fd_bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let open_fds = if fd_bytes > 0 {
        Some(fd_bytes as u64 / size_of::<libc::proc_fdinfo>() as u64)
    } else {
        None
    };

    let binary_size_bytes = std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|meta| meta.len())
        .unwrap_or(0);
    Ok(ProcSample {
        rss_kib: usage.ri_resident_size / 1024,
        pss_kib: 0,
        private_clean_kib: 0,
        private_dirty_kib: 0,
        vm_hwm_kib,
        threads,
        binary_size_bytes,
        // `ri_user_time` / `ri_system_time` are nanoseconds on Darwin.
        cpu_user_ms: usage.ri_user_time / 1_000_000,
        cpu_system_ms: usage.ri_system_time / 1_000_000,
        open_fds,
    })
}

/// Unsupported-platform stub — fails loudly rather than fabricating a reading.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn sample_self() -> anyhow::Result<ProcSample> {
    anyhow::bail!(
        "proc_metrics::sample_self supports Linux and macOS (this is a {} build)",
        std::env::consts::OS
    )
}

/// Median of a slice of `u64`, averaging the two middle values for even counts.
/// Empty input yields zero.
fn median_u64(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        // Average without overflow.
        sorted[mid - 1] + (sorted[mid] - sorted[mid - 1]) / 2
    }
}

/// Aggregated result for one roster size across several fresh-process samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterResult {
    pub roster_size: usize,
    pub sample_count: usize,
    pub median_rss_kib: u64,
    pub min_rss_kib: u64,
    pub max_rss_kib: u64,
    pub mean_rss_kib: u64,
    pub median_pss_kib: u64,
    pub max_vm_hwm_kib: u64,
    pub median_threads: u64,
    pub binary_size_bytes: u64,
    /// The raw per-process samples, retained as a CI artifact.
    pub samples: Vec<ProcSample>,
}

impl RosterResult {
    /// Aggregate raw samples into the reported statistics. RSS is summarised as
    /// median (steady-state), min/max/mean for spread; PSS/threads as median;
    /// `VmHWM` as the max (peak) across processes.
    pub fn from_samples(roster_size: usize, samples: Vec<ProcSample>) -> Self {
        let rss: Vec<u64> = samples.iter().map(|s| s.rss_kib).collect();
        let pss: Vec<u64> = samples.iter().map(|s| s.pss_kib).collect();
        let threads: Vec<u64> = samples.iter().map(|s| s.threads).collect();
        let count = samples.len();
        let mean_rss_kib = if count == 0 {
            0
        } else {
            rss.iter().sum::<u64>() / count as u64
        };
        Self {
            roster_size,
            sample_count: count,
            median_rss_kib: median_u64(&rss),
            min_rss_kib: rss.iter().copied().min().unwrap_or(0),
            max_rss_kib: rss.iter().copied().max().unwrap_or(0),
            mean_rss_kib,
            median_pss_kib: median_u64(&pss),
            max_vm_hwm_kib: samples.iter().map(|s| s.vm_hwm_kib).max().unwrap_or(0),
            median_threads: median_u64(&threads),
            binary_size_bytes: samples.first().map(|s| s.binary_size_bytes).unwrap_or(0),
            samples,
        }
    }
}

/// The full benchmark report, serialized to the raw JSON CI artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    pub schema_version: u32,
    pub git_sha: String,
    pub kernel: String,
    pub rss_budget_kib: u64,
    pub rss_hard_cap_kib: u64,
    pub rosters: Vec<RosterResult>,
}

/// Current schema version for [`BenchReport`]; bump on any field change so
/// downstream trend tooling can detect format shifts.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

impl BenchReport {
    /// Marginal steady-state RSS cost of each additional agent (#5046), derived
    /// from the smallest and largest rosters measured:
    /// `(median_rss(max) - median_rss(min)) / (max_size - min_size)`.
    ///
    /// Returns `(min_roster_size, max_roster_size, kib_per_agent)`, or `None` when
    /// fewer than two distinct roster sizes were measured (no incremental cost is
    /// derivable). For the default `{1, 8}` rosters this is the per-agent cost of
    /// agents 2–8.
    pub fn per_agent_increment_kib(&self) -> Option<(usize, usize, u64)> {
        let min = self.rosters.iter().min_by_key(|r| r.roster_size)?;
        let max = self.rosters.iter().max_by_key(|r| r.roster_size)?;
        let span = max.roster_size.checked_sub(min.roster_size)?;
        if span == 0 {
            return None;
        }
        let per_agent = max.median_rss_kib.saturating_sub(min.median_rss_kib) / span as u64;
        Some((min.roster_size, max.roster_size, per_agent))
    }
}

fn kib_to_mib(kib: u64) -> f64 {
    kib as f64 / 1024.0
}

/// Human-readable Markdown summary for stdout + `$GITHUB_STEP_SUMMARY`.
pub fn human_summary(report: &BenchReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "### Embedded `openhuman_core` RSS benchmark (#5046)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "kernel `{}` · git `{}` · target ≤ {:.0} MiB · hard cap ≤ {:.0} MiB",
        report.kernel,
        report.git_sha,
        kib_to_mib(report.rss_budget_kib),
        kib_to_mib(report.rss_hard_cap_kib),
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| roster | n | median RSS | min–max RSS | median PSS | peak VmHWM | threads | binary |"
    );
    let _ = writeln!(
        out,
        "| ------ | - | ---------- | ----------- | ---------- | ---------- | ------- | ------ |"
    );
    for r in &report.rosters {
        let over = if r.median_rss_kib > report.rss_hard_cap_kib {
            " ⚠️"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "| {} agent{} | {} | {:.1} MiB{} | {:.1}–{:.1} MiB | {:.1} MiB | {:.1} MiB | {} | {:.1} MiB |",
            r.roster_size,
            if r.roster_size == 1 { "" } else { "s" },
            r.sample_count,
            kib_to_mib(r.median_rss_kib),
            over,
            kib_to_mib(r.min_rss_kib),
            kib_to_mib(r.max_rss_kib),
            kib_to_mib(r.median_pss_kib),
            kib_to_mib(r.max_vm_hwm_kib),
            r.median_threads,
            r.binary_size_bytes as f64 / (1024.0 * 1024.0),
        );
    }
    if let Some((min_size, max_size, per_agent_kib)) = report.per_agent_increment_kib() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Per-agent increment (roster {min_size}→{max_size}): {:.2} MiB/agent",
            kib_to_mib(per_agent_kib),
        );
    }
    out
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
