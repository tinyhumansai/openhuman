//! Process memory sampling from Linux `/proc`.
//!
//! Reads the current process's resident-memory breakdown from
//! `/proc/self/smaps_rollup` + `/proc/self/status` and aggregates repeated
//! samples into a [`RosterResult`] / [`BenchReport`]. Written for the
//! `rss-bench` benchmark harness (#5046), which measures the steady-state RSS
//! of an embedded `openhuman_core` agent roster against the 20–30 MiB budget,
//! but [`sample_self`] is a general capability: any caller wanting this
//! process's RSS / PSS / private-page / peak-RSS figures on Linux can use it.
//!
//! The parsers ([`parse_status`], [`parse_smaps_rollup`]) are OS-agnostic and
//! take `&str`, so they are unit-tested without a live `/proc`. [`sample_self`]
//! is Linux-only and returns a structured error elsewhere — it never fabricates
//! a reading (a macOS local run fails loudly rather than emitting garbage).

use serde::{Deserialize, Serialize};

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

/// Sample this process's resident memory. Linux-only.
#[cfg(target_os = "linux")]
pub fn sample_self() -> anyhow::Result<ProcSample> {
    use anyhow::Context;
    let status = std::fs::read_to_string("/proc/self/status").context("read /proc/self/status")?;
    let smaps = std::fs::read_to_string("/proc/self/smaps_rollup")
        .context("read /proc/self/smaps_rollup")?;
    let status = parse_status(&status);
    let smaps = parse_smaps_rollup(&smaps);
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
    })
}

/// Sample this process's resident memory. Non-Linux stub — fails loudly rather
/// than fabricating a reading.
#[cfg(not(target_os = "linux"))]
pub fn sample_self() -> anyhow::Result<ProcSample> {
    anyhow::bail!(
        "proc_metrics::sample_self requires Linux /proc/self/smaps_rollup + status (this is a {} build)",
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
mod tests {
    use super::*;

    const SAMPLE_STATUS: &str = "Name:\trss-bench\nVmPeak:\t  123456 kB\nVmRSS:\t   20480 kB\nVmHWM:\t   24576 kB\nThreads:\t8\n";

    const SAMPLE_SMAPS_ROLLUP: &str = "00400000-7fff00000000 ---p 00000000 00:00 0 [rollup]\nRss:\t   20480 kB\nPss:\t   18000 kB\nPss_Anon:\t   1000 kB\nPss_Dirty:\t   500 kB\nPrivate_Clean:\t   4096 kB\nPrivate_Dirty:\t   12000 kB\n";

    #[test]
    fn parse_status_extracts_rss_hwm_threads() {
        let f = parse_status(SAMPLE_STATUS);
        assert_eq!(f.vm_rss_kib, 20480);
        assert_eq!(f.vm_hwm_kib, 24576);
        assert_eq!(f.threads, 8);
    }

    #[test]
    fn parse_status_missing_keys_stay_zero() {
        let f = parse_status("Name:\tx\nState:\tR\n");
        assert_eq!(f, StatusFields::default());
    }

    #[test]
    fn parse_smaps_rollup_extracts_pss_and_private_pages() {
        let f = parse_smaps_rollup(SAMPLE_SMAPS_ROLLUP);
        assert_eq!(f.pss_kib, 18000);
        assert_eq!(f.private_clean_kib, 4096);
        assert_eq!(f.private_dirty_kib, 12000);
    }

    #[test]
    fn parse_smaps_rollup_does_not_match_pss_breakdown_lines() {
        // `Pss_Anon:` / `Pss_Dirty:` must not be read as `Pss:`.
        let f = parse_smaps_rollup("Pss_Anon:\t   9999 kB\nPss_Dirty:\t   8888 kB\n");
        assert_eq!(f.pss_kib, 0);
    }

    fn sample(rss: u64, pss: u64, hwm: u64, threads: u64) -> ProcSample {
        ProcSample {
            rss_kib: rss,
            pss_kib: pss,
            private_clean_kib: 0,
            private_dirty_kib: 0,
            vm_hwm_kib: hwm,
            threads,
            binary_size_bytes: 1024,
        }
    }

    #[test]
    fn median_handles_odd_and_even() {
        assert_eq!(median_u64(&[]), 0);
        assert_eq!(median_u64(&[5]), 5);
        assert_eq!(median_u64(&[3, 1, 2]), 2);
        assert_eq!(median_u64(&[1, 2, 3, 4]), 2); // (2+3)/2 floored -> 2
    }

    #[test]
    fn from_samples_aggregates_rss_pss_and_peak() {
        let samples = vec![
            sample(20000, 18000, 21000, 8),
            sample(22000, 19000, 26000, 8),
            sample(21000, 18500, 24000, 8),
        ];
        let r = RosterResult::from_samples(8, samples);
        assert_eq!(r.roster_size, 8);
        assert_eq!(r.sample_count, 3);
        assert_eq!(r.median_rss_kib, 21000);
        assert_eq!(r.min_rss_kib, 20000);
        assert_eq!(r.max_rss_kib, 22000);
        assert_eq!(r.mean_rss_kib, 21000);
        assert_eq!(r.max_vm_hwm_kib, 26000); // peak across processes
        assert_eq!(r.median_threads, 8);
    }

    #[test]
    fn report_serde_round_trips() {
        let report = BenchReport {
            schema_version: REPORT_SCHEMA_VERSION,
            git_sha: "abc123".into(),
            kernel: "6.1.0".into(),
            rss_budget_kib: RSS_BUDGET_KIB,
            rss_hard_cap_kib: RSS_HARD_CAP_KIB,
            rosters: vec![RosterResult::from_samples(
                1,
                vec![sample(15000, 14000, 16000, 6)],
            )],
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: BenchReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rosters.len(), 1);
        assert_eq!(back.rosters[0].samples[0], report.rosters[0].samples[0]);
        assert_eq!(back.rss_hard_cap_kib, RSS_HARD_CAP_KIB);
    }

    #[test]
    fn human_summary_flags_over_cap_roster() {
        let report = BenchReport {
            schema_version: REPORT_SCHEMA_VERSION,
            git_sha: "deadbeef".into(),
            kernel: "6.1.0".into(),
            rss_budget_kib: RSS_BUDGET_KIB,
            rss_hard_cap_kib: RSS_HARD_CAP_KIB,
            rosters: vec![RosterResult::from_samples(
                8,
                vec![sample(40000, 30000, 42000, 12)],
            )],
        };
        let summary = human_summary(&report);
        assert!(summary.contains("8 agents"));
        assert!(summary.contains("⚠️"), "over-cap roster must be flagged");
    }

    #[test]
    fn per_agent_increment_from_min_and_max_rosters() {
        let report = BenchReport {
            schema_version: REPORT_SCHEMA_VERSION,
            git_sha: "x".into(),
            kernel: "6.1.0".into(),
            rss_budget_kib: RSS_BUDGET_KIB,
            rss_hard_cap_kib: RSS_HARD_CAP_KIB,
            rosters: vec![
                RosterResult::from_samples(1, vec![sample(20_000, 0, 0, 6)]),
                RosterResult::from_samples(8, vec![sample(27_000, 0, 0, 6)]),
            ],
        };
        // (27000 - 20000) / (8 - 1) = 1000 KiB per agent.
        assert_eq!(report.per_agent_increment_kib(), Some((1, 8, 1000)));
        assert!(human_summary(&report).contains("Per-agent increment (roster 1→8)"));
    }

    #[test]
    fn per_agent_increment_none_for_single_roster() {
        let report = BenchReport {
            schema_version: REPORT_SCHEMA_VERSION,
            git_sha: "x".into(),
            kernel: "6.1.0".into(),
            rss_budget_kib: RSS_BUDGET_KIB,
            rss_hard_cap_kib: RSS_HARD_CAP_KIB,
            rosters: vec![RosterResult::from_samples(1, vec![sample(20_000, 0, 0, 6)])],
        };
        assert_eq!(report.per_agent_increment_kib(), None);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn sample_self_is_linux_only() {
        assert!(sample_self().is_err());
    }
}
