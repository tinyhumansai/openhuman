use super::*;
use crate::security::SecurityPolicy;
use serde_json::json;
use tinytools::Tool;

fn fetch() -> WebFetchTool {
    web_fetch_tool(
        Arc::new(SecurityPolicy::default()),
        vec!["example.com".into()],
        None,
        None,
    )
}

#[test]
fn web_fetch_opts_into_an_optional_summary_focus() {
    let schema = fetch().parameters_schema();
    assert_eq!(schema["properties"][SUMMARY_FOCUS_ARG]["type"], "string");
    assert!(!schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!(SUMMARY_FOCUS_ARG)));
    // The tool's schema declares TinyJuice's own property, not a lookalike.
    assert!(crate::inference::tokenjuice::focus::declares_summary_focus(
        &schema
    ));
}

/// The exact wire contract `web_fetch` had before it moved: name, description,
/// permission, exposure and the schema including `summary_focus`.
#[test]
fn web_fetch_advertises_its_pre_move_contract() {
    let tool = fetch();
    let described = json!({
        "name": tool.name(),
        "description": tool.description(),
        "permission_level": format!("{:?}", tool.permission_level()),
        "exposure": format!("{:?}", tool.exposure()),
        "schema": tool.parameters_schema(),
    });
    let expected: serde_json::Value = serde_json::from_str(
        r#"{
  "description": "GET a URL and read the page. HTML returns as Markdown (links kept, scripts dropped); `raw: true` for the body as sent. For POST or custom headers use `http_request`.",
  "exposure": "Direct",
  "name": "web_fetch",
  "permission_level": "ReadOnly",
  "schema": {
    "properties": {
      "max_bytes": {
        "description": "Cap the returned text at this many bytes (default 1_000_000). Bounds the OUTPUT — the extracted markdown, or the raw body with raw:true — never the markup the extractor reads, so lowering it cannot cost you content the page actually had.",
        "minimum": 1,
        "type": "integer"
      },
      "raw": {
        "description": "Return the body as sent.",
        "type": "boolean"
      },
      "summary_focus": {
        "description": "What you need from this result. A large result is summarized around this; the full output stays retrievable.",
        "type": "string"
      },
      "url": {
        "description": "Absolute http(s) URL.",
        "type": "string"
      }
    },
    "required": [
      "url"
    ],
    "type": "object"
  }
}"#,
    )
    .unwrap();
    assert_eq!(described, expected);
}

#[test]
fn zero_limits_fall_back_to_the_http_request_config_defaults() {
    let defaults = HttpRequestConfig::default();
    let limits = http_limits();
    assert_eq!(limits.max_response_size, defaults.max_response_size);
    assert_eq!(limits.timeout_secs, defaults.timeout_secs);
    assert_ne!(limits.max_response_size, 0);
    assert_ne!(limits.timeout_secs, 0);
}

#[tokio::test]
async fn tinyjuice_detects_and_converts_html() {
    if std::env::var_os("TINYJUICE_TEST_MODULE").is_none() {
        eprintln!("SKIPPED: released module fixture is not configured");
        return;
    }
    let page = "<!DOCTYPE html><html><body><p>hi</p></body></html>";
    assert!(TinyJuiceHtml.looks_like_html(page).await.unwrap());
    assert!(!TinyJuiceHtml
        .looks_like_html("# Just a README\n\nSome prose.\n")
        .await
        .unwrap());
    assert!(TinyJuiceHtml
        .to_markdown(page)
        .await
        .unwrap()
        .contains("hi"));
}
