//! Account-id shortening for yuanbao.
//!
//! Yuanbao uids (`from_account`) are 64-char hashes assigned by the platform.
//! The composite `ChannelMessage` thread_id that downstream consumers derive
//! from `sender` and `reply_target` (`channel:yuanbao_<sender>_<reply_target>`)
//! becomes ~145 chars. After the conversation store hex-encodes that for the
//! per-thread JSONL filename it grows to ~296 chars, exceeding `NAME_MAX`
//! (255 bytes) on ext4/HFS+/APFS/NTFS — writes fail with `ENAMETOOLONG` and
//! channel history is lost.
//!
//! Rather than push the filesystem limit into shared `ConversationStore` code,
//! we shorten yuanbao-specific ids at the channel boundary. Internal yuanbao
//! state (echo guard, access control, owner-command check) keeps the original
//! `from_account` — only the value emitted on `ChannelMessage.sender` /
//! `ChannelMessage.reply_target` is shortened.
//!
//! Format: `<first 8 chars of uid>_<first 16 hex chars of sha256(uid)>`.
//! The 8-char prefix keeps logs roughly groupable for the same user; the
//! sha256 suffix guarantees uniqueness across uids that share a prefix.

use sha2::{Digest, Sha256};

/// Max raw account-id length before the shortening kicks in.
///
/// Anything shorter is passed through unchanged so short upstream-style ids
/// (e.g. numeric ids, future protocol changes) keep their natural form.
const ACCOUNT_ID_PASSTHROUGH_MAX: usize = 24;

/// Shorten a yuanbao account id for use in `ChannelMessage.sender` /
/// `ChannelMessage.reply_target`. See module docs for rationale.
pub(super) fn shorten_account_id(uid: &str) -> String {
    if uid.len() <= ACCOUNT_ID_PASSTHROUGH_MAX {
        return uid.to_string();
    }
    let prefix: String = uid.chars().take(8).collect();
    let digest = Sha256::digest(uid.as_bytes());
    format!("{prefix}_{:.16x}", digest)
}

/// Shorten a yuanbao `reply_target`, preserving the `g:<group_code>` shape
/// used for group chats. The `g:` discriminator is required by outbound
/// routing (see [`super::types::InboundContext::reply_target`]).
pub(super) fn shorten_reply_target(reply_target: &str) -> String {
    if let Some(group_code) = reply_target.strip_prefix("g:") {
        format!("g:{}", shorten_account_id(group_code))
    } else {
        shorten_account_id(reply_target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_short_ids_through_unchanged() {
        assert_eq!(shorten_account_id("123456"), "123456");
        assert_eq!(shorten_account_id(""), "");
        let exactly_max = "a".repeat(ACCOUNT_ID_PASSTHROUGH_MAX);
        assert_eq!(shorten_account_id(&exactly_max), exactly_max);
    }

    #[test]
    fn shortens_long_ids_to_prefix_plus_hash() {
        let long_uid = "a".repeat(64);
        let shortened = shorten_account_id(&long_uid);
        assert_eq!(shortened.len(), 8 + 1 + 16, "8 prefix + '_' + 16 hex");
        assert!(shortened.starts_with("aaaaaaaa_"));
    }

    #[test]
    fn shortening_is_deterministic_and_collision_resistant() {
        let a = "f".repeat(64);
        let mut b = a.clone();
        b.replace_range(63..64, "e"); // differ in last char only
        let sa = shorten_account_id(&a);
        let sb = shorten_account_id(&b);
        assert_eq!(sa, shorten_account_id(&a), "deterministic");
        assert_ne!(sa, sb, "different uids hash to different ids");
    }

    #[test]
    fn group_reply_target_preserves_g_prefix() {
        let short_group = shorten_reply_target("g:short_group");
        assert_eq!(short_group, "g:short_group");

        let long_code = "a".repeat(64);
        let long_group = format!("g:{long_code}");
        let shortened = shorten_reply_target(&long_group);
        assert!(shortened.starts_with("g:aaaaaaaa_"));
        assert_eq!(shortened.len(), 2 + 8 + 1 + 16);
    }

    #[test]
    fn dm_reply_target_shortens_like_account_id() {
        let uid = "z".repeat(64);
        assert_eq!(shorten_reply_target(&uid), shorten_account_id(&uid));
    }

    #[test]
    fn shortened_thread_id_fits_under_name_max() {
        // Simulate the worst case: long uid for sender + reply_target.
        let uid = "f".repeat(64);
        let sender = shorten_account_id(&uid);
        let reply_target = shorten_account_id(&uid);
        let thread_id = format!("channel:yuanbao_{sender}_{reply_target}");
        // hex-encoded filename used by ConversationStore (`<hex>.jsonl`).
        let hex_name_len = thread_id.len() * 2 + ".jsonl".len();
        // NAME_MAX on common filesystems is 255 bytes.
        assert!(
            hex_name_len <= 255,
            "shortened thread_id hex filename ({hex_name_len} bytes) must fit under NAME_MAX (255)"
        );
    }
}
