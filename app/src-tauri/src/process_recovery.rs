//! Startup recovery for OpenHuman processes left behind by hard exits.

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use serde::Serialize;

    use crate::cef_preflight;
    use crate::core_process;
    use crate::process_kill::{kill_pid_force, kill_pid_term};

    const TERM_GRACE: Duration = Duration::from_millis(500);

    #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
    pub(crate) struct ProcessInfo {
        pub pid: u32,
        pub ppid: u32,
        pub argv0: String,
        pub command: String,
    }

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

#[cfg(target_os = "macos")]
pub(crate) use imp::{enumerate_openhuman_processes, reap_stale_openhuman_processes, ProcessInfo};

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub argv0: String,
    pub command: String,
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn reap_stale_openhuman_processes() {
    log::debug!("[startup-recovery] skipped on non-macos platform");
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn enumerate_openhuman_processes() -> Result<Vec<ProcessInfo>, String> {
    Ok(Vec::new())
}
