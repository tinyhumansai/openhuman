use super::*;

#[test]
fn ready_line_parses() {
    let ready: ReadyLine =
        serde_json::from_str(r#"{"ready":true,"protocol":1,"backends":["spacy"]}"#).unwrap();
    assert!(ready.ready);
    assert_eq!(ready.protocol, Some(PROTOCOL_VERSION));
    assert_eq!(ready.backends, vec!["spacy"]);
}

#[test]
fn response_parses_error_envelope() {
    let response: PythonServerResponse = serde_json::from_str(
        r#"{"id":"7","ok":false,"error":{"code":"bad_request","message":"missing text"}}"#,
    )
    .unwrap();
    assert!(!response.ok);
    assert_eq!(response.id.as_deref(), Some("7"));
    assert_eq!(response.error.unwrap().code, "bad_request");
}
