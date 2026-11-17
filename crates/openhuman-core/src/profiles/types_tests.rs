use super::*;

const RAW: ProfileIdMode = ProfileIdMode::Raw;
const HASHED: ProfileIdMode = ProfileIdMode::Hashed;

fn assert_fits_charset(raw: &str) {
    assert!(!raw.is_empty() && raw.len() <= MAX_RAW_ID_LEN, "{raw}");
    assert!(
        raw.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
        "{raw}"
    );
}

#[test]
fn a_user_maps_to_one_stable_profile() {
    for mode in [RAW, HASHED] {
        let a = ProfileId::for_user("User-123", mode).unwrap();
        assert_eq!(a, ProfileId::for_user("User-123", mode).unwrap());
        assert_ne!(a, ProfileId::for_user("User-124", mode).unwrap());
    }
}

#[test]
fn raw_mode_keeps_ids_that_fit_the_charset() {
    // The desktop's 24-hex backend ids pass unchanged.
    let backend_id = "65f1c0ffee0123456789abcd";
    assert_eq!(
        ProfileId::for_user(backend_id, RAW).unwrap().as_str(),
        backend_id
    );
    assert_eq!(
        ProfileId::for_user("alice_1-b", RAW).unwrap().as_str(),
        "alice_1-b"
    );
    let longest = "a".repeat(MAX_RAW_ID_LEN);
    assert_eq!(
        ProfileId::for_user(&longest, RAW).unwrap().as_str(),
        longest
    );
}

#[test]
fn raw_mode_hashes_everything_else() {
    for user in [
        "Alice",
        "alice@example.com",
        "-leading-dash",
        "_leading",
        "../etc",
        "a/b",
        "local",
        "operator",
        "h-0123456789abcdef0123456789abcdef",
        "h-short",
        &"a".repeat(MAX_RAW_ID_LEN + 1),
    ] {
        let id = ProfileId::for_user(user, RAW).unwrap();
        assert!(id.is_hashed(), "{user} -> {id}");
        assert_ne!(id.as_str(), user);
        assert_fits_charset(id.as_str());
    }
}

#[test]
fn hashed_mode_hashes_every_user_and_hides_the_id() {
    let id = ProfileId::for_user("alice", HASHED).unwrap();
    assert!(id.is_hashed());
    assert!(!id.as_str().contains("alice"));
    assert_fits_charset(id.as_str());
    // The same user hashes the same way in either mode when raw does not apply.
    assert_eq!(
        ProfileId::for_user("Alice", RAW).unwrap(),
        ProfileId::for_user("Alice", HASHED).unwrap()
    );
}

#[test]
fn bad_user_ids_are_refused() {
    for mode in [RAW, HASHED] {
        assert!(ProfileId::for_user("", mode).is_err());
        assert!(ProfileId::for_user(&"x".repeat(MAX_USER_ID_LEN + 1), mode).is_err());
        assert!(ProfileId::for_user("a\nb", mode).is_err());
        assert!(ProfileId::for_user(&"x".repeat(MAX_USER_ID_LEN), mode).is_ok());
    }
}

#[test]
fn only_produced_ids_parse() {
    for mode in [RAW, HASHED] {
        let id = ProfileId::for_user("Some User", mode).unwrap();
        assert_eq!(ProfileId::parse(id.as_str()).unwrap(), id);
    }
    assert!(ProfileId::parse("alice").is_ok());
    assert!(ProfileId::parse("h-0123456789abcdef0123456789abcdef").is_ok());
    for bad in [
        "",
        "local",
        "operator",
        "Alice",
        "h-",
        "h-XYZ",
        "h-../../etc",
        "h-0123456789abcdef0123456789abcdeg",
        "h-0123456789abcdef0123456789abcdef0",
        "../etc",
        "a/b",
        "-a",
        &"a".repeat(MAX_RAW_ID_LEN + 1),
    ] {
        assert!(ProfileId::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn serde_round_trips_and_validates() {
    let id = ProfileId::for_user("u", RAW).unwrap();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(serde_json::from_str::<ProfileId>(&json).unwrap(), id);
    assert!(serde_json::from_str::<ProfileId>("\"operator\"").is_err());
    assert!(serde_json::from_str::<ProfileId>("\"../x\"").is_err());
}

#[test]
fn the_mode_reads_as_lowercase_words() {
    #[derive(serde::Deserialize)]
    struct Wrap {
        mode: ProfileIdMode,
    }
    let raw: Wrap = toml::from_str("mode = \"raw\"").unwrap();
    let hashed: Wrap = toml::from_str("mode = \"hashed\"").unwrap();
    assert_eq!(raw.mode, ProfileIdMode::Raw);
    assert_eq!(hashed.mode, ProfileIdMode::Hashed);
    assert_eq!(ProfileIdMode::default(), ProfileIdMode::Raw);
    assert!(toml::from_str::<Wrap>("mode = \"Raw\"").is_err());
}
