use super::*;

#[test]
fn content_hash_is_stable_under_whitespace_and_case() {
    let a = content_hash("I prefer Postgres for new services.");
    let b = content_hash("  i PREFER  postgres   for new services.  ");
    assert_eq!(a, b);
}

#[test]
fn content_hash_differs_for_different_text() {
    let a = content_hash("I prefer Postgres for new services.");
    let b = content_hash("I prefer SQLite for new services.");
    assert_ne!(a, b);
}
