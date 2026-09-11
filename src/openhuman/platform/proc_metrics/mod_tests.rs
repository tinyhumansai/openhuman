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
        cpu_user_ms: 0,
        cpu_system_ms: 0,
        open_fds: None,
    }
}

// A realistic `/proc/self/stat` line whose `comm` field embeds spaces and a
// close-paren, to prove the last-`)` split is robust.
const SAMPLE_STAT: &str = "1234 (weird ) name) R 1 1234 1234 0 -1 4194304 500 0 0 0 \
    420 137 0 0 20 0 8 0 99999 123456789 512 18446744073709551615";

#[test]
fn parse_proc_stat_cpu_extracts_utime_stime() {
    let f = parse_proc_stat_cpu(SAMPLE_STAT);
    assert_eq!(f.utime_ticks, 420);
    assert_eq!(f.stime_ticks, 137);
}

#[test]
fn parse_proc_stat_cpu_short_input_stays_zero() {
    assert_eq!(
        parse_proc_stat_cpu("1234 (x) R 1"),
        StatCpuFields::default()
    );
    assert_eq!(parse_proc_stat_cpu(""), StatCpuFields::default());
}

#[test]
fn cpu_ticks_to_ms_converts_and_guards_zero_rate() {
    // 420 ticks at 100 Hz == 4200 ms.
    assert_eq!(cpu_ticks_to_ms(420, 100), 4200);
    assert_eq!(cpu_ticks_to_ms(1000, 0), 0);
    assert_eq!(cpu_ticks_to_ms(1000, -1), 0);
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn sample_self_rejects_unsupported_platform() {
    assert!(sample_self().is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn sample_self_reports_macos_resident_memory() {
    let sample = sample_self().expect("macOS process metrics");
    assert!(sample.rss_kib > 0);
    assert!(sample.vm_hwm_kib >= sample.rss_kib);
    assert!(sample.threads > 0);
    assert!(sample.binary_size_bytes > 0);
    assert_eq!(sample.pss_kib, 0);
    // A live process has consumed at least some CPU and holds open fds.
    assert!(sample.open_fds.map(|n| n > 0).unwrap_or(false));
}
