//! Startup recovery for OpenHuman processes left behind by hard exits.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub argv0: String,
    pub command: String,
}

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use crate::cef_preflight;
    use crate::core_process;
    use crate::process_kill::{kill_pid_force, kill_pid_term};

    pub(crate) use super::ProcessInfo;

    const TERM_GRACE: Duration = Duration::from_millis(500);

    #[derive(Debug, Default, PartialEq, Eq)]
    struct ReapSummary {
        term: usize,
        kill: usize,
        total: usize,
    }

    trait ProcessKiller {
        fn term(&mut self, pid: u32) -> Result<(), String>;
        fn force(&mut self, pid: u32) -> Result<(), String>;
    }

    struct SystemKiller;

    impl ProcessKiller for SystemKiller {
        fn term(&mut self, pid: u32) -> Result<(), String> {
            kill_pid_term(pid)
        }

        fn force(&mut self, pid: u32) -> Result<(), String> {
            kill_pid_force(pid)
        }
    }

    pub(crate) fn reap_stale_openhuman_processes() {
        if core_process::reuse_existing_listener_enabled() {
            log::info!(
                "[startup-recovery] OPENHUMAN_CORE_REUSE_EXISTING=1; skipping stale process reap"
            );
            return;
        }

        if let Some(pid) = live_cef_lock_holder_pid() {
            if pid != std::process::id() as i32 {
                log::info!(
                    "[startup-recovery] live CEF SingletonLock holder pid={pid}; skipping stale process reap so the normal preflight handles the second-instance path"
                );
                return;
            }
        }

        let initial = match enumerate_openhuman_processes() {
            Ok(processes) => processes,
            Err(err) => {
                log::warn!("[startup-recovery] failed to enumerate OpenHuman processes: {err}");
                return;
            }
        };
        let stale = filter_self_pid(&initial, std::process::id());
        if stale.is_empty() {
            log::info!("[startup-recovery] no stale OpenHuman processes found");
            return;
        }

        let mut killer = SystemKiller;
        for process in &stale {
            match killer.term(process.pid) {
                Ok(()) => log::warn!(
                    "[startup-recovery] SIGTERM stale OpenHuman pid={} argv0={}",
                    process.pid,
                    process.argv0
                ),
                Err(err) => log::warn!(
                    "[startup-recovery] failed to SIGTERM stale OpenHuman pid={}: {err}",
                    process.pid
                ),
            }
        }

        std::thread::sleep(TERM_GRACE);

        let after_term = match enumerate_openhuman_processes() {
            Ok(processes) => processes,
            Err(err) => {
                log::warn!(
                    "[startup-recovery] failed to re-enumerate after SIGTERM; skipping SIGKILL escalation: {err}"
                );
                return;
            }
        };
        let summary =
            reap_from_snapshots(&stale, &after_term, std::process::id(), &mut killer, false);
        if summary.kill > 0 {
            log::warn!(
                "[startup-recovery] reap complete term={} kill={} total={}",
                stale.len(),
                summary.kill,
                stale.len()
            );
        } else {
            log::info!(
                "[startup-recovery] reap complete term={} kill=0 total={}",
                stale.len(),
                stale.len()
            );
        }
    }

    pub(crate) fn enumerate_openhuman_processes() -> Result<Vec<ProcessInfo>, String> {
        let Some((contents_dir, main_exe)) = current_bundle_contents_dir() else {
            log::debug!("[startup-recovery] current executable is not inside a .app bundle");
            return Ok(Vec::new());
        };
        let output = std::process::Command::new("ps")
            .args(["-ax", "-o", "pid=,ppid=,command="])
            .output()
            .map_err(|err| format!("spawn ps: {err}"))?;
        if !output.status.success() {
            return Err(format!("ps exited with {}", output.status));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_ps_output(&stdout, &contents_dir, Some(&main_exe)))
    }

    fn reap_from_snapshots(
        initial_stale: &[ProcessInfo],
        after_term: &[ProcessInfo],
        self_pid: u32,
        killer: &mut impl ProcessKiller,
        send_term: bool,
    ) -> ReapSummary {
        let initial_stale = filter_self_pid(initial_stale, self_pid);
        let mut summary = ReapSummary {
            total: initial_stale.len(),
            ..ReapSummary::default()
        };

        if send_term {
            for process in &initial_stale {
                if killer.term(process.pid).is_ok() {
                    summary.term += 1;
                }
            }
        } else {
            summary.term = initial_stale.len();
        }

        let expected: HashMap<u32, &str> = initial_stale
            .iter()
            .map(|process| (process.pid, process.command.as_str()))
            .collect();
        let still_running: Vec<&ProcessInfo> = after_term
            .iter()
            .filter(|process| process.pid != self_pid)
            .filter(|process| {
                expected
                    .get(&process.pid)
                    .is_some_and(|command| *command == process.command)
            })
            .collect();

        for process in still_running {
            match killer.force(process.pid) {
                Ok(()) => {
                    summary.kill += 1;
                    log::warn!(
                        "[startup-recovery] SIGKILL stale OpenHuman pid={} argv0={}",
                        process.pid,
                        process.argv0
                    );
                }
                Err(err) => log::warn!(
                    "[startup-recovery] failed to SIGKILL stale OpenHuman pid={}: {err}",
                    process.pid
                ),
            }
        }

        summary
    }

    fn filter_self_pid(processes: &[ProcessInfo], self_pid: u32) -> Vec<ProcessInfo> {
        let mut seen = HashSet::new();
        processes
            .iter()
            .filter(|process| process.pid != self_pid)
            .filter(|process| seen.insert(process.pid))
            .cloned()
            .collect()
    }

    fn parse_ps_output(
        stdout: &str,
        contents_dir: &Path,
        main_exe: Option<&Path>,
    ) -> Vec<ProcessInfo> {
        stdout
            .lines()
            .filter_map(|line| parse_ps_line(line, contents_dir, main_exe))
            .collect()
    }

    fn parse_ps_line(
        line: &str,
        contents_dir: &Path,
        main_exe: Option<&Path>,
    ) -> Option<ProcessInfo> {
        let line = line.trim_start();
        let (pid_raw, rest) = split_once_whitespace(line)?;
        let (ppid_raw, command) = split_once_whitespace(rest.trim_start())?;
        let command = command.trim().to_string();
        let argv0 = extract_bundle_argv0(&command, contents_dir, main_exe)?;
        Some(ProcessInfo {
            pid: pid_raw.parse().ok()?,
            ppid: ppid_raw.parse().ok()?,
            argv0,
            command,
        })
    }

    fn split_once_whitespace(s: &str) -> Option<(&str, &str)> {
        let idx = s.find(char::is_whitespace)?;
        Some((&s[..idx], &s[idx..]))
    }

    fn extract_bundle_argv0(
        command: &str,
        contents_dir: &Path,
        main_exe: Option<&Path>,
    ) -> Option<String> {
        let command = command.trim_start();
        let contents = contents_dir.to_string_lossy();
        if !command.starts_with(contents.as_ref()) {
            return None;
        }

        if let Some(main_exe) = main_exe {
            let main = main_exe.to_string_lossy();
            if command == main || command.starts_with(&format!("{main} ")) {
                return Some(main.into_owned());
            }
        }

        let frameworks_prefix = format!("{}/Frameworks/", contents);
        if command.starts_with(&frameworks_prefix) {
            let marker = ".app/Contents/MacOS/";
            let marker_idx = command.find(marker)?;
            let bundle_name = Path::new(&command[..marker_idx])
                .file_name()?
                .to_string_lossy();
            let argv0 = format!("{}{}{}", &command[..marker_idx], marker, bundle_name);
            if command == argv0 || command.starts_with(&format!("{argv0} ")) {
                return Some(argv0);
            }
        }

        let first = command.split_whitespace().next()?;
        if Path::new(first).starts_with(contents_dir) {
            Some(first.to_string())
        } else {
            None
        }
    }

    fn current_bundle_contents_dir() -> Option<(PathBuf, PathBuf)> {
        let exe = std::env::current_exe().ok()?;
        let mut cursor = exe.parent();
        while let Some(path) = cursor {
            if path.file_name().is_some_and(|name| name == "Contents")
                && path
                    .parent()
                    .and_then(Path::extension)
                    .is_some_and(|ext| ext == "app")
            {
                return Some((path.to_path_buf(), exe));
            }
            cursor = path.parent();
        }
        None
    }

    fn live_cef_lock_holder_pid() -> Option<i32> {
        let cache_path = cef_cache_path()?;
        let target = fs::read_link(cache_path.join("SingletonLock")).ok()?;
        let target = target.to_string_lossy();
        let (_, pid) = cef_preflight::parse_lock_target(&target)?;
        cef_preflight::is_pid_alive(pid).then_some(pid)
    }

    fn cef_cache_path() -> Option<PathBuf> {
        if let Some(configured) = std::env::var_os("OPENHUMAN_CEF_CACHE_PATH") {
            return Some(PathBuf::from(configured));
        }
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Caches")
                .join(cef_preflight::APP_IDENTIFIER)
                .join("cef"),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn contents_dir() -> PathBuf {
            PathBuf::from("/Applications/OpenHuman.app/Contents")
        }

        fn main_exe() -> PathBuf {
            contents_dir().join("MacOS/OpenHuman")
        }

        #[test]
        fn parse_ps_matches_main_and_helper_bundle_argv0() {
            let stdout = "\
  123   1 /Applications/OpenHuman.app/Contents/MacOS/OpenHuman
  124 123 /Applications/OpenHuman.app/Contents/Frameworks/OpenHuman Helper (Renderer).app/Contents/MacOS/OpenHuman Helper (Renderer) --type=renderer
  999   1 /Applications/Other.app/Contents/MacOS/OpenHuman
";
            let processes = parse_ps_output(stdout, &contents_dir(), Some(&main_exe()));
            assert_eq!(processes.len(), 2);
            assert_eq!(processes[0].pid, 123);
            assert_eq!(processes[0].argv0, main_exe().to_string_lossy());
            assert_eq!(processes[1].pid, 124);
            assert_eq!(
                processes[1].argv0,
                "/Applications/OpenHuman.app/Contents/Frameworks/OpenHuman Helper (Renderer).app/Contents/MacOS/OpenHuman Helper (Renderer)"
            );
        }

        #[test]
        fn filter_self_pid_drops_current_process() {
            let processes = vec![
                ProcessInfo {
                    pid: 10,
                    ppid: 1,
                    argv0: "self".into(),
                    command: "self".into(),
                },
                ProcessInfo {
                    pid: 11,
                    ppid: 1,
                    argv0: "other".into(),
                    command: "other".into(),
                },
            ];
            let filtered = filter_self_pid(&processes, 10);
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].pid, 11);
        }

        #[test]
        fn reap_from_snapshots_escalates_sigkill_for_term_holdouts() {
            #[derive(Default)]
            struct MockKiller {
                term: Vec<u32>,
                force: Vec<u32>,
            }

            impl ProcessKiller for MockKiller {
                fn term(&mut self, pid: u32) -> Result<(), String> {
                    self.term.push(pid);
                    Ok(())
                }

                fn force(&mut self, pid: u32) -> Result<(), String> {
                    self.force.push(pid);
                    Ok(())
                }
            }

            let stale = ProcessInfo {
                pid: 42,
                ppid: 1,
                argv0: main_exe().to_string_lossy().into_owned(),
                command: format!("{}", main_exe().display()),
            };
            let still_running = stale.clone();
            let mut killer = MockKiller::default();
            let summary = reap_from_snapshots(
                std::slice::from_ref(&stale),
                &[still_running],
                99,
                &mut killer,
                true,
            );

            assert_eq!(killer.term, vec![42]);
            assert_eq!(killer.force, vec![42]);
            assert_eq!(
                summary,
                ReapSummary {
                    term: 1,
                    kill: 1,
                    total: 1
                }
            );
        }
    }
}

/// Linux implementation: use /proc/<pid>/cmdline to enumerate openhuman-core processes.
#[cfg(target_os = "linux")]
mod linux_imp {
    use crate::core_process;
    use crate::process_kill::{kill_pid_force, kill_pid_term};
    use std::time::Duration;

    pub(crate) use super::ProcessInfo;

    const TERM_GRACE: Duration = Duration::from_millis(500);

    pub(crate) fn reap_stale_openhuman_processes() {
        if core_process::reuse_existing_listener_enabled() {
            log::info!(
                "[startup-recovery] OPENHUMAN_CORE_REUSE_EXISTING=1; skipping stale process reap"
            );
            return;
        }

        let self_pid = std::process::id();
        log::debug!("[startup-recovery] linux: scanning /proc for stale OpenHuman processes (self_pid={self_pid})");

        let stale = match enumerate_openhuman_processes() {
            Ok(procs) => procs,
            Err(err) => {
                log::warn!("[startup-recovery] linux: failed to enumerate processes: {err}");
                return;
            }
        };

        if stale.is_empty() {
            log::info!("[startup-recovery] linux: no stale OpenHuman processes found");
            return;
        }

        log::info!(
            "[startup-recovery] linux: found {} stale OpenHuman process(es), sending SIGTERM",
            stale.len()
        );
        for proc in &stale {
            match kill_pid_term(proc.pid) {
                Ok(()) => log::warn!(
                    "[startup-recovery] linux: SIGTERM stale OpenHuman pid={} cmd={}",
                    proc.pid,
                    proc.argv0
                ),
                Err(err) => log::warn!(
                    "[startup-recovery] linux: failed to SIGTERM pid={}: {err}",
                    proc.pid
                ),
            }
        }

        std::thread::sleep(TERM_GRACE);

        let after_term = match enumerate_openhuman_processes() {
            Ok(procs) => procs,
            Err(err) => {
                log::warn!("[startup-recovery] linux: failed to re-enumerate after SIGTERM: {err}");
                return;
            }
        };

        let stale_pids: std::collections::HashSet<u32> = stale.iter().map(|p| p.pid).collect();
        let mut kill_count = 0usize;
        for proc in &after_term {
            if stale_pids.contains(&proc.pid) {
                match kill_pid_force(proc.pid) {
                    Ok(()) => {
                        kill_count += 1;
                        log::warn!(
                            "[startup-recovery] linux: SIGKILL stale OpenHuman pid={} cmd={}",
                            proc.pid,
                            proc.argv0
                        );
                    }
                    Err(err) => log::warn!(
                        "[startup-recovery] linux: failed to SIGKILL pid={}: {err}",
                        proc.pid
                    ),
                }
            }
        }

        log::info!(
            "[startup-recovery] linux: reap complete term={} kill={} total={}",
            stale.len(),
            kill_count,
            stale.len()
        );
    }

    pub(crate) fn enumerate_openhuman_processes() -> Result<Vec<ProcessInfo>, String> {
        let self_pid = std::process::id();
        let mut results = Vec::new();

        let proc_dir = std::fs::read_dir("/proc").map_err(|e| format!("read_dir /proc: {e}"))?;

        for entry in proc_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let pid: u32 = match name_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            if pid == self_pid {
                continue;
            }

            let cmdline_path = format!("/proc/{pid}/cmdline");
            let cmdline_bytes = match std::fs::read(&cmdline_path) {
                Ok(b) => b,
                Err(_) => continue,
            };

            // /proc/<pid>/cmdline uses NUL bytes as argument separators.
            let cmdline = cmdline_bytes
                .split(|&b| b == 0)
                .filter(|seg| !seg.is_empty())
                .map(|seg| String::from_utf8_lossy(seg).into_owned())
                .collect::<Vec<_>>();

            let argv0 = match cmdline.first() {
                Some(a) => a.clone(),
                None => continue,
            };

            if !is_openhuman_executable(&argv0) {
                continue;
            }

            let ppid = read_ppid(pid).unwrap_or(0);
            let command = cmdline.join(" ");

            log::debug!(
                "[startup-recovery] linux: found OpenHuman process pid={pid} argv0={argv0}"
            );
            results.push(ProcessInfo {
                pid,
                ppid,
                argv0,
                command,
            });
        }

        Ok(results)
    }

    fn read_ppid(pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // /proc/<pid>/stat: "pid (comm) state ppid ..."
        // The comm field can contain spaces and parens, find the closing ')' first.
        let after_comm = stat.rfind(')')?;
        let rest = stat[after_comm + 1..].trim_start();
        // rest: "state ppid ..."
        let mut parts = rest.split_whitespace();
        let _state = parts.next()?;
        parts.next()?.parse().ok()
    }

    fn is_openhuman_executable(argv0: &str) -> bool {
        let filename = std::path::Path::new(argv0)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(argv0);
        let lower = filename.to_ascii_lowercase();
        lower == "openhuman-core" || lower == "openhuman"
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn is_openhuman_executable_matches_core_binary() {
            assert!(is_openhuman_executable("/usr/local/bin/openhuman-core"));
            assert!(is_openhuman_executable("openhuman-core"));
            assert!(is_openhuman_executable("/opt/OpenHuman/openhuman-core"));
        }

        #[test]
        fn is_openhuman_executable_matches_app_binary() {
            assert!(is_openhuman_executable("/opt/OpenHuman/OpenHuman"));
            assert!(is_openhuman_executable("openhuman"));
        }

        #[test]
        fn is_openhuman_executable_rejects_unrelated() {
            assert!(!is_openhuman_executable("bash"));
            assert!(!is_openhuman_executable("/usr/bin/python3"));
            assert!(!is_openhuman_executable("node"));
        }

        #[test]
        fn enumerate_openhuman_processes_returns_no_self() {
            // Enumerate and confirm self is not in the result.
            let self_pid = std::process::id();
            let result = enumerate_openhuman_processes().expect("enumerate");
            assert!(
                result.iter().all(|p| p.pid != self_pid),
                "self pid {self_pid} must not appear in enumerated list"
            );
        }
    }
}

/// Windows implementation: use sysinfo to enumerate openhuman processes.
#[cfg(target_os = "windows")]
mod windows_imp {
    use crate::core_process;
    use crate::process_kill::{kill_pid_force, kill_pid_term};
    use std::time::Duration;

    pub(crate) use super::ProcessInfo;

    const TERM_GRACE: Duration = Duration::from_millis(500);

    pub(crate) fn reap_stale_openhuman_processes() {
        if core_process::reuse_existing_listener_enabled() {
            log::info!(
                "[startup-recovery] OPENHUMAN_CORE_REUSE_EXISTING=1; skipping stale process reap"
            );
            return;
        }

        let self_pid = std::process::id();
        log::debug!(
            "[startup-recovery] windows: scanning processes for stale OpenHuman (self_pid={self_pid})"
        );

        let stale = match enumerate_openhuman_processes() {
            Ok(procs) => procs,
            Err(err) => {
                log::warn!("[startup-recovery] windows: failed to enumerate processes: {err}");
                return;
            }
        };

        if stale.is_empty() {
            log::info!("[startup-recovery] windows: no stale OpenHuman processes found");
            return;
        }

        log::info!(
            "[startup-recovery] windows: found {} stale OpenHuman process(es), sending terminate",
            stale.len()
        );
        for proc in &stale {
            match kill_pid_term(proc.pid) {
                Ok(()) => log::warn!(
                    "[startup-recovery] windows: TERM stale OpenHuman pid={} exe={}",
                    proc.pid,
                    proc.argv0
                ),
                Err(err) => log::warn!(
                    "[startup-recovery] windows: failed to terminate pid={}: {err}",
                    proc.pid
                ),
            }
        }

        std::thread::sleep(TERM_GRACE);

        let after_term = match enumerate_openhuman_processes() {
            Ok(procs) => procs,
            Err(err) => {
                log::warn!(
                    "[startup-recovery] windows: failed to re-enumerate after terminate: {err}"
                );
                return;
            }
        };

        let stale_pids: std::collections::HashSet<u32> = stale.iter().map(|p| p.pid).collect();
        let mut kill_count = 0usize;
        for proc in &after_term {
            if stale_pids.contains(&proc.pid) {
                match kill_pid_force(proc.pid) {
                    Ok(()) => {
                        kill_count += 1;
                        log::warn!(
                            "[startup-recovery] windows: force-killed stale OpenHuman pid={} exe={}",
                            proc.pid,
                            proc.argv0
                        );
                    }
                    Err(err) => log::warn!(
                        "[startup-recovery] windows: failed to force-kill pid={}: {err}",
                        proc.pid
                    ),
                }
            }
        }

        log::info!(
            "[startup-recovery] windows: reap complete term={} kill={} total={}",
            stale.len(),
            kill_count,
            stale.len()
        );
    }

    pub(crate) fn enumerate_openhuman_processes() -> Result<Vec<ProcessInfo>, String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let self_pid = std::process::id();

        // Use WMIC to enumerate processes with their parent PIDs and executable paths.
        // Output format: Caption,ProcessId,ParentProcessId,ExecutablePath
        let output = std::process::Command::new("wmic")
            .args([
                "process",
                "get",
                "Caption,ProcessId,ParentProcessId,ExecutablePath",
                "/format:csv",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("spawn wmic: {e}"))?;

        if !output.status.success() {
            return Err(format!("wmic exited with {}", output.status));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_wmic_output(&stdout, self_pid))
    }

    fn parse_wmic_output(stdout: &str, self_pid: u32) -> Vec<ProcessInfo> {
        let mut results = Vec::new();
        let mut lines = stdout.lines();

        // Skip header lines until we find the CSV header row.
        let header = loop {
            match lines.next() {
                Some(line) if line.trim().starts_with("Node,") => break line,
                Some(_) => continue,
                None => return results,
            }
        };

        // Find column indices from the header.
        let cols: Vec<&str> = header.split(',').collect();
        let idx_caption = cols.iter().position(|c| c.trim() == "Caption");
        let idx_pid = cols.iter().position(|c| c.trim() == "ProcessId");
        let idx_ppid = cols.iter().position(|c| c.trim() == "ParentProcessId");
        let idx_exe = cols.iter().position(|c| c.trim() == "ExecutablePath");

        let (Some(idx_caption), Some(idx_pid), Some(idx_ppid), Some(idx_exe)) =
            (idx_caption, idx_pid, idx_ppid, idx_exe)
        else {
            log::warn!("[startup-recovery] windows: wmic CSV header missing expected columns");
            return results;
        };

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.splitn(cols.len(), ',').collect();
            if fields.len() < cols.len() {
                continue;
            }

            let caption = fields[idx_caption].trim();
            let exe_path = fields[idx_exe].trim();
            let pid: u32 = match fields[idx_pid].trim().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let ppid: u32 = fields[idx_ppid].trim().parse().unwrap_or(0);

            if pid == self_pid {
                continue;
            }

            let argv0 = if !exe_path.is_empty() {
                exe_path.to_string()
            } else {
                caption.to_string()
            };

            if !is_openhuman_executable(caption, exe_path) {
                continue;
            }

            log::debug!(
                "[startup-recovery] windows: found OpenHuman process pid={pid} argv0={argv0}"
            );
            results.push(ProcessInfo {
                pid,
                ppid,
                argv0: argv0.clone(),
                command: argv0,
            });
        }

        results
    }

    fn is_openhuman_executable(caption: &str, exe_path: &str) -> bool {
        let caption_lower = caption.to_ascii_lowercase();
        let exe_filename = std::path::Path::new(exe_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(exe_path)
            .to_ascii_lowercase();
        caption_lower == "openhuman-core.exe"
            || caption_lower == "openhuman.exe"
            || exe_filename == "openhuman-core.exe"
            || exe_filename == "openhuman.exe"
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parse_wmic_output_finds_openhuman_processes() {
            let csv = "\
Node,Caption,ExecutablePath,ParentProcessId,ProcessId\r\n\
\r\n\
DESKTOP-ABC,openhuman-core.exe,C:\\Program Files\\OpenHuman\\openhuman-core.exe,1234,5678\r\n\
DESKTOP-ABC,chrome.exe,C:\\Program Files\\Google\\Chrome\\chrome.exe,1,9000\r\n\
";
            let results = parse_wmic_output(csv, 9999);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].pid, 5678);
            assert_eq!(results[0].ppid, 1234);
            assert!(results[0].argv0.contains("openhuman-core"));
        }

        #[test]
        fn parse_wmic_output_excludes_self_pid() {
            let csv = "\
Node,Caption,ExecutablePath,ParentProcessId,ProcessId\r\n\
\r\n\
DESKTOP-ABC,openhuman-core.exe,C:\\Program Files\\OpenHuman\\openhuman-core.exe,1,1234\r\n\
";
            let results = parse_wmic_output(csv, 1234);
            assert!(results.is_empty(), "self pid should be excluded");
        }

        #[test]
        fn is_openhuman_executable_matches_core() {
            assert!(is_openhuman_executable(
                "openhuman-core.exe",
                "C:\\path\\openhuman-core.exe"
            ));
            assert!(is_openhuman_executable(
                "OpenHuman.exe",
                "C:\\path\\OpenHuman.exe"
            ));
        }

        #[test]
        fn is_openhuman_executable_rejects_unrelated() {
            assert!(!is_openhuman_executable(
                "chrome.exe",
                "C:\\Chrome\\chrome.exe"
            ));
            assert!(!is_openhuman_executable("python.exe", "C:\\python.exe"));
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use imp::{enumerate_openhuman_processes, reap_stale_openhuman_processes};

#[cfg(target_os = "linux")]
pub(crate) use linux_imp::{enumerate_openhuman_processes, reap_stale_openhuman_processes};

#[cfg(target_os = "windows")]
pub(crate) use windows_imp::{enumerate_openhuman_processes, reap_stale_openhuman_processes};
