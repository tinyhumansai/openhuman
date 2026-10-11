//! Property tests for the gateway's user header and its signature. Both
//! headers are attacker-shaped (a gateway may forward a client's headers by
//! mistake), so whatever arrives the outcome is a refusal (400, 401 or 403)
//! or the profile of exactly the user that was signed for — never another
//! user's profile, and never the operator plane.
//!
//! Case counts follow `PROPTEST_CASES` (default 256); CI pins the seed with
//! `PROPTEST_RNG_SEED`.

use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
use crate::profiles::ProfileIdMode;
use proptest::prelude::*;
use std::sync::OnceLock;

/// Runs an async host call to completion from a (synchronous) property body.
fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    })
    .block_on(future)
}

const SECRET: &str = "gateway-secret-0123456789abcdef0123456789";
const NOW: u64 = 1_700_000_000;

/// Users provisioned on the shared test host: two raw ids and one that hashes.
const PROVISIONED: &[&str] = &["alice", "bob", "Carol@example.com"];

/// One host per mode, provisioned once and shared by every case (opening a
/// profile is the slow part).
fn host(mode: ProfileIdMode) -> &'static ProfileHost {
    static RAW: OnceLock<(tempfile::TempDir, ProfileHost)> = OnceLock::new();
    static HASHED: OnceLock<(tempfile::TempDir, ProfileHost)> = OnceLock::new();
    let cell = match mode {
        ProfileIdMode::Raw => &RAW,
        ProfileIdMode::Hashed => &HASHED,
    };
    &cell
        .get_or_init(|| {
            let tmp = tempfile::tempdir().unwrap();
            let mut saas = SaasConfig::new(tmp.path());
            saas.profile_ids = mode;
            saas.require_user_signature = true;
            saas.max_profiles_open = 16;
            let host = ProfileHost::new(saas, CoreContext::for_test(DomainSet::full(), None));
            for user in PROVISIONED {
                block(host.provision(&ProfileId::for_user(user, mode).unwrap())).unwrap();
            }
            (tmp, host)
        })
        .1
}

fn mode() -> impl Strategy<Value = ProfileIdMode> {
    prop_oneof![Just(ProfileIdMode::Raw), Just(ProfileIdMode::Hashed)]
}

/// User header values: provisioned users and near-misses of them (case,
/// whitespace, duplicates joined with a comma, oversized), plus noise.
fn user_header() -> impl Strategy<Value = String> {
    let known = prop::sample::select(PROVISIONED.to_vec()).prop_map(String::from);
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

/// How a signature header is built for a request claiming `user`.
#[derive(Debug, Clone)]
enum Sig {
    Missing,
    /// Signed for `signed_for` at `NOW + skew`.
    Signed {
        signed_for: SignedFor,
        skew: i64,
    },
    /// A valid header with the tag cut to `keep` hex characters.
    Truncated {
        keep: usize,
    },
    /// A valid header with extra parts, duplicates or whitespace around it.
    Decorated {
        before: String,
        after: String,
    },
    /// A valid tag paired with a different timestamp.
    WrongT {
        delta: u64,
    },
    Garbage(String),
}

#[derive(Debug, Clone)]
enum SignedFor {
    /// The user the request claims.
    Claimed,
    /// Some other user: a replay of A's signature as B.
    Other(String),
    /// The claimed user under another secret.
    OtherSecret,
}

fn sig() -> impl Strategy<Value = Sig> {
    let signed_for = prop_oneof![
        3 => Just(SignedFor::Claimed),
        2 => prop::sample::select(PROVISIONED.to_vec()).prop_map(|u| SignedFor::Other(u.to_string())),
        1 => "[a-zA-Z0-9@. ]{0,12}".prop_map(SignedFor::Other),
        1 => Just(SignedFor::OtherSecret),
    ];
    let skew = prop_oneof![
        3 => -(SIGNATURE_WINDOW_SECS as i64)..=SIGNATURE_WINDOW_SECS as i64,
        1 => -100_000i64..100_000,
        1 => Just(SIGNATURE_WINDOW_SECS as i64 + 1),
        1 => Just(-(SIGNATURE_WINDOW_SECS as i64) - 1),
    ];
    prop_oneof![
        1 => Just(Sig::Missing),
        4 => (signed_for, skew).prop_map(|(signed_for, skew)| Sig::Signed { signed_for, skew }),
        1 => (0usize..64).prop_map(|keep| Sig::Truncated { keep }),
        1 => ("[ ,=tv1a-f0-9]{0,12}", "[ ,=tv1a-f0-9]{0,12}")
            .prop_map(|(before, after)| Sig::Decorated { before, after }),
        1 => (1u64..10_000).prop_map(|delta| Sig::WrongT { delta }),
        1 => any::<String>().prop_map(Sig::Garbage),
    ]
}

fn build_sig(sig: &Sig, claimed: &str) -> Option<String> {
    let valid = || sign(SECRET, claimed, NOW);
    match sig {
        Sig::Missing => None,
        Sig::Signed { signed_for, skew } => {
            let ts = NOW.saturating_add_signed(*skew);
            Some(match signed_for {
                SignedFor::Claimed => sign(SECRET, claimed, ts),
                SignedFor::Other(other) => sign(SECRET, other, ts),
                SignedFor::OtherSecret => sign("another-secret", claimed, ts),
            })
        }
        Sig::Truncated { keep } => {
            let header = valid();
            let (head, tag) = header.split_once("v1=").unwrap();
            Some(format!("{head}v1={}", &tag[..(*keep).min(tag.len())]))
        }
        Sig::Decorated { before, after } => Some(format!("{before}{}{after}", valid())),
        Sig::WrongT { delta } => {
            let tag = valid().split_once("v1=").unwrap().1.to_string();
            Some(format!("t={},v1={tag}", NOW + delta))
        }
        Sig::Garbage(s) => Some(s.clone()),
    }
}

/// Whether `header` would verify for `user` at `NOW`, recomputed from first
/// principles: some `t` within the window, and a `v1` equal to the full tag.
fn is_valid_for(user: &str, header: &str) -> bool {
    verify(SECRET, user, header, NOW).is_ok()
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

    /// Whatever the headers, resolving a named user yields a refusal with a
    /// client status, or exactly the profile of the claimed user — and only
    /// when the signature is genuinely valid for that user.
    #[test]
    fn resolving_a_user_is_a_refusal_or_their_own_profile(
        user in user_header(),
        sig in sig(),
        mode in mode(),
    ) {
        let host = host(mode);
        let header = build_sig(&sig, &user);
        match block(resolve_user_on(host, &user, header.as_deref(), SECRET, NOW)) {
            Ok(profile) => {
                let expected = ProfileId::for_user(&user, mode).unwrap();
                prop_assert_eq!(&profile.id, &expected, "{:?} landed on another profile", user);
                prop_assert_eq!(profile.context().profile(), Some(expected.as_str()));
                prop_assert_eq!(profile.context().session_agent(), None);
                prop_assert!(PROVISIONED.contains(&user.as_str()), "{:?} opened", user);
                prop_assert!(is_valid_for(&user, header.as_deref().unwrap_or("")));
            }
            Err(refusal) => {
                prop_assert!(matches!(refusal.status, 400 | 401 | 403), "{refusal:?}");
            }
        }
    }

    /// Without a valid signature a provisioned user and an unknown one get
    /// the same refusal, so a caller with a bad signature cannot enumerate
    /// profiles.
    #[test]
    fn bad_signatures_do_not_reveal_provisioning(sig in sig(), mode in mode(), known in prop::sample::select(PROVISIONED.to_vec())) {
        let host = host(mode);
        let unknown = "mallory";
        let known_header = build_sig(&sig, known);
        let unknown_header = build_sig(&sig, unknown);
        // A genuinely valid signature is not a probe (and would push proptest
        // past its global reject cap at high case counts if assumed away).
        if is_valid_for(known, known_header.as_deref().unwrap_or(""))
            || is_valid_for(unknown, unknown_header.as_deref().unwrap_or(""))
        {
            return Ok(());
        }
        let a = block(resolve_user_on(host, known, known_header.as_deref(), SECRET, NOW)).unwrap_err();
        let b = block(resolve_user_on(host, unknown, unknown_header.as_deref(), SECRET, NOW)).unwrap_err();
        prop_assert_eq!(a.status, 401);
        prop_assert_eq!(a, b);
    }
}
