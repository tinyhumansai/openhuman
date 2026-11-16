//! Shared helpers for platform service install/lifecycle (all OS targets).
//!
//! Each helper serves only the managers compiled for the current OS, so the
//! ones another OS needs are allowed to be unused here.
#![cfg_attr(
    not(any(target_os = "macos", target_os = "linux", windows)),
    allow(dead_code)
)]

use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// The `ExecStart=` value of a systemd unit: the quoted executable followed by
/// its arguments, each kept as one word.
///
/// systemd splits `ExecStart=` on whitespace, expands `%` specifiers and `$`
/// variables, and reads C-style `\\` / `\"` escapes inside double quotes, so
/// every word is escaped and any word that needs it is quoted. A newline would
/// end the directive, so it is refused.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn systemd_command_line(exe: &Path, args: &[String]) -> Result<String> {
    let mut line = systemd_word(&exe.display().to_string(), true)?;
    for arg in args {
        line.push(' ');
        line.push_str(&systemd_word(arg, false)?);
    }
    Ok(line)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_word(word: &str, always_quote: bool) -> Result<String> {
    if word.contains(['\n', '\r']) {
        anyhow::bail!("a systemd unit argument cannot contain a line break: {word:?}");
    }
    let escaped = word
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    let needs_quotes = always_quote
        || word.is_empty()
        || word
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'));
    Ok(if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    })
}

/// The command line a Windows `.cmd` wrapper runs: the quoted executable
/// followed by its arguments, each kept as one argument.
///
/// A word with spaces, cmd metacharacters or nothing at all is quoted with the
/// MSVC argv rules (backslashes before a closing quote are doubled), and `%` is
/// doubled because a `.cmd` file expands it even inside quotes. An embedded
/// `"` or a line break cannot be written safely for both cmd and the argv
/// parser, so it is refused.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cmd_command_line(exe: &Path, args: &[String]) -> Result<String> {
    let mut line = cmd_word(&exe.display().to_string(), true)?;
    for arg in args {
        line.push(' ');
        line.push_str(&cmd_word(arg, false)?);
    }
    Ok(line)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn cmd_word(word: &str, always_quote: bool) -> Result<String> {
    if word.contains(['"', '\n', '\r']) {
        anyhow::bail!("a Windows service argument cannot contain a quote or line break: {word:?}");
    }
    let word = word.replace('%', "%%");
    let needs_quotes = always_quote
        || word.is_empty()
        || word.chars().any(|c| {
            c.is_whitespace()
                || matches!(c, '&' | '|' | '<' | '>' | '^' | '(' | ')' | ',' | ';' | '=')
        });
    if !needs_quotes {
        return Ok(word);
    }
    // Inside quotes, only backslashes that end the word precede a quote.
    let trailing = word.len() - word.trim_end_matches('\\').len();
    Ok(format!("\"{word}{}\"", "\\".repeat(trailing)))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Suppress conhost allocation for Windows command spawns. Without this,
/// every `schtasks /Query` polled by service status checks flashes a
/// console — visible to users (#1475 follow-up to #731 + #1338).
#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

pub(crate) fn run_checked(cmd: &mut Command) -> Result<()> {
    no_window(cmd);
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("command failed with status {status}");
    }
    Ok(())
}

pub(crate) fn run_capture(cmd: &mut Command) -> Result<String> {
    no_window(cmd);
    let output = cmd.output()?;
    if !output.status.success() {
        anyhow::bail!("command failed with status {}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn run_best_effort(cmd: &mut Command) {
    no_window(cmd);
    match cmd.stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(status) => {
            if !status.success() {
                log::debug!("[service] best-effort command failed with status {status}");
            }
        }
        Err(err) => {
            log::debug!("[service] best-effort command failed to execute: {err}");
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn run_check_silent(cmd: &mut Command) -> bool {
    no_window(cmd);
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
