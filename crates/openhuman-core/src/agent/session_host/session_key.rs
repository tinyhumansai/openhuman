//! Session keys: `{unix_ts}_{token}_{agent}`, unique per session host.
//!
//! The leading segment stays whole unix seconds because transcript views read
//! a spawn time from it. The token is twelve lowercase hex digits and carries
//! no `_`, so the agent part can be swapped without parsing the agent name.

const TOKEN_LEN: usize = 12;

/// Maps an agent name onto the characters a transcript stem allows.
pub(crate) fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A fresh key for a session host running `agent`.
pub(crate) fn mint(agent: &str) -> String {
    let unix_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let token = uuid::Uuid::new_v4().simple().to_string();
    let key = format!("{unix_ts}_{}_{}", &token[..TOKEN_LEN], sanitize(agent));
    tracing::debug!(session_key = %key, "[session_host] minted session key");
    key
}

/// `key` with its agent part replaced by `new_agent`, keeping the timestamp
/// and token. A key not minted for `old_agent` is replaced by a fresh one.
pub(crate) fn rename(key: &str, old_agent: &str, new_agent: &str) -> String {
    let prefix = key
        .strip_suffix(sanitize(old_agent).as_str())
        .and_then(|rest| rest.strip_suffix('_'))
        .and_then(|rest| rest.split_once('_'))
        .filter(|(unix, token)| {
            unix.parse::<u64>().is_ok()
                && token.len() == TOKEN_LEN
                && token.bytes().all(|b| b.is_ascii_hexdigit())
        });
    match prefix {
        Some((unix, token)) => format!("{unix}_{token}_{}", sanitize(new_agent)),
        None => {
            tracing::debug!(
                session_key = %key,
                old_agent,
                "[session_host] session key has an unexpected shape; minting a new one"
            );
            mint(new_agent)
        }
    }
}

#[cfg(test)]
#[path = "session_key_tests.rs"]
mod tests;
