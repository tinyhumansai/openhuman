//! Process-*tree* resident memory sampling.
//!
//! Where [`super::sample_self`] measures only the current process, this module
//! measures the process *and all of its descendants* — the interpreter child
//! processes (`node`, `python`, …) that a skill run or shell tool spawns. The
//! true resource cost of "run this skill" includes those children, which never
//! show up in a self-only RSS reading.
//!
//! [`sample_tree`] returns a [`TreeSample`]: this process's own
//! [`ProcSample`](super::ProcSample), a flat list of descendant
//! [`ChildSample`]s (pid + name + RSS), and `tree_rss_kib` (self + every
//! descendant). Per-child RSS lookups that fail (a child that raced away, or a
//! permission error) are skipped with a `[proc_metrics]` stderr note rather
//! than aborting the whole sample.
//!
//! - **Linux** walks `/proc/*/stat` to recover each pid's `ppid`, chains those
//!   into a descendant set, and reads RSS from `/proc/<pid>/status` (`VmRSS`).
//! - **macOS** enumerates descendants via `proc_listchildpids` (recursively),
//!   names them via `proc_pidinfo(PROC_PIDTBSDINFO)`, and reads RSS via
//!   `proc_pid_rusage` (`ri_resident_size`).
//!
//! The pure graph walk ([`collect_descendants`]) and the Linux `/proc/<pid>/stat`
//! parser ([`parse_stat_comm_ppid`]) are OS-agnostic and unit-tested without a
//! live `/proc`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::ProcSample;

/// One descendant process in a [`TreeSample`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildSample {
    /// Process id of the descendant.
    pub pid: i32,
    /// Executable / accounting name (`node`, `python3`, …). May be empty when
    /// the platform lookup fails.
    pub name: String,
    /// Resident set size of this descendant, in KiB.
    pub rss_kib: u64,
}

/// A process-tree resident-memory sample: this process plus every descendant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeSample {
    /// This process's own self sample.
    pub self_sample: ProcSample,
    /// Every descendant process found at sample time.
    pub children: Vec<ChildSample>,
    /// Self RSS + the RSS of every descendant, in KiB.
    pub tree_rss_kib: u64,
}

impl TreeSample {
    /// Number of descendant processes captured.
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Sum `self.rss` + every child's RSS into `tree_rss_kib`.
    fn assemble(self_sample: ProcSample, children: Vec<ChildSample>) -> Self {
        let tree_rss_kib =
            self_sample.rss_kib + children.iter().map(|child| child.rss_kib).sum::<u64>();
        Self {
            self_sample,
            children,
            tree_rss_kib,
        }
    }
}

/// Collect every transitive descendant of `root` given a `pid -> ppid` map.
///
/// OS-agnostic and pure so it can be unit-tested without a live process table.
/// Cycles (which a live table should never contain, but a racy snapshot might)
/// are broken by a visited set; `root` itself is never included.
pub fn collect_descendants(root: i32, ppid_of: &HashMap<i32, i32>) -> Vec<i32> {
    let mut children_of: HashMap<i32, Vec<i32>> = HashMap::new();
    for (&pid, &ppid) in ppid_of {
        children_of.entry(ppid).or_default().push(pid);
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(parent) = stack.pop() {
        if let Some(kids) = children_of.get(&parent) {
            for &kid in kids {
                if kid != root && seen.insert(kid) {
                    out.push(kid);
                    stack.push(kid);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// Parse the `comm` (name) and `ppid` out of a `/proc/<pid>/stat` line.
///
/// `stat` is `pid (comm) state ppid …`; `comm` is parenthesised and may itself
/// contain spaces and parens, so we take everything between the first `(` and
/// the **last** `)`. After that `)`, whitespace token 0 is `state` and token 1
/// is `ppid`. Missing / malformed input yields `None`.
pub fn parse_stat_comm_ppid(contents: &str) -> Option<(String, i32)> {
    let open = contents.find('(')?;
    let close = contents.rfind(')')?;
    if close <= open {
        return None;
    }
    let comm = contents[open + 1..close].to_string();
    let after = &contents[close + 1..];
    let fields: Vec<&str> = after.split_whitespace().collect();
    // token 0 == state, token 1 == ppid.
    let ppid = fields.get(1)?.parse::<i32>().ok()?;
    Some((comm, ppid))
}

/// Sample this process and every descendant. Linux implementation.
#[cfg(target_os = "linux")]
pub fn sample_tree() -> anyhow::Result<TreeSample> {
    use anyhow::Context;

    let self_pid = std::process::id() as i32;
    let self_sample = super::sample_self().context("sample self for tree")?;

    // Build the full pid -> ppid map and a pid -> name map from /proc/*/stat.
    let mut ppid_of: HashMap<i32, i32> = HashMap::new();
    let mut name_of: HashMap<i32, String> = HashMap::new();
    let entries = std::fs::read_dir("/proc").context("read /proc")?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<i32>() else {
            continue; // non-pid entry (self, cpuinfo, …)
        };
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(contents) => contents,
            Err(_) => continue, // process exited between readdir and read
        };
        if let Some((comm, ppid)) = parse_stat_comm_ppid(&stat) {
            ppid_of.insert(pid, ppid);
            name_of.insert(pid, comm);
        }
    }

    let descendants = collect_descendants(self_pid, &ppid_of);
    let mut children = Vec::with_capacity(descendants.len());
    for pid in descendants {
        let status = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
            Ok(contents) => contents,
            Err(err) => {
                eprintln!(
                    "[proc_metrics] tree: skipping child pid={pid}: status read failed: {err}"
                );
                continue;
            }
        };
        let rss_kib = super::parse_status(&status).vm_rss_kib;
        children.push(ChildSample {
            pid,
            name: name_of.get(&pid).cloned().unwrap_or_default(),
            rss_kib,
        });
    }

    Ok(TreeSample::assemble(self_sample, children))
}

/// Sample this process and every descendant. macOS implementation.
#[cfg(target_os = "macos")]
pub fn sample_tree() -> anyhow::Result<TreeSample> {
    let self_pid = std::process::id() as i32;
    let self_sample = super::sample_self()?;

    let descendants = macos::descendants(self_pid);
    let mut children = Vec::with_capacity(descendants.len());
    for pid in descendants {
        match macos::child_rss_kib(pid) {
            Some(rss_kib) => children.push(ChildSample {
                pid,
                name: macos::proc_name(pid),
                rss_kib,
            }),
            None => {
                eprintln!(
                    "[proc_metrics] tree: skipping child pid={pid}: proc_pid_rusage unavailable (exited or permission denied)"
                );
            }
        }
    }

    Ok(TreeSample::assemble(self_sample, children))
}

/// Unsupported-platform stub — fails loudly rather than fabricating a reading.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn sample_tree() -> anyhow::Result<TreeSample> {
    anyhow::bail!(
        "proc_metrics::sample_tree supports Linux and macOS (this is a {} build)",
        std::env::consts::OS
    )
}

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::HashSet;
    use std::mem::{size_of, MaybeUninit};

    /// Direct children of `ppid` via `proc_listchildpids`. Empty on any error.
    ///
    /// Note the Darwin ABI quirk: `proc_listchildpids` returns the **number of
    /// pids** written (not a byte count, unlike `proc_listpids`), and fills the
    /// buffer with that many `pid_t`. It also doesn't reliably support the NULL
    /// size-probe form, so we allocate a real buffer up front and grow it if the
    /// kernel filled it completely (the list may have been truncated).
    fn child_pids(ppid: i32) -> Vec<i32> {
        let mut cap = 256usize;
        loop {
            let mut buf = vec![0 as libc::pid_t; cap];
            let byte_cap = (buf.len() * size_of::<libc::pid_t>()) as libc::c_int;
            // SAFETY: `buf` is `cap` writable pid_t slots; `byte_cap` matches its size.
            let written = unsafe {
                libc::proc_listchildpids(ppid, buf.as_mut_ptr().cast::<libc::c_void>(), byte_cap)
            };
            if written <= 0 {
                return Vec::new();
            }
            let count = written as usize;
            // Buffer filled to the brim ⇒ the list may be truncated; grow + retry.
            if count >= cap && cap < 65_536 {
                cap *= 4;
                continue;
            }
            buf.truncate(count.min(buf.len()));
            return buf.into_iter().filter(|&pid| pid > 0).collect();
        }
    }

    /// Every transitive descendant of `root`, walked breadth-first through
    /// `proc_listchildpids`. A visited set breaks any cycle a racy snapshot
    /// might present.
    pub(super) fn descendants(root: i32) -> Vec<i32> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = child_pids(root);
        while let Some(pid) = stack.pop() {
            if pid == root || !seen.insert(pid) {
                continue;
            }
            out.push(pid);
            stack.extend(child_pids(pid));
        }
        out.sort_unstable();
        out
    }

    /// Resident set size of `pid` in KiB via `proc_pid_rusage`, or `None` when
    /// the call fails (process exited, or permission denied).
    pub(super) fn child_rss_kib(pid: i32) -> Option<u64> {
        let mut usage = MaybeUninit::<libc::rusage_info_v2>::uninit();
        // SAFETY: `usage` is writable storage of the exact size for
        // `RUSAGE_INFO_V2`; the kernel initializes it on success (return 0).
        let rc = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V2,
                usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
            )
        };
        if rc != 0 {
            return None;
        }
        // SAFETY: `proc_pid_rusage` returned success and initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        Some(usage.ri_resident_size / 1024)
    }

    /// Executable / accounting name of `pid` via `proc_pidinfo(PROC_PIDTBSDINFO)`.
    /// Prefers the longer `pbi_name`, falling back to `pbi_comm`; empty string
    /// when the lookup fails.
    pub(super) fn proc_name(pid: i32) -> String {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
        let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: `info` is writable storage of the exact structure size passed
        // to `proc_pidinfo`, which initializes it when the returned count matches.
        let filled = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast::<libc::c_void>(),
                size,
            )
        };
        if filled != size {
            return String::new();
        }
        // SAFETY: the kernel returned the full structure size.
        let info = unsafe { info.assume_init() };
        let name = cstr_field(&info.pbi_name);
        if name.is_empty() {
            cstr_field(&info.pbi_comm)
        } else {
            name
        }
    }

    /// Convert a fixed-size, NUL-padded `c_char` array into a `String` up to the
    /// first NUL, lossily decoding any non-UTF-8 bytes.
    fn cstr_field(raw: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = raw
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
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
}
