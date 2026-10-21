use super::canonical_segment;

#[test]
fn valid_underscores_are_preserved() {
    assert_eq!(canonical_segment("server_name"), "server_name");
    assert_eq!(canonical_segment("server-name_2"), "server-name_2");
}

#[test]
fn invalid_names_use_the_reserved_escape_namespace() {
    assert_eq!(
        canonical_segment("Server A/α"),
        "__x_i_53657276657220412fceb1"
    );
    assert_eq!(canonical_segment(""), "__x_i_");
    assert_eq!(canonical_segment("__x"), "__x_v_5f5f78");
}

#[test]
fn literal_escape_looking_names_do_not_collide_with_encoded_names() {
    let encoded = canonical_segment("Server");
    let literal = canonical_segment("__x_i_536572766572");

    assert_eq!(encoded, "__x_i_536572766572");
    assert!(literal.starts_with("__x_v_"));
    assert_ne!(encoded, literal);
}

#[test]
fn distinct_configured_names_remain_distinct_after_encoding() {
    use std::collections::HashSet;

    let names = [
        "server_name",
        "Server_name",
        "server name",
        "server/name",
        "",
        "__x_i_736572766572",
    ];
    let ids = names
        .iter()
        .map(|name| canonical_segment(name))
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), names.len());
    assert!(ids.iter().all(|id| {
        id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    }));
}
