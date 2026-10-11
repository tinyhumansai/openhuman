//! Property tests for profile ids: the gateway user id is untrusted, and the
//! id it becomes is a path segment (`<root>/users/<id>`) and a memory
//! namespace. Whatever comes in, the result must be a refusal or an id in the
//! documented charset that stays one segment under `users/`, and two users
//! must never share one.
//!
//! Case counts follow `PROPTEST_CASES` (default 256); CI pins the seed with
//! `PROPTEST_RNG_SEED`.

use super::*;
use crate::config::schema::profile_layout::{users_dir, ProfileLayout};
use proptest::prelude::*;
use std::path::Component;

/// Path-ish fragments that must never survive into an id.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("..".to_string()),
        Just(".".to_string()),
        Just("/".to_string()),
        Just("\\".to_string()),
        Just("\0".to_string()),
        Just(":".to_string()),
        Just(" ".to_string()),
        Just("h-".to_string()),
        Just("local".to_string()),
        Just("operator".to_string()),
        "[a-z0-9_-]{1,8}",
        "[A-Z]{1,4}",
        "\\PC{1,3}",
    ]
}

/// Untrusted gateway user ids: arbitrary unicode, path segments, reserved
/// names in any case, hashed-looking ids, and lengths up to 512.
fn user_id() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[a-z0-9_-]{0,80}",
        prop::collection::vec(fragment(), 0..12).prop_map(|parts| parts.concat()),
        (
            prop::sample::select(vec![
                "local", "operator", "h-", "Local", "LOCAL", "OPERATOR", "H-"
            ]),
            "[a-fA-F0-9]{0,40}",
        )
            .prop_map(|(head, tail)| format!("{head}{tail}")),
        prop::collection::vec(any::<char>(), 0..=512).prop_map(String::from_iter),
        (
            0usize..=512,
            prop::sample::select(vec!['a', '0', '-', 'Z', '/', 'é'])
        )
            .prop_map(|(n, c)| std::iter::repeat_n(c, n).collect()),
    ]
}

fn mode() -> impl Strategy<Value = ProfileIdMode> {
    prop_oneof![Just(ProfileIdMode::Raw), Just(ProfileIdMode::Hashed)]
}

/// The documented id charset: `^[a-z0-9][a-z0-9_-]{0,63}$` (not reserved, not
/// in the hashed namespace) or `^h-[0-9a-f]{32}$`.
fn matches_documented_charset(id: &str) -> bool {
    let raw = !id.is_empty()
        && id.len() <= MAX_RAW_ID_LEN
        && id
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && !RESERVED_IDS.contains(&id)
        && !id.starts_with("h-");
    let hashed = id.strip_prefix("h-").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    raw || hashed
}

/// Whether `for_user` must refuse `user_id`, whatever the mode.
fn must_refuse(user_id: &str) -> bool {
    user_id.is_empty() || user_id.len() > MAX_USER_ID_LEN || user_id.chars().any(char::is_control)
}

/// `dir` is exactly one normal component below `users`, and that component is
/// the id.
fn is_one_segment_under(users: &std::path::Path, dir: &std::path::Path, id: &str) -> bool {
    let Ok(rest) = dir.strip_prefix(users) else {
        return false;
    };
    let components: Vec<_> = rest.components().collect();
    matches!(components.as_slice(), [Component::Normal(seg)] if *seg == std::ffi::OsStr::new(id))
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// `for_user` refuses exactly the documented inputs and otherwise yields an
    /// id in the charset, with no separator, that parses back to itself.
    #[test]
    fn for_user_refuses_or_yields_a_charset_id(user in user_id(), mode in mode()) {
        match ProfileId::for_user(&user, mode) {
            Err(_) => prop_assert!(must_refuse(&user), "refused a valid user id {user:?}"),
            Ok(id) => {
                prop_assert!(!must_refuse(&user), "accepted {user:?}");
                let s = id.as_str();
                prop_assert!(matches_documented_charset(s), "{user:?} -> {s:?}");
                prop_assert!(!s.contains(['/', '\\', '.', '\0', ':']), "{s:?}");
                prop_assert!(!RESERVED_IDS.contains(&s));
                prop_assert_eq!(ProfileId::parse(s), Ok(id.clone()));
                let json = serde_json::to_string(&id).unwrap();
                prop_assert_eq!(serde_json::from_str::<ProfileId>(&json).unwrap(), id.clone());
                if mode == ProfileIdMode::Hashed {
                    prop_assert!(id.is_hashed());
                }
            }
        }
    }

    /// Raw mode keeps exactly the ids already in the raw form and hashes the
    /// rest the same way hashed mode does; hashed mode is deterministic.
    #[test]
    fn raw_mode_passes_only_the_raw_form(user in user_id()) {
        let (Ok(raw), Ok(hashed)) = (
            ProfileId::for_user(&user, ProfileIdMode::Raw),
            ProfileId::for_user(&user, ProfileIdMode::Hashed),
        ) else {
            return Ok(());
        };
        prop_assert_eq!(&hashed, &ProfileId::for_user(&user, ProfileIdMode::Hashed).unwrap());
        if raw.is_hashed() {
            prop_assert_eq!(&raw, &hashed);
        } else {
            prop_assert_eq!(raw.as_str(), user.as_str());
        }
    }

    /// Two distinct users never share a profile, in either mode.
    #[test]
    fn distinct_users_get_distinct_ids(a in user_id(), b in user_id(), mode in mode()) {
        prop_assume!(a != b);
        if let (Ok(ia), Ok(ib)) = (ProfileId::for_user(&a, mode), ProfileId::for_user(&b, mode)) {
            prop_assert_ne!(ia, ib, "{:?} and {:?} collide", a, b);
        }
    }

    /// Sending another user's id as your own user id never lands on their
    /// profile: an `h-…` input is not in the raw form, so it is hashed again,
    /// and case variants of a raw id are hashed too.
    #[test]
    fn an_id_used_as_a_user_id_never_aliases_its_owner(user in user_id(), mode in mode()) {
        let Ok(owner) = ProfileId::for_user(&user, mode) else { return Ok(()); };
        let claimed = ProfileId::for_user(owner.as_str(), mode).unwrap();
        if owner.is_hashed() || mode == ProfileIdMode::Hashed {
            prop_assert_ne!(&claimed, &owner);
        }
        let upper = owner.as_str().to_ascii_uppercase();
        if upper != owner.as_str() {
            prop_assert_ne!(ProfileId::for_user(&upper, mode).unwrap(), owner);
        }
    }

    /// `parse` accepts exactly the documented charset, and anything it accepts
    /// round-trips.
    #[test]
    fn parse_accepts_exactly_the_charset(raw in user_id()) {
        match ProfileId::parse(&raw) {
            Ok(id) => {
                prop_assert!(matches_documented_charset(&raw), "{raw:?}");
                prop_assert_eq!(id.as_str(), raw.as_str());
            }
            Err(_) => prop_assert!(!matches_documented_charset(&raw), "{raw:?}"),
        }
    }

    /// The profile directory is one segment directly under `users/`, lexically.
    #[test]
    fn layout_dir_is_one_segment_under_users(user in user_id(), mode in mode()) {
        let Ok(id) = ProfileId::for_user(&user, mode) else { return Ok(()); };
        let root = std::path::Path::new("/srv/openhuman");
        let layout = ProfileLayout::new(root, &id);
        prop_assert!(is_one_segment_under(&users_dir(root), &layout.dir, id.as_str()), "{:?}", layout.dir);
        for path in [&layout.meta_path, &layout.config_path, &layout.workspace_dir, &layout.sandbox_dir] {
            prop_assert!(path.starts_with(&layout.dir));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        // Each case touches the filesystem.
        cases: ProptestConfig::default().cases.min(256),
        ..ProptestConfig::default()
    })]

    /// Once created and canonicalised on a real directory, the profile
    /// directory still sits strictly under `users/`, and is a direct child.
    #[test]
    fn layout_dir_is_under_users_after_canonicalising(user in user_id(), mode in mode()) {
        let Ok(id) = ProfileId::for_user(&user, mode) else { return Ok(()); };
        let tmp = tempfile::tempdir().unwrap();
        let layout = ProfileLayout::new(tmp.path(), &id);
        std::fs::create_dir_all(&layout.dir).unwrap();
        let users = std::fs::canonicalize(users_dir(tmp.path())).unwrap();
        let dir = std::fs::canonicalize(&layout.dir).unwrap();
        prop_assert!(dir != users, "{:?}", dir);
        prop_assert!(is_one_segment_under(&users, &dir, id.as_str()), "{:?} not under {:?}", dir, users);
    }
}
