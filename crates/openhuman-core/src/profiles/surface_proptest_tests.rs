//! Property tests for the SaaS surface: which methods each scope may reach,
//! and which thread ids a user may mint. Method names and thread ids both
//! come straight from the request, so near-misses (case, prefix, suffix,
//! `.`/`_` swaps, unicode lookalikes) must land on the closed side.
//!
//! Case counts follow `PROPTEST_CASES` (default 256); CI pins the seed with
//! `PROPTEST_RNG_SEED`.

use super::*;
use proptest::prelude::*;

/// The operator plane as the registry tags it: every `profiles.*` controller
/// (`DomainGroup::Operator` in `core::all`).
fn operator_methods() -> Vec<String> {
    crate::profiles::all_profiles_registered_controllers()
        .iter()
        .map(|c| c.rpc_method_name())
        .collect()
}

/// Real method names from both planes plus a few closed ones.
fn known_methods() -> Vec<String> {
    let mut methods: Vec<String> = USER_METHODS.iter().map(|m| m.to_string()).collect();
    methods.extend(operator_methods());
    methods.extend(
        [
            "openhuman.threads_delete",
            "openhuman.threads_purge",
            "openhuman.config_get_config",
            "openhuman.config_update_autonomy_settings",
            "openhuman.channel_web_send",
        ]
        .map(String::from),
    );
    methods
}

/// One near-miss edit of a real method name.
#[derive(Debug, Clone)]
enum Mutation {
    None,
    Upper,
    UpperFirst,
    Prefix(String),
    Suffix(String),
    DotToUnderscore,
    UnderscoreToDot,
    Trim(usize),
    Pad,
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        Just(Mutation::None),
        Just(Mutation::Upper),
        Just(Mutation::UpperFirst),
        "[ ._/a-z\\x00]{1,4}".prop_map(Mutation::Prefix),
        "[ ._/a-z\\x00]{1,4}".prop_map(Mutation::Suffix),
        Just(Mutation::DotToUnderscore),
        Just(Mutation::UnderscoreToDot),
        (1usize..4).prop_map(Mutation::Trim),
        Just(Mutation::Pad),
    ]
}

fn apply(method: &str, mutation: &Mutation) -> String {
    match mutation {
        Mutation::None => method.to_string(),
        Mutation::Upper => method.to_uppercase(),
        Mutation::UpperFirst => {
            let mut chars = method.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
        Mutation::Prefix(p) => format!("{p}{method}"),
        Mutation::Suffix(s) => format!("{method}{s}"),
        Mutation::DotToUnderscore => method.replace('.', "_"),
        Mutation::UnderscoreToDot => method.replacen('_', ".", 1),
        Mutation::Trim(n) => method
            .chars()
            .take(method.chars().count().saturating_sub(*n))
            .collect(),
        Mutation::Pad => format!(" {method} "),
    }
}

fn method_name() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => (prop::sample::select(known_methods()), mutation()).prop_map(|(m, edit)| apply(&m, &edit)),
        1 => any::<String>(),
        1 => "openhuman\\.[a-z_]{1,40}",
    ]
}

fn scope() -> impl Strategy<Value = Scope> {
    prop_oneof![Just(Scope::None), Just(Scope::Operator), Just(Scope::User)]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// With the operator-plane tag the registry would give the name, a user
    /// scope admits exactly `USER_METHODS`, the operator scope exactly the
    /// operator plane, and a missing scope nothing.
    #[test]
    fn each_saas_scope_admits_exactly_its_plane(method in method_name()) {
        let operator_plane = operator_methods().contains(&method);
        prop_assert_eq!(
            visible_in(true, Scope::User, &method, operator_plane),
            USER_METHODS.contains(&method.as_str()),
            "user scope, {:?}", method
        );
        prop_assert_eq!(visible_in(true, Scope::Operator, &method, operator_plane), operator_plane);
        prop_assert!(!visible_in(true, Scope::None, &method, operator_plane));
    }

    /// Whatever the registry tags, the two SaaS planes never overlap and a
    /// user never reaches a name off the allowlist.
    #[test]
    fn the_planes_never_overlap(method in method_name(), operator_plane in any::<bool>(), scope in scope()) {
        let user = visible_in(true, Scope::User, &method, operator_plane);
        let operator = visible_in(true, Scope::Operator, &method, operator_plane);
        prop_assert!(!(user && operator), "{:?} is on both planes", method);
        if user {
            prop_assert!(USER_METHODS.contains(&method.as_str()) && !operator_plane);
        }
        if visible_in(true, scope, &method, operator_plane) {
            prop_assert!(scope != Scope::None);
        }
    }
}

/// Characters that look like a reserved prefix's letters or colon.
fn lookalike() -> impl Strategy<Value = char> {
    prop::sample::select(vec![
        '\u{FF1A}', // fullwidth colon
        '\u{A789}', // modifier letter colon
        '\u{2236}', // ratio
        '\u{0441}', // Cyrillic es (c)
        '\u{0430}', // Cyrillic a
        '\u{043E}', // Cyrillic o
        '\u{0440}', // Cyrillic er (p)
        '\u{0435}', // Cyrillic ie (e)
        '\u{0456}', // Cyrillic i
        '\u{212A}', // Kelvin sign (folds to k)
        '\u{017F}', // long s (folds to s)
        '\u{0131}', // dotless i
    ])
}

/// A reserved prefix with case changes and lookalike substitutions.
fn reserved_variant() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec![
            "channel:",
            "proactive:",
            "subagent:",
            "cron:",
            "system:",
        ]),
        prop::collection::vec(any::<bool>(), 12),
        prop::collection::vec(prop::option::of(lookalike()), 12),
        "[A-Za-z0-9_-]{0,16}",
    )
        .prop_map(|(prefix, upper, swaps, rest)| {
            let head: String = prefix
                .chars()
                .enumerate()
                .map(|(i, c)| match swaps.get(i).copied().flatten() {
                    Some(look) if i % 3 == 0 || c == ':' => look,
                    _ if upper.get(i).copied().unwrap_or(false) => c.to_ascii_uppercase(),
                    _ => c,
                })
                .collect();
            format!("{head}{rest}")
        })
}

fn thread_id() -> impl Strategy<Value = String> {
    prop_oneof![
        reserved_variant(),
        "[A-Za-z0-9_-]{0,140}",
        any::<String>(),
        "[A-Za-z0-9_:./\\\\ -]{0,40}",
        prop::collection::vec(any::<char>(), 0..=200).prop_map(String::from_iter),
    ]
}

/// The documented thread-id charset: 1 to 128 ASCII letters, digits, `-`, `_`.
fn in_documented_charset(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// A user may mint exactly the documented charset; anything accepted
    /// carries no reserved prefix in any case and no colon of any script.
    #[test]
    fn user_thread_ids_are_exactly_the_charset(id in thread_id()) {
        let accepted = validate_user_thread_id(&id).is_ok();
        prop_assert_eq!(accepted, in_documented_charset(&id), "{:?}", id);
        if accepted {
            let folded = id.to_lowercase();
            for prefix in ["channel:", "proactive:", "subagent:", "cron:", "system:"] {
                prop_assert!(!folded.starts_with(prefix), "{:?}", id);
            }
            prop_assert!(id.is_ascii());
            let colons = [':', '\u{FF1A}', '\u{A789}', '\u{2236}'];
            prop_assert!(!id.contains(colons), "{:?}", id);
        }
    }

    /// Every reserved-prefix variant, however disguised, is refused.
    #[test]
    fn reserved_prefix_variants_are_refused(id in reserved_variant()) {
        prop_assert!(validate_user_thread_id(&id).is_err(), "{:?}", id);
    }
}
