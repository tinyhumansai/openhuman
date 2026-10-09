use super::*;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn confine_path_rules() {
    let root = tempdir();
    let outside = tempdir();
    std::fs::write(root.path().join("view.html"), "<p>hi</p>").unwrap();
    std::fs::write(root.path().join("notes.txt"), "x").unwrap();
    std::fs::write(outside.path().join("secret.html"), "x").unwrap();
    let roots = vec![root.path().to_path_buf()];

    assert!(confine_path(&roots, "view.html").is_ok());
    assert!(confine_path(&roots, "notes.txt").is_err());
    assert!(confine_path(&roots, "../secret.html").is_err());
    assert!(confine_path(&roots, outside.path().join("secret.html").to_str().unwrap()).is_err());
    assert!(confine_path(&roots, "missing.html").is_err());
    assert!(confine_path(&roots, "").is_err());
}

#[cfg(unix)]
#[test]
fn confine_path_refuses_symlink_escape() {
    let root = tempdir();
    let outside = tempdir();
    std::fs::write(outside.path().join("secret.html"), "x").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.html"),
        root.path().join("link.html"),
    )
    .unwrap();
    assert!(confine_path(&[root.path().to_path_buf()], "link.html").is_err());
}

#[tokio::test]
async fn inline_html_produces_host_inline_presentation() {
    let tool = ShowUiTool::new(vec![]);
    let result = tool
        .execute(
            serde_json::json!({"html": "<p>chart</p>", "title": "Sales\nchart", "data": {"n": 1}}),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    let metadata = result.metadata.unwrap();
    assert_eq!(metadata["kind"], "mcp_ui");
    assert_eq!(metadata["flavor"], "host_inline");
    assert_eq!(metadata["title"], "Saleschart");
    assert!(metadata.get("server_id").is_none());
    let id = metadata["inline_id"].as_str().unwrap();
    let entry = cache::get_inline(id).unwrap();
    assert_eq!(entry.resource.html, "<p>chart</p>");
    assert!(entry.server_id.is_none());
}

#[tokio::test]
async fn requires_exactly_one_source() {
    let tool = ShowUiTool::new(vec![]);
    let both = tool
        .execute(serde_json::json!({"html": "<p/>", "path": "a.html"}))
        .await
        .unwrap();
    assert!(both.is_error);
    let neither = tool.execute(serde_json::json!({})).await.unwrap();
    assert!(neither.is_error);
}

#[tokio::test]
async fn path_source_is_read_from_root() {
    let root = tempdir();
    std::fs::write(root.path().join("v.html"), "<b>ok</b>").unwrap();
    let tool = ShowUiTool::new(vec![root.path().to_path_buf()]);
    let result = tool
        .execute(serde_json::json!({"path": "v.html"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    let id = result.metadata.unwrap()["inline_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(cache::get_inline(&id).unwrap().resource.html, "<b>ok</b>");
}
