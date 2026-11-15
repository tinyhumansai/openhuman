//! The sites the browser never opens, read from the file the host names in
//! `OPENHUMAN_BROWSER_DENY_FILE` (Teeny's bin/spine points it at
//! `deploy/browser-deny.txt`). One host per line; `#` starts a comment; a
//! host covers itself and every subdomain of it. Unset, nothing is denied.
//! Set but unreadable, every page is refused: the list fails closed.
//!
//! Two places consult it. `check_url` refuses a navigation the core starts
//! (`open`, `task`, the `browser_open` tool). `BrowserEgressHost::authorize`
//! refuses every connection the browser itself makes, which is where a click
//! or a redirect to a listed host is stopped before its request is sent. A
//! page the browser has already left is not shown, but it is never fetched
//! by a dial the list refuses.

use std::ffi::OsStr;
use std::fs;

/// The environment variable that names the never-open file.
pub(crate) const DENY_FILE_ENV: &str = "OPENHUMAN_BROWSER_DENY_FILE";

/// Why `url` may not be opened under the never-open list the environment
/// names, or None when the browser may open it.
pub(crate) fn refusal(url: &str) -> Option<String> {
    refusal_for(std::env::var_os(DENY_FILE_ENV).as_deref(), url)
}

/// Why a connection to `hostname` is refused under the environment's list, or
/// None when it may be made.
pub(crate) fn dial_refusal(hostname: &str) -> Option<String> {
    dial_refusal_for(std::env::var_os(DENY_FILE_ENV).as_deref(), hostname)
}

/// `refusal` for a given file (None: no list is set).
fn refusal_for(file: Option<&OsStr>, url: &str) -> Option<String> {
    let file = file?;
    match load(file) {
        Ok(list) => check(url, &list),
        Err(reason) => Some(reason),
    }
}

/// `dial_refusal` for a given file (None: no list is set).
fn dial_refusal_for(file: Option<&OsStr>, hostname: &str) -> Option<String> {
    let file = file?;
    match load(file) {
        Ok(list) => check_host(hostname, &list),
        Err(reason) => Some(reason),
    }
}

fn load(file: &OsStr) -> Result<Vec<String>, String> {
    match fs::read_to_string(file) {
        Ok(text) => Ok(parse(&text)),
        Err(error) => Err(format!(
            "the never-open list at {} could not be read ({error}), so no page is opened",
            file.to_string_lossy()
        )),
    }
}

/// The canonical form of a host, the way the URL parser writes it: IDNA and
/// lower-case, without a trailing dot. None when it is not a host.
fn canonical(host: &str) -> Option<String> {
    url::Host::parse(host.trim().trim_end_matches('.'))
        .ok()
        .map(|parsed| parsed.to_string())
}

/// The hosts listed in `text`, canonical, without comments or blank lines.
/// Entries that are not hosts are dropped, never matched.
pub(crate) fn parse(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .filter_map(canonical)
        .collect()
}

/// Why `url` is denied by `list`, when it is: its host is an entry or under one.
pub(crate) fn check(url: &str, list: &[String]) -> Option<String> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned));
    match host {
        Some(host) => check_host(&host, list),
        None => Some(format!("'{url}' has no host the never-open list can check")),
    }
}

/// Why `hostname` is denied by `list`, when it is.
pub(crate) fn check_host(hostname: &str, list: &[String]) -> Option<String> {
    let Some(host) = canonical(hostname) else {
        return Some(format!("'{hostname}' is not a host the never-open list can check"));
    };
    list.iter()
        .find(|denied| host == **denied || host.ends_with(&format!(".{denied}")))
        .map(|denied| format!("host '{host}' is on the never-open list (entry '{denied}')"))
}

#[cfg(test)]
#[path = "browser_never_open_tests.rs"]
mod tests;
