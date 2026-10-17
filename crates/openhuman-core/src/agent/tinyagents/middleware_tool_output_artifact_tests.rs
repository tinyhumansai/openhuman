//! `ToolOutputMiddleware` and persisted tool-result artifacts (#6284): reads
//! of an artifact and what gets stored. Split from
//! `middleware_tool_output_tests.rs` to keep each file under the layout limit.

use super::*;
use crate::agent::tinyagents::middleware::tool_output::ToolOutputMiddleware;

fn artifact_mw(
    summarizer: Option<Arc<DynPayloadSummarizer>>,
    storage_dir: &std::path::Path,
) -> ToolOutputMiddleware {
    ToolOutputMiddleware {
        budget_bytes: 1_000,
        payload_summarizer: summarizer,
        artifact_store: Some(
            crate::agent::harness::tool_result_artifacts::in_memory_tool_result_store(
                storage_dir.to_path_buf(),
                "session",
            ),
        ),
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Full,
        runtime_config: Some(summarizing_config()),
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    }
}

fn summarized(summary: &str) -> Arc<DynPayloadSummarizer> {
    StubSummarizer::replying(Ok(summary.to_string()))
}

#[tokio::test]
async fn a_wrapped_read_of_a_persisted_artifact_is_paged_not_resummarized_or_repersisted() {
    let tmp = tempfile::tempdir().unwrap();
    let mw = artifact_mw(Some(summarized("SUMMARY")), tmp.path());
    // A relative pointer from a transcript written before the store was
    // detached still pages.
    let path = "artifacts/tool-results/session/use_skill/earlier.txt";
    // The read arrives wrapped, reported under the wrapper's name, exactly as
    // `use_skill` delivers `file_read`.
    let mut call = TaToolCall::new(
        "c1",
        "use_skill",
        json!({"skill": "files", "tool": "file_read", "args": {"path": path, "offset": 2_000}}),
    );
    let mut ctx = ctx();
    mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();
    let mut result = tool_result("use_skill", &"y".repeat(5_000));

    mw.after_tool(&mut ctx, &(), &invocation("c1", "use_skill"), &mut result)
        .await
        .unwrap();

    assert!(
        result_text(&result).starts_with("yyyy"),
        "an artifact read must return the stored body, not a summary of it: {:?}",
        &result_text(&result)[..result_text(&result).len().min(120)]
    );
    let rendered = result_text(&result);
    let page_end = rendered
        .find("\n\n[artifact page")
        .expect("an over-budget read must be paged with a continuation marker");
    assert!(
        rendered.contains(&format!("\"offset\":{}", 2_000 + page_end)),
        "the page must name the exact offset the next read starts at: {}",
        &rendered[page_end..]
    );
    assert!(
        !tmp.path().join("tool-results/session").exists(),
        "reading an artifact must not persist it again as a new artifact"
    );
}

/// The store's own pointer is a blob key; a read of it is recognised and paged
/// the same way, not summarized or stored again.
#[tokio::test]
async fn a_read_of_a_blob_key_pointer_is_paged_not_repersisted() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = tmp.path();
    let mw = artifact_mw(Some(summarized("SUMMARY")), storage);
    let pointer = "tool-results/session/shell/earlier.txt".to_string();
    let mut call = TaToolCall::new("c1", "file_read", json!({"path": pointer}));
    let mut ctx = ctx();
    mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();
    let mut result = tool_result("file_read", &"y".repeat(5_000));

    mw.after_tool(&mut ctx, &(), &invocation("c1", "file_read"), &mut result)
        .await
        .unwrap();

    let rendered = result_text(&result);
    assert!(
        rendered.starts_with("yyyy"),
        "{}",
        &rendered[..rendered.len().min(120)]
    );
    assert!(
        rendered.contains(&format!(
            "Continue with file_read {{\"path\":\"{pointer}\",\"offset\":"
        )),
        "the continuation must name the same blob key: {rendered}"
    );
    assert!(
        mw.artifact_store
            .as_ref()
            .unwrap()
            .read("tool-results/session/file_read/c1.txt")
            .await
            .unwrap()
            .is_none(),
        "reading an artifact must not persist it again as a new artifact"
    );
}

#[tokio::test]
async fn an_oversized_result_is_stored_as_the_tool_returned_it_not_as_rewritten() {
    let tmp = tempfile::tempdir().unwrap();
    // A summary still over the 1,000-byte budget, so the result is persisted
    // after an earlier stage has already rewritten it.
    let mw = artifact_mw(Some(summarized(&"s".repeat(3_000))), tmp.path());
    let raw = "r".repeat(8_000);
    let mut result = tool_result("echo", &raw);

    with_module(mw.after_tool(&mut ctx(), &(), &invocation("c1", "echo"), &mut result))
        .await
        .0
        .unwrap();

    assert!(
        result_text(&result).contains("[tool_result_preview]"),
        "an over-budget result is persisted: {}",
        result_text(&result)
    );
    let stored = mw
        .artifact_store
        .as_ref()
        .unwrap()
        .read("tool-results/session/echo/c1.txt")
        .await
        .unwrap()
        .expect("artifact written");
    assert_eq!(
        stored, raw,
        "the artifact must hold the tool's own output, not the rewritten copy"
    );
    assert!(
        result_text(&result).contains("original_bytes: 8000"),
        "the envelope must report the size of what was stored: {}",
        result_text(&result)
    );
}

#[tokio::test]
async fn a_raw_result_file_read_cannot_open_is_stored_as_the_processed_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let summary = "s".repeat(3_000);
    let raw_len = tinytools_std::filesystem::FileReadTool::MAX_FILE_SIZE_BYTES as usize + 1;
    let mw = artifact_mw(Some(summarized(&summary)), tmp.path());
    let mut result = tool_result("echo", &"r".repeat(raw_len));

    with_module(mw.after_tool(&mut ctx(), &(), &invocation("c1", "echo"), &mut result))
        .await
        .0
        .unwrap();

    let stored = mw
        .artifact_store
        .as_ref()
        .unwrap()
        .read("tool-results/session/echo/c1.txt")
        .await
        .unwrap()
        .expect("artifact written");
    assert_eq!(
        stored, summary,
        "a raw body over file_read's limit would be unreadable, so the processed copy is stored"
    );
}
