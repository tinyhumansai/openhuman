//! Regression contract for bounded, no-egress source location.
use super::{rank_sources, SourceDocument};
use sha2::Digest;

fn src(path: &str, text: &str) -> SourceDocument {
    SourceDocument {
        path: path.to_owned(),
        source_hash: format!("{:x}", sha2::Sha256::digest(text.as_bytes())),
        content: text.to_owned(),
    }
}

#[test]
fn favors_relevant_symbols_and_paths_over_unrelated_long_documents() {
    let documents = vec![
        src("docs/readme.md", "User guide. Login is a process. The actual session code is elsewhere."),
        src("src/auth/session_manager.ts",
            "export function assertSessionFresh(session: Session) {\n  return session.expiresAt > Date.now();\n}\n"),
        src("src/billing/invoices.ts", "export function chargeInvoice() { return 1; }\n"),
    ];
    let found = rank_sources(&documents, "where is session freshness checked", 5);
    assert_eq!(found["results"][0]["path"], "src/auth/session_manager.ts");
    assert_eq!(found["results"][0]["evidence"][0]["line"], 1);
    assert_eq!(
        found["results"][0]["evidence"][0]["text"],
        "export function assertSessionFresh(session: Session) {"
    );
    assert_eq!(found["ranking_kind"], "local_lexical_heuristic");
    assert_eq!(found["source_of_truth_verified"], false);
    assert_eq!(found["results"][0]["source_hash"], documents[1].source_hash);
}

#[test]
fn token_boundaries_do_not_treat_partial_words_as_matches() {
    let docs = vec![
        src("src/superuser.ts", "const superuser = true;\n"),
        src("src/user.ts", "const user = true;\n"),
    ];
    let found = rank_sources(&docs, "user", 5);
    assert_eq!(found["results"].as_array().unwrap().len(), 1);
    assert_eq!(found["results"][0]["path"], "src/user.ts");
}

#[test]
fn path_and_camel_case_tokens_are_indexed_and_ties_are_deterministic() {
    let docs = vec![
        src("z/authSession.ts", "export const authSession = true;\n"),
        src("a/auth_session.ts", "export const authSession = true;\n"),
    ];
    let a = rank_sources(&docs, "auth session", 5);
    let b = rank_sources(
        &docs.iter().rev().cloned().collect::<Vec<_>>(),
        "auth session",
        5,
    );
    assert_eq!(a["results"], b["results"]);
    assert_eq!(a["results"][0]["path"], "a/auth_session.ts");
}

#[test]
fn missing_matches_are_declared_unknown_not_a_negative_finding() {
    let docs = vec![src("src/unrelated.rs", "fn something() {}\n")];
    let found = rank_sources(&docs, "payment settlement", 3);
    assert_eq!(found["results"], serde_json::json!([]));
    assert_eq!(found["no_match_is_exhaustive"], false);
    assert_eq!(found["source_of_truth_verified"], false);
}

#[test]
fn stopword_queries_are_rejected_without_fabricating_candidates() {
    let found = rank_sources(&[src("src/lib.rs", "fn main() {}\n")], "where is the", 5);
    assert_eq!(found["results"], serde_json::json!([]));
    assert_eq!(found["effective_query_terms"], 0);
}

#[test]
fn result_is_bounded_and_excerpt_is_valid_unicode() {
    let line = format!("fn reveal() {{ {} }}", "🙂".repeat(512));
    let docs: Vec<_> = (0..32)
        .map(|i| {
            src(
                &format!("src/session_{i:02}.rs"),
                &format!("{line}\n{line}\n{line}\n{line}\n"),
            )
        })
        .collect();
    let found = rank_sources(&docs, "reveal", 5);
    assert!(found.to_string().len() <= 26_000);
    assert!(found["results"].as_array().unwrap().len() <= 5);
    for item in found["results"].as_array().unwrap() {
        assert!(item["evidence"].as_array().unwrap().len() <= 3);
        for entry in item["evidence"].as_array().unwrap() {
            assert!(entry["text"].as_str().unwrap().chars().count() <= 160);
        }
    }
}

#[test]
fn limit_is_clamped_and_zero_cannot_be_unbounded() {
    let docs: Vec<_> = (0..32)
        .map(|i| {
            src(
                &format!("src/check_{i:02}.ts"),
                "export const check = true;\n",
            )
        })
        .collect();
    assert_eq!(
        rank_sources(&docs, "check", 0)["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        rank_sources(&docs, "check", 500)["results"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn location_excerpt_contains_the_actual_match_on_very_long_unicode_lines() {
    let prefix = "🙂".repeat(220);
    let line = format!("{prefix} authSessionFresh is checked here");
    let found = rank_sources(
        &[src("src/session.ts", &format!("{line}\n"))],
        "auth session fresh",
        5,
    );
    let excerpt = &found["results"][0]["evidence"][0];
    assert!(excerpt["text"]
        .as_str()
        .unwrap()
        .contains("authSessionFresh"));
    assert!(excerpt["column_start"].as_u64().unwrap() > 1);
    assert_eq!(excerpt["line"], 1);
    assert!(excerpt["text"].as_str().unwrap().chars().count() <= 160);
}

#[test]
fn bounded_query_keeps_late_specific_symbols_instead_of_alphabetical_prefix() {
    let query = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform zuluCritical";
    let docs = vec![
        src("src/generic.rs", "fn totally_unrelated() {}\n"),
        src("src/target.rs", "fn zuluCritical() {}\n"),
    ];
    let found = rank_sources(&docs, query, 5);
    assert_eq!(found["effective_query_terms"], 16);
    assert_eq!(
        found["results"][0]["path"], "src/target.rs",
        "late user constraints must not disappear under sorted-set truncation"
    );
    assert!(found["results"][0]["matched_terms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "zulu" || value == "critical"));
}

#[test]
fn long_source_line_excerpt_includes_the_actual_matching_symbol() {
    let text = format!("{}criticalFunction() {}\n", "x".repeat(220), "y".repeat(60));
    let docs = vec![src("src/long.ts", &text)];
    let result = rank_sources(&docs, "critical function", 5);
    let excerpt = &result["results"][0]["evidence"][0];
    assert_eq!(excerpt["line"], 1);
    assert!(
        excerpt["text"]
            .as_str()
            .unwrap()
            .contains("criticalFunction"),
        "a line hit should show its matching token, not an unrelated prefix"
    );
    assert!(excerpt["column_start"].as_u64().unwrap() > 1);
    assert_eq!(excerpt["excerpt_truncated"], true);
}

#[test]
fn source_excerpt_never_splits_unicode_code_points() {
    let text = format!("{}unicodeMarker();\n", "🧠".repeat(220));
    let docs = vec![src("src/unicode.ts", &text)];
    let result = rank_sources(&docs, "unicode marker", 5);
    let excerpt = result["results"][0]["evidence"][0]["text"]
        .as_str()
        .unwrap();
    assert!(excerpt.contains("unicodeMarker"));
    assert!(excerpt.chars().count() <= 160);
    assert!(!excerpt.contains('�'));
}

#[test]
fn first_column_excerpt_keeps_original_text_and_provenance() {
    let docs = vec![src("src/api.rs", "fn loadAccount() {}\n")];
    let result = rank_sources(&docs, "load account", 5);
    let evidence = &result["results"][0]["evidence"][0];
    assert_eq!(evidence["text"], "fn loadAccount() {}");
    assert_eq!(evidence["column_start"], 1);
    assert_eq!(evidence["excerpt_truncated"], false);
}

#[test]
fn long_line_excerpt_includes_matched_symbol_not_an_unrelated_prefix() {
    let line = format!("{} sessionVerifier() {}\n", "noisy ".repeat(180), "{}");
    let docs = vec![src("src/long.rs", &line)];
    let found = rank_sources(&docs, "session verifier", 5);
    assert_eq!(found["results"].as_array().unwrap().len(), 1);
    let sample = &found["results"][0]["evidence"][0];
    let snippet = sample["text"].as_str().unwrap();
    assert!(
        snippet.contains("sessionVerifier"),
        "locator must display matching evidence: {snippet}"
    );
    assert!(sample["column_start"].as_u64().unwrap() > 1);
    assert!(snippet.chars().count() <= 160);
}

#[test]
fn unicode_excerpts_count_columns_in_characters_not_utf8_bytes() {
    let line = format!("{} sessionVerifier()\n", "🙂 ".repeat(150));
    let docs = vec![src("src/unicode.rs", &line)];
    let found = rank_sources(&docs, "session verifier", 5);
    let sample = &found["results"][0]["evidence"][0];
    assert!(sample["text"].as_str().unwrap().contains("sessionVerifier"));
    assert!(sample["column_start"].as_u64().unwrap() > 1);
    assert_eq!(sample["line"], 1);
}
