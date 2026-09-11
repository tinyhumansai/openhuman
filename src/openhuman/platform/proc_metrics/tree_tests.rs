use super::*;

#[test]
fn collect_descendants_walks_transitive_chain() {
    // 1 -> 2 -> 4, 1 -> 3; 5 is unrelated (parent 99).
    let map: HashMap<i32, i32> = [(2, 1), (3, 1), (4, 2), (5, 99)].into_iter().collect();
    let mut got = collect_descendants(1, &map);
    got.sort_unstable();
    assert_eq!(got, vec![2, 3, 4]);
}

#[test]
fn collect_descendants_excludes_root_and_unrelated() {
    let map: HashMap<i32, i32> = [(2, 1), (3, 2)].into_iter().collect();
    assert_eq!(collect_descendants(2, &map), vec![3]);
    assert!(collect_descendants(42, &map).is_empty());
}

#[test]
fn collect_descendants_survives_a_cycle() {
    // Degenerate self-parent + mutual cycle must terminate, not hang.
    let map: HashMap<i32, i32> = [(2, 1), (1, 2)].into_iter().collect();
    let got = collect_descendants(1, &map);
    assert_eq!(got, vec![2]);
}

#[test]
fn parse_stat_extracts_comm_and_ppid() {
    // pid (comm) state ppid pgrp …
    let line = "4321 (node) R 4300 4321 4300 0 -1 4194304 100 0 0 0 5 2 0 0 20 0 11 0";
    let (comm, ppid) = parse_stat_comm_ppid(line).unwrap();
    assert_eq!(comm, "node");
    assert_eq!(ppid, 4300);
}

#[test]
fn parse_stat_handles_comm_with_spaces_and_parens() {
    let line = "7 (weird ) proc) S 3 7 3 0 -1 0 0 0 0 0 1 1 0 0 20 0 2 0";
    let (comm, ppid) = parse_stat_comm_ppid(line).unwrap();
    assert_eq!(comm, "weird ) proc");
    assert_eq!(ppid, 3);
}

#[test]
fn parse_stat_rejects_short_or_malformed() {
    assert!(parse_stat_comm_ppid("").is_none());
    assert!(parse_stat_comm_ppid("123 no-parens here").is_none());
    assert!(parse_stat_comm_ppid("1 (x) R").is_none());
}

#[test]
fn assemble_sums_self_plus_children() {
    let self_sample = ProcSample {
        rss_kib: 1000,
        pss_kib: 0,
        private_clean_kib: 0,
        private_dirty_kib: 0,
        vm_hwm_kib: 0,
        threads: 1,
        binary_size_bytes: 0,
        cpu_user_ms: 0,
        cpu_system_ms: 0,
        open_fds: None,
    };
    let children = vec![
        ChildSample {
            pid: 2,
            name: "node".into(),
            rss_kib: 400,
        },
        ChildSample {
            pid: 3,
            name: "python3".into(),
            rss_kib: 250,
        },
    ];
    let tree = TreeSample::assemble(self_sample, children);
    assert_eq!(tree.tree_rss_kib, 1650);
    assert_eq!(tree.child_count(), 2);
}

#[cfg(target_os = "macos")]
#[test]
fn sample_tree_reports_self_on_macos() {
    // A leaf test process has no children, but tree_rss must equal self RSS
    // and the self sample must be populated.
    let tree = sample_tree().expect("macOS tree sample");
    assert!(tree.self_sample.rss_kib > 0);
    assert_eq!(
        tree.tree_rss_kib,
        tree.self_sample.rss_kib + tree.children.iter().map(|c| c.rss_kib).sum::<u64>()
    );
}
