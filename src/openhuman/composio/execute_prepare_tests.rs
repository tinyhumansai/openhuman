use serde_json::json;

use super::prepare_execute_arguments;

#[test]
fn calendar_bare_date_becomes_rfc3339() {
    let args = json!({
        "timeMin": "2026-05-14",
        "timeMax": "2026-05-15"
    });
    let prepared = prepare_execute_arguments("GOOGLECALENDAR_EVENTS_LIST", Some(args)).unwrap();
    assert_eq!(prepared["timeMin"], "2026-05-14T00:00:00Z");
    assert_eq!(prepared["timeMax"], "2026-05-15T00:00:00Z");
}

#[test]
fn notion_fetch_data_infers_fetch_type_from_filter() {
    let args = json!({
        "filter": { "value": "page", "property": "object" },
        "page_size": 25
    });
    let prepared = prepare_execute_arguments("NOTION_FETCH_DATA", Some(args)).unwrap();
    assert_eq!(prepared["fetch_type"], "pages");
}

#[test]
fn gmail_send_requires_recipient() {
    let err = prepare_execute_arguments("GMAIL_SEND_EMAIL", Some(json!({ "subject": "hi" })))
        .unwrap_err();
    assert!(err.contains("recipient") || err.contains("`to`"));
}

#[test]
fn gmail_add_label_requires_label_ids() {
    let err = prepare_execute_arguments(
        "GMAIL_ADD_LABEL_TO_EMAIL",
        Some(json!({ "message_id": "m1", "add_label_ids": [], "remove_label_ids": [] })),
    )
    .unwrap_err();
    assert!(err.contains("add_label_ids") || err.contains("remove_label_ids"));
}
