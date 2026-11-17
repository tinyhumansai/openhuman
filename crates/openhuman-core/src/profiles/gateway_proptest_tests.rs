//! Property tests for resolving a gateway user to a profile. The user header
//! is attacker-shaped (a gateway may forward a client's headers by mistake),
//! so whatever arrives the outcome is a refusal or the profile of exactly the
//! claimed user — never another user's profile — and only once the signature
//! check accepted. The HMAC itself is the server's (`openhuman-rpc`, with its
//! own properties); here a stand-in check accepts `sig:<user>` alone.
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

/// The signature a request claiming `user` carries.
#[derive(Debug, Clone)]
enum Sig {
    Missing,
    /// Valid for the claimed user.
    Valid,
    /// Valid for another user: a replay of A's signature as B.
    For(String),
    Garbage(String),
}

fn sig() -> impl Strategy<Value = Sig> {
    prop_oneof![
        1 => Just(Sig::Missing),
        3 => Just(Sig::Valid),
        2 => prop::sample::select(PROVISIONED.to_vec()).prop_map(|u| Sig::For(u.to_string())),
        1 => any::<String>().prop_map(Sig::Garbage),
    ]
}

fn build_sig(sig: &Sig, claimed: &str) -> Option<String> {
    match sig {
        Sig::Missing => None,
        Sig::Valid => Some(format!("sig:{claimed}")),
        Sig::For(other) => Some(format!("sig:{other}")),
        Sig::Garbage(s) => Some(s.clone()),
    }
}

/// The stand-in signature check: accepts `sig:<user>` for `user` alone.
fn check(user: &str, header: Option<&str>) -> Result<(), String> {
    match header {
        None => Err("missing signature".to_string()),
        Some(header) if header == format!("sig:{user}") => Ok(()),
        Some(_) => Err("user signature does not match".to_string()),
    }
}

fn resolve(
    host: &ProfileHost,
    user: &str,
    header: Option<&str>,
) -> Result<Arc<Profile>, ResolveUserError> {
    block(host.resolve_user(user, || check(user, header)))
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Whatever the headers, resolving a named user yields a refusal, or
    /// exactly the profile of the claimed user — and only when the signature
    /// check accepted it.
    #[test]
    fn resolving_a_user_is_a_refusal_or_their_own_profile(
        user in user_header(),
        sig in sig(),
        mode in mode(),
    ) {
        let host = host(mode);
        let header = build_sig(&sig, &user);
        match resolve(host, &user, header.as_deref()) {
            Ok(profile) => {
                let expected = ProfileId::for_user(&user, mode).unwrap();
                prop_assert_eq!(&profile.id, &expected, "{:?} landed on another profile", user);
                prop_assert_eq!(profile.context().profile(), Some(expected.as_str()));
                prop_assert_eq!(profile.context().session_agent(), None);
                prop_assert!(PROVISIONED.contains(&user.as_str()), "{:?} opened", user);
                prop_assert!(check(&user, header.as_deref()).is_ok());
            }
            Err(ResolveUserError::InvalidUser(_) | ResolveUserError::Signature(_)) => {}
            Err(ResolveUserError::Open(error)) => {
                prop_assert!(matches!(error, OpenError::NotProvisioned(_)), "{error:?}");
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
        // A valid signature is not a probe.
        if check(known, known_header.as_deref()).is_ok()
            || check(unknown, unknown_header.as_deref()).is_ok()
        {
            return Ok(());
        }
        let a = resolve(host, known, known_header.as_deref()).unwrap_err();
        let b = resolve(host, unknown, unknown_header.as_deref()).unwrap_err();
        prop_assert!(matches!(a, ResolveUserError::Signature(_)), "{a:?}");
        prop_assert_eq!(a, b);
    }
}
