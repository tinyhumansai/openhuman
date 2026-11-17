//! Property tests for the tenant keys: every per-tenant table and storage
//! scope rests on these encodings being injective, so they are checked over
//! arbitrary `(profile, agent, id)` triples — including the separator bytes
//! the encodings use (`\x1e`, `\x1f`, `~`, `:`, `%`) and digits that look
//! like length prefixes.

use super::*;
use proptest::prelude::*;

/// Short strings biased towards the bytes the encodings are built from.
fn part() -> impl Strategy<Value = String> {
    let ch = prop_oneof![
        3 => prop_oneof![
            Just('\u{1e}'),
            Just('\u{1f}'),
            Just('~'),
            Just(':'),
            Just('%'),
            Just('0'),
            Just('1'),
            Just('7'),
            Just('a'),
            Just('b'),
            Just(' '),
        ],
        1 => any::<char>(),
    ];
    proptest::collection::vec(ch, 0..8).prop_map(String::from_iter)
}

fn tenant() -> impl Strategy<Value = Tenant> {
    (proptest::option::of(part()), proptest::option::of(part()))
        .prop_map(|(profile, agent)| Tenant { profile, agent })
}

/// `session_key` treats an agent named `default` as the default agent.
fn normalized(t: &Tenant) -> Tenant {
    Tenant {
        profile: t.profile.clone(),
        agent: t
            .agent
            .clone()
            .filter(|a| a != crate::storage::sessions::DEFAULT_AGENT),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn tenant_key_is_injective(a in tenant(), x in part(), b in tenant(), y in part()) {
        let same = tenant_key(&a, &x) == tenant_key(&b, &y);
        prop_assert_eq!(same, (&a, &x) == (&b, &y));
    }

    #[test]
    fn tenant_key_splits_back(t in tenant(), id in part()) {
        let key = tenant_key(&t, &id);
        prop_assert_eq!(split_key(&key), Some((t, id.as_str())));
    }

    #[test]
    fn a_tenant_without_profile_or_agent_keeps_plain_ids(id in "[a-zA-Z0-9_:~-]{0,24}") {
        prop_assert_eq!(tenant_key(&Tenant::default(), &id), id);
    }

    #[test]
    fn session_key_is_injective_across_profiles(
        p in part(), a in proptest::option::of(part()),
        q in part(), b in proptest::option::of(part()),
    ) {
        let left = Tenant { profile: Some(p), agent: a };
        let right = Tenant { profile: Some(q), agent: b };
        let same = session_key(&left) == session_key(&right);
        prop_assert_eq!(same, normalized(&left) == normalized(&right));
    }

    #[test]
    fn web_chat_keys_unscope_to_the_callers_id(t in tenant(), id in part()) {
        let key = crate::web_chat::key_in(&t, &id);
        prop_assert_eq!(crate::web_chat::unscope_in(&t, &key), id);
    }

    #[test]
    fn storage_scopes_never_collide_across_profiles(
        p in part(), q in part(), a in proptest::option::of(part()), saas in any::<bool>(),
    ) {
        let left = crate::storage::scope_from(Some(&p), a.as_deref(), saas).unwrap();
        let right = crate::storage::scope_from(Some(&q), None, saas).unwrap();
        prop_assert_eq!(left == right, p == q);
        prop_assert!(!left.is_local());
    }

    #[test]
    fn saas_storage_is_never_local(
        p in proptest::option::of(part()), a in proptest::option::of(part()),
    ) {
        match crate::storage::scope_from(p.as_deref(), a.as_deref(), true) {
            Ok(scope) => {
                prop_assert!(p.is_some(), "only a profile gets a SaaS scope");
                prop_assert!(!scope.is_local());
            }
            Err(_) => prop_assert!(p.is_none()),
        }
    }
}
