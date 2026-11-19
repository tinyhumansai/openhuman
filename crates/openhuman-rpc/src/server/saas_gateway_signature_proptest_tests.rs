//! Property tests for the gateway's user signature. The headers are
//! attacker-shaped (a gateway may forward a client's headers by mistake), so
//! `verify` must accept only the full HMAC of exactly the signed user inside
//! the clock window. The core's side, which user's profile a signed request
//! lands on, has its own properties next to `ProfileHost::resolve_user`.
//!
//! Case counts follow `PROPTEST_CASES` (default 256); CI pins the seed with
//! `PROPTEST_RNG_SEED`.

use super::*;
use proptest::prelude::*;

const SECRET: &str = "gateway-secret-0123456789abcdef0123456789";
const NOW: u64 = 1_700_000_000;

/// Users a gateway names: two raw ids and one that the hashed profile id mode
/// would hash.
const KNOWN_USERS: &[&str] = &["alice", "bob", "Carol@example.com"];

/// User header values: provisioned users and near-misses of them (case,
/// whitespace, duplicates joined with a comma, oversized), plus noise.
fn user_header() -> impl Strategy<Value = String> {
    let known = prop::sample::select(KNOWN_USERS.to_vec()).prop_map(String::from);
    prop_oneof![
        3 => known.clone(),
        1 => known.clone().prop_map(|u| u.to_uppercase()),
        1 => known.clone().prop_map(|u| format!(" {u}")),
        1 => known.clone().prop_map(|u| format!("{u} ")),
        1 => known.clone().prop_map(|u| format!("{u}\t")),
        1 => (known.clone(), known.clone()).prop_map(|(a, b)| format!("{a},{b}")),
        1 => known.prop_map(|u| format!("{u}{}", "x".repeat(300))),
        1 => Just("mallory".to_string()),
        1 => "[a-z0-9_-]{1,20}",
        1 => any::<String>(),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// `verify` never panics, and accepts a header only when its `v1` is the
    /// full HMAC of `<t>.<user>` for a `t` inside the window.
    #[test]
    fn verify_only_accepts_a_full_tag_for_this_user(user in user_header(), header in any::<String>()) {
        if verify(SECRET, &user, &header, NOW).is_ok() {
            let ts = header.split(',').filter_map(|p| p.trim().strip_prefix("t=")).next_back();
            let ts: u64 = ts.and_then(|t| t.parse().ok()).expect("accepted without a t");
            prop_assert!(ts.abs_diff(NOW) <= SIGNATURE_WINDOW_SECS);
            prop_assert!(header.to_ascii_lowercase().contains(sign(SECRET, &user, ts).split_once("v1=").unwrap().1));
        }
    }

    /// A signature for one user never verifies for another, at any skew.
    #[test]
    fn a_signature_never_transfers_between_users(
        a in user_header(),
        b in user_header(),
        skew in -(SIGNATURE_WINDOW_SECS as i64)..=SIGNATURE_WINDOW_SECS as i64,
    ) {
        prop_assume!(a != b);
        let header = sign(SECRET, &a, NOW.saturating_add_signed(skew));
        prop_assert!(verify(SECRET, &a, &header, NOW).is_ok());
        prop_assert!(verify(SECRET, &b, &header, NOW).is_err());
    }

    /// A truncated tag never verifies, down to an empty `v1=`.
    #[test]
    fn a_truncated_tag_never_verifies(user in user_header(), keep in 0usize..64) {
        let header = sign(SECRET, &user, NOW);
        let (head, tag) = header.split_once("v1=").unwrap();
        let cut = format!("{head}v1={}", &tag[..keep]);
        prop_assert!(verify(SECRET, &user, &cut, NOW).is_err(), "{cut}");
    }
}
