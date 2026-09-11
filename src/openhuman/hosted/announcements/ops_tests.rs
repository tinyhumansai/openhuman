use super::*;

#[test]
fn announcement_not_found_error_is_detected() {
    let err = anyhow::Error::new(BackendApiError::AnnouncementNotFound);
    assert!(is_announcement_not_found(&err));
}

#[test]
fn other_backend_errors_are_not_announcement_not_found() {
    let err = anyhow::Error::new(BackendApiError::Unauthorized {
        method: "GET".to_string(),
        path: "/announcements/latest".to_string(),
    });
    assert!(!is_announcement_not_found(&err));

    let plain = anyhow::anyhow!("GET /announcements/latest failed (500): boom");
    assert!(!is_announcement_not_found(&plain));
}
