//! The sites the browser never opens, read from the file the host names in
//! `OPENHUMAN_BROWSER_DENY_FILE` (Teeny's bin/spine points it at
//! `deploy/browser-deny.txt`). One host per line; `#` starts a comment; a
//! host covers itself and every subdomain of it. Unset, nothing is denied.
//! Set but unreadable, every page is refused: the list fails closed.
//!
//! `check_url` consults this for every navigation, so `open`, `task` and the
//! `browser_open` tool all refuse a denied host, whatever
//! `OPENHUMAN_BROWSER_ALLOW_ALL` says.

use std::ffi::OsStr;
use std::fs;

/// The environment variable that names the never-open file.
pub(crate) const DENY_FILE_ENV: &str = "OPENHUMAN_BROWSER_DENY_FILE";

/// Why `url` may not be opened under the never-open list the environment
/// names, or None when the browser may open it.
pub(crate) fn refusal(url: &str) -> Option<String> {
    refusal_for(std::env::var_os(DENY_FILE_ENV).as_deref(), url)
}

/// `refusal` for a given file (None: no list is set).
fn refusal_for(file: Option<&OsStr>, url: &str) -> Option<String> {
    let file = file?;
    match fs::read_to_string(file) {
        Ok(text) => check(url, &parse(&text)),
        Err(error) => Some(format!(
            "the never-open list at {} could not be read ({error}), so no page is opened",
            file.to_string_lossy()
        )),
    }
}

/// The hosts listed in `text`, lower-cased, without comments or blank lines.
pub(crate) fn parse(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        .collect()
}

/// Why `url` is denied by `list`, when it is: its host is an entry or under one.
pub(crate) fn check(url: &str, list: &[String]) -> Option<String> {
    let host = url::Url::parse(url).ok().and_then(|parsed| {
        parsed
            .host_str()
            .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
    });
    let Some(host) = host else {
        return Some(format!("'{url}' has no host the never-open list can check"));
    };
    list.iter()
        .find(|denied| host == **denied || host.ends_with(&format!(".{denied}")))
        .map(|denied| format!("host '{host}' is on the never-open list (entry '{denied}')"))
}

#[cfg(test)]
#[path = "browser_never_open_tests.rs"]
mod tests;
