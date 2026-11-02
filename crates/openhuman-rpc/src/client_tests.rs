use super::redact_url_for_log;
use serde_json::Value;

#[test]
fn shared_characterization_corpus_tracks_rpc_url_redaction() {
    let corpus: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-extended-corpus.json"
    )))
    .unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        let input = case["input_parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .map(|part| part.as_str().unwrap())
                    .collect::<String>()
            })
            .unwrap_or_else(|| case["input"].as_str().unwrap().to_string());
        let needle = case["needle"].as_str().unwrap();
        let expected = case["removed_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sink| sink.as_str() == Some("rpc_url"));
        assert_eq!(
            !redact_url_for_log(&input).contains(needle),
            expected,
            "case={}",
            case["case"]
        );
    }
}
