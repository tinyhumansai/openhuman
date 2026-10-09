use super::*;

fn get(uri: &str) -> Response<Vec<u8>> {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Vec::new())
        .unwrap();
    response_for(&request)
}

fn csp_of(response: &Response<Vec<u8>>) -> String {
    response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn proxy_denies_connections_by_default() {
    let response = get("ohwidget://localhost/proxy");
    assert_eq!(response.status(), StatusCode::OK);
    let csp = csp_of(&response);
    assert!(csp.contains("connect-src 'none'"));
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("form-action 'none'"));
    assert!(String::from_utf8(response.body().clone())
        .unwrap()
        .contains("sandbox-proxy-ready"));
}

#[test]
fn proxy_opens_only_declared_https_origins() {
    let response = get(
        "http://ohwidget.localhost/proxy?connect=https%3A%2F%2Fapi.example.com%2Fv1&connect=http%3A%2F%2Finsecure.example.com&resource=https%3A%2F%2Fcdn.example.com&resource=javascript%3Aalert(1)",
    );
    let csp = csp_of(&response);
    assert!(csp.contains("connect-src https://api.example.com"));
    assert!(!csp.contains("insecure.example.com"));
    assert!(csp.contains("script-src 'unsafe-inline' https://cdn.example.com"));
    assert!(!csp.contains("javascript"));
}

#[test]
fn unknown_paths_and_methods_are_refused() {
    assert_eq!(
        get("ohwidget://localhost/other").status(),
        StatusCode::NOT_FOUND
    );
    let request = Request::builder()
        .method("POST")
        .uri("ohwidget://localhost/proxy")
        .body(Vec::new())
        .unwrap();
    assert_eq!(
        response_for(&request).status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[test]
fn csp_semicolons_cannot_be_injected() {
    let response =
        get("ohwidget://localhost/proxy?connect=https%3A%2F%2Fa.example.com%3B%20script-src%20*");
    let csp = csp_of(&response);
    assert!(!csp.contains("script-src *"));
}

#[test]
fn widget_frame_cannot_be_navigated_to_a_remote_page() {
    let csp = csp_of(&get(
        "ohwidget://localhost/proxy?connect=https%3A%2F%2Fapi.example.com&resource=https%3A%2F%2Fcdn.example.com",
    ));
    let frame_src = csp
        .split("; ")
        .find(|directive| directive.starts_with("frame-src"))
        .unwrap();
    assert_eq!(frame_src, "frame-src about:");
}

#[test]
fn proxy_sandbox_grants_no_navigation_or_popups() {
    let sandbox = PROXY_HTML
        .split("setAttribute(\"sandbox\", \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap();
    assert_eq!(sandbox, "allow-scripts allow-forms");
    assert!(!PROXY_HTML.contains("allow-same-origin"));
    assert!(!PROXY_HTML.contains("allow-top-navigation"));
    assert!(!PROXY_HTML.contains("allow-popups"));
}
