//! Property tests for the SaaS gateway layer's header handling. The user and
//! signature headers are attacker-shaped, so for any combination of them:
//!
//! - without the service bearer, every probe with one readable user header
//!   gets the same `401`, whether that user is provisioned or not;
//! - with it, the answer is a 400/401 refusal, or the one user header and the
//!   one signature the request carried are handed on to be resolved.
//!
//! Case counts follow `PROPTEST_CASES` (default 256); CI pins the seed with
//! `PROPTEST_RNG_SEED`.

use super::*;
use axum::body::Body;
use axum::http::HeaderValue;
use proptest::prelude::*;

const SECRET: &str = "service-token";
const NOW: u64 = 1_700_000_000;
const KNOWN: &str = "alice";

/// Raw header bytes: real users, near-misses, signatures and noise. Values
/// `HeaderValue` cannot hold are dropped by the request builder below.
fn user_value() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => prop::sample::select(vec!["alice", "bob", "mallory", "ALICE", " alice", "alice ", "alice,bob", ""])
            .prop_map(|s| s.as_bytes().to_vec()),
        1 => "[ -~]{0,40}".prop_map(String::into_bytes),
        1 => (0usize..9000).prop_map(|n| vec![b'a'; n]),
        1 => prop::collection::vec(any::<u8>(), 0..40),
    ]
}

/// [`user_value`] restricted to values that are one readable header, so the
/// probe gets past the header checks to the bearer.
fn readable_user_value() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => prop::sample::select(vec!["bob", "mallory", "ALICE", " alice", "alice ", "alice,bob", ""])
            .prop_map(|s| s.as_bytes().to_vec()),
        1 => "[\t -~]{0,40}".prop_map(String::into_bytes),
        1 => (0usize..9000).prop_map(|n| vec![b'a'; n]),
    ]
}

fn sig_value() -> impl Strategy<Value = Vec<u8>> {
    let user = prop::sample::select(vec!["alice", "bob", "mallory"]);
    prop_oneof![
        3 => (user.clone(), -120i64..120).prop_map(|(u, skew)| {
            sign(SECRET, u, NOW.saturating_add_signed(skew)).into_bytes()
        }),
        1 => (user, 0usize..64).prop_map(|(u, keep)| {
            let header = sign(SECRET, u, NOW);
            let (head, tag) = header.split_once("v1=").unwrap();
            format!("{head}v1={}", &tag[..keep]).into_bytes()
        }),
        1 => "[ -~]{0,80}".prop_map(String::into_bytes),
        1 => prop::collection::vec(any::<u8>(), 0..80),
    ]
}

fn authorization() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![
        Just(None),
        Just(Some(b"Bearer nope".to_vec())),
        Just(Some(b"Bearer ".to_vec())),
        Just(Some(format!("bearer {SECRET}").into_bytes())),
        Just(Some(format!("Basic {SECRET}").into_bytes())),
        Just(Some(format!("Bearer {SECRET}x").into_bytes())),
        "[ -~]{0,40}".prop_map(|s| Some(s.into_bytes())),
    ]
}

fn request(auth: Option<&[u8]>, users: &[Vec<u8>], sigs: &[Vec<u8>]) -> Request {
    let mut req = Request::builder().uri("/rpc");
    let headers = req.headers_mut().unwrap();
    if let Some(auth) = auth.and_then(|a| HeaderValue::from_bytes(a).ok()) {
        headers.append(header::AUTHORIZATION, auth);
    }
    for value in users.iter().filter_map(|v| HeaderValue::from_bytes(v).ok()) {
        headers.append(USER_HEADER, value);
    }
    for value in sigs.iter().filter_map(|v| HeaderValue::from_bytes(v).ok()) {
        headers.append(USER_SIG_HEADER, value);
    }
    req.body(Body::empty()).unwrap()
}

/// A refusal's status and message, for comparing two of them exactly.
fn parts(refusal: Refused) -> (u16, &'static str) {
    (refusal.status, refusal.message)
}

fn readable_single(values: &[Vec<u8>]) -> Option<String> {
    let readable: Vec<_> = values
        .iter()
        .filter_map(|v| HeaderValue::from_bytes(v).ok())
        .collect();
    match readable.as_slice() {
        [one] => one.to_str().ok().map(String::from),
        _ => None,
    }
}

proptest! {
    /// Without the service bearer, a probe for a known user and one for an
    /// unknown user get identical `401`s.
    #[test]
    fn unauthenticated_probes_cannot_tell_users_apart(
        auth in authorization(),
        unknown in readable_user_value(),
        sigs in prop::collection::vec(sig_value(), 0..3),
    ) {
        prop_assert!(readable_single(std::slice::from_ref(&unknown)).is_some());
        let probe = |user: &[u8]| {
            let req = request(auth.as_deref(), &[user.to_vec()], &sigs);
            decide(&req, Some(SECRET))
        };
        let (Err(known), Err(other)) = (probe(KNOWN.as_bytes()), probe(&unknown)) else {
            return Err(TestCaseError::fail("an unauthenticated user request was admitted"));
        };
        let known = parts(known);
        prop_assert_eq!(known.0, 401);
        prop_assert_eq!(known, parts(other));
    }

    /// With the bearer, any header combination is refused with a client
    /// status or admitted carrying exactly the request's single user header
    /// and single signature.
    #[test]
    fn authenticated_requests_carry_only_what_they_sent(
        users in prop::collection::vec(user_value(), 0..3),
        sigs in prop::collection::vec(sig_value(), 0..3),
    ) {
        let bearer = format!("Bearer {SECRET}").into_bytes();
        let req = request(Some(&bearer), &users, &sigs);
        let user_count = req.headers().get_all(USER_HEADER).iter().count();
        let sig_count = req.headers().get_all(USER_SIG_HEADER).iter().count();
        match decide(&req, Some(SECRET)) {
            Ok(Admitted::Operator) => prop_assert_eq!(user_count, 0),
            Ok(Admitted::User { user, signature }) => {
                prop_assert_eq!(user_count, 1);
                prop_assert!(sig_count <= 1);
                prop_assert_eq!(Some(user), readable_single(&users));
                if sig_count == 1 {
                    prop_assert!(signature.is_none() || signature == readable_single(&sigs));
                } else {
                    prop_assert!(signature.is_none());
                }
            }
            Err(refusal) => {
                prop_assert!(matches!(refusal.status, 400 | 401), "status {}", refusal.status);
                if user_count > 1 || sig_count > 1 {
                    prop_assert_eq!(refusal.status, 400);
                }
            }
        }
    }
}
