use super::client::{redact_token, CanvasEndpoint, CanvasRequestPolicy};

#[test]
fn request_policy_builds_only_allowed_canvas_api_urls() {
    let policy = CanvasRequestPolicy::new("https://mango-cmu.instructure.com").unwrap();

    assert_eq!(
        policy.url_for(CanvasEndpoint::Courses).unwrap().as_str(),
        "https://mango-cmu.instructure.com/api/v1/courses"
    );
    assert_eq!(
        policy
            .url_for(CanvasEndpoint::PlannerItems {
                context_codes: vec!["course_101".to_string(), "course_202".to_string()],
            })
            .unwrap()
            .as_str(),
        "https://mango-cmu.instructure.com/api/v1/planner/items?context_codes%5B%5D=course_101&context_codes%5B%5D=course_202"
    );
    assert_eq!(
        policy
            .url_for(CanvasEndpoint::Assignments {
                course_id: "101".to_string(),
            })
            .unwrap()
            .as_str(),
        "https://mango-cmu.instructure.com/api/v1/courses/101/assignments?include%5B%5D=submission"
    );
    assert_eq!(
        policy
            .url_for(CanvasEndpoint::Assignment {
                course_id: "101".to_string(),
                assignment_id: "55".to_string(),
            })
            .unwrap()
            .as_str(),
        "https://mango-cmu.instructure.com/api/v1/courses/101/assignments/55?include%5B%5D=submission"
    );
}

#[test]
fn request_policy_rejects_non_https_hosts() {
    let err = CanvasRequestPolicy::new("http://mango-cmu.instructure.com").unwrap_err();

    assert!(err.to_string().contains("HTTPS"));
}

#[test]
fn request_policy_rejects_urls_outside_allowed_host() {
    let policy = CanvasRequestPolicy::new("https://mango-cmu.instructure.com").unwrap();

    let err = policy
        .validate_url("https://evil.example/api/v1/courses")
        .unwrap_err();

    assert!(err.to_string().contains("configured Canvas host"));
}

#[test]
fn request_policy_rejects_disallowed_endpoint_families() {
    let policy = CanvasRequestPolicy::new("https://mango-cmu.instructure.com").unwrap();

    for path in [
        "https://mango-cmu.instructure.com/api/v1/users/self",
        "https://mango-cmu.instructure.com/api/v1/courses/101/students/submissions",
        "https://mango-cmu.instructure.com/api/v1/courses/101/assignments/55/submissions",
    ] {
        assert!(
            policy.validate_url(path).is_err(),
            "expected {path} to be rejected"
        );
    }
}

#[test]
fn request_policy_rejects_empty_or_traversal_ids() {
    let policy = CanvasRequestPolicy::new("https://mango-cmu.instructure.com").unwrap();

    assert!(policy
        .url_for(CanvasEndpoint::Assignments {
            course_id: String::new(),
        })
        .is_err());
    assert!(policy
        .url_for(CanvasEndpoint::Assignment {
            course_id: "../101".to_string(),
            assignment_id: "55".to_string(),
        })
        .is_err());
}

#[test]
fn redact_token_removes_secret_from_error_text() {
    let redacted = redact_token("bad secret", "secret");

    assert_eq!(redacted, "bad [REDACTED]");
    assert!(!redacted.contains("secret"));
}
