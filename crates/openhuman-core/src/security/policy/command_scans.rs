//! Unbounded system scan detection.
//!
//! Detect broad, system-wide filesystem scans that should be scoped to a workspace
//! or permitted path instead of traversing sensitive OS roots on macOS/Linux.

/// Detect broad, system-wide filesystem scans that should be scoped to a workspace
/// or permitted path instead of traversing sensitive OS roots on macOS/Linux.
pub fn is_unbounded_system_scan(command: &str) -> Option<String> {
    for segment in split_unquoted_segments_simple(command) {
        let cmd_part = skip_env_assignments_simple(&segment);
        let mut words = cmd_part.split_whitespace();
        let Some(base_raw) = words.next() else {
            continue;
        };
        let base = base_raw
            .rsplit('/')
            .next()
            .unwrap_or(base_raw)
            .to_ascii_lowercase();
        let args: Vec<&str> = words.collect();
        if let Some(target) = check_unbounded_segment_scan(&base, &args) {
            return Some(target);
        }
    }
    None
}

/// Core detector for unbounded scans on a single parsed command segment.
pub fn check_unbounded_segment_scan(base: &str, args: &[&str]) -> Option<String> {
    if base == "find" || base == "mfind" {
        for arg in args {
            if matches!(
                *arg,
                "-H" | "-L" | "-P" | "-E" | "-X" | "-d" | "-s" | "-x" | "-f"
            ) || arg.starts_with("-O")
                || arg.starts_with("-D")
            {
                continue;
            }
            if arg.starts_with('-') || *arg == "!" || *arg == "(" || *arg == ")" {
                break;
            }
            let clean = arg.trim_matches('\'').trim_matches('"');
            if is_blocked_system_root(clean) {
                return Some(clean.to_string());
            }
        }
    } else if base == "ls" || base == "dir" {
        let is_recursive_or_wildcard = args
            .iter()
            .any(|arg| (arg.starts_with('-') && arg.contains('R')) || arg.contains('*'));
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            let clean = arg.trim_matches('\'').trim_matches('"');
            if is_blocked_system_root(clean) && (clean.contains('*') || is_recursive_or_wildcard) {
                return Some(clean.to_string());
            }
        }
    } else if base == "grep" || base == "rg" || base == "ripgrep" || base == "ack" {
        let is_recursive = matches!(base, "rg" | "ripgrep" | "ack")
            || args.iter().any(|a| {
                *a == "-r"
                    || *a == "-R"
                    || *a == "--recursive"
                    || (a.starts_with('-')
                        && !a.starts_with("--")
                        && (a.contains('r') || a.contains('R')))
            });
        if is_recursive {
            let files_mode = args.iter().any(|a| *a == "--files");
            let has_explicit_pattern = args.iter().any(|a| {
                *a == "-e"
                    || *a == "--regexp"
                    || a.starts_with("--regexp=")
                    || *a == "-f"
                    || *a == "--file"
                    || a.starts_with("--file=")
            });
            let mut non_flag_args = Vec::new();
            let mut skip_next = false;
            for arg in args {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                if matches!(
                    *arg,
                    "-e" | "-f" | "-g" | "-t" | "-T" | "-m" | "-C" | "-B" | "-A"
                ) {
                    skip_next = true;
                    continue;
                }
                if arg.starts_with('-') {
                    continue;
                }
                non_flag_args.push(*arg);
            }

            let path_operands: &[&str] = if files_mode || has_explicit_pattern {
                &non_flag_args
            } else if non_flag_args.len() > 1 {
                &non_flag_args[1..]
            } else {
                &[]
            };

            for path in path_operands {
                let clean = path.trim_matches('\'').trim_matches('"');
                if is_blocked_system_root(clean) {
                    return Some(clean.to_string());
                }
            }
        }
    }
    None
}

/// Normalize path for root scan checking (strips trailing slashes and  segments).
fn normalize_target_root(path: &str) -> &str {
    let mut p = path.trim_matches('\'').trim_matches('"');
    while p.len() > 1 && (p.ends_with('/') || p.ends_with("/.")) {
        if let Some(stripped) = p.strip_suffix("/.") {
            p = stripped;
        } else if let Some(stripped) = p.strip_suffix('/') {
            p = stripped;
        }
    }
    p
}

/// Test whether a path resolves to a blocked system root or broad scan target.
pub fn is_blocked_system_root(path: &str) -> bool {
    let norm = normalize_target_root(path);
    if norm == "/" || norm == "/*" {
        return true;
    }
    let blocked_exact = [
        "/System",
        "/Library",
        "/Applications",
        "/Volumes",
        "/System/Volumes",
        "/private",
        "/etc",
        "/usr",
        "/var",
    ];
    for b in blocked_exact {
        if norm == b || norm == format!("{b}/*") || norm == format!("{b}*") {
            return true;
        }
    }
    false
}

pub(crate) fn split_unquoted_segments_simple(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
            }
            ';' | '\n' if !in_single && !in_double => {
                if !current.trim().is_empty() {
                    segments.push(current);
                }
                current = String::new();
            }
            '&' if !in_single && !in_double => {
                if chars.peek() == Some(&'&') {
                    chars.next();
                }
                if !current.trim().is_empty() {
                    segments.push(current);
                }
                current = String::new();
            }
            '|' if !in_single && !in_double => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                }
                if !current.trim().is_empty() {
                    segments.push(current);
                }
                current = String::new();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
}

pub(crate) fn skip_env_assignments_simple(segment: &str) -> String {
    let mut rest = segment.trim_start();
    while let Some(word) = rest.split_whitespace().next() {
        if let Some(eq_idx) = word.find('=') {
            let key = &word[..eq_idx];
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                rest = rest[word.len()..].trim_start();
                continue;
            }
        }
        break;
    }
    rest.to_string()
}
