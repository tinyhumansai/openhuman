use super::*;
#[tokio::test]
async fn toc_is_absent_when_no_result_was_offloaded() {
    let mw = ArtifactIndexTocMiddleware::new(0);
    let mut ctx = ctx_with_artifacts(&[]).await;
    let mut request = ModelRequest {
        messages: vec![TaMessage::user("hi")],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    assert_eq!(request.messages.len(), 1, "no artifacts, no contents list");
}

/// The paths reach the model even though nothing in the transcript carries
/// them — which is the whole point: the pointer's old home (the tool body) is
/// what the reduction steps act on.
#[tokio::test]
async fn toc_lists_every_persisted_artifact_as_a_system_message() {
    let mw = ArtifactIndexTocMiddleware::new(0);
    let mut ctx = ctx_with_artifacts(&[
        ("call-1", "fetch_issues", "outputs/issues-p1.json", 240_000),
        ("call-2", "web_search", "outputs/search-2.json", 91_000),
    ])
    .await;
    // A transcript that has already lost both pointers: one body blanked by
    // microcompact, the other never present.
    let mut request = ModelRequest {
        messages: vec![
            TaMessage::user("what did you find?"),
            TaMessage::tool("call-1", CLEARED_PLACEHOLDER),
        ],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let last = request.messages.last().expect("a contents list");
    assert!(
        matches!(last, TaMessage::System(_)),
        "must be a system message — compression keeps those verbatim and the trim never drops \
         one, so the pointer cannot be reduced away: {last:?}"
    );
    let text = last.text();
    assert!(
        text.contains("outputs/issues-p1.json"),
        "missing path: {text}"
    );
    assert!(
        text.contains("outputs/search-2.json"),
        "missing path: {text}"
    );
    assert!(text.contains("fetch_issues"), "missing tool name: {text}");
}

/// Rendered fresh per request, so repeated passes cannot stack stale lists.
#[tokio::test]
async fn toc_does_not_accumulate_across_calls() {
    let mw = ArtifactIndexTocMiddleware::new(0);
    let mut ctx = ctx_with_artifacts(&[("call-1", "fetch", "outputs/a.json", 100)]).await;

    let mut first = ModelRequest {
        messages: vec![TaMessage::user("hi")],
        ..Default::default()
    };
    mw.before_model(&mut ctx, &(), &mut first).await.unwrap();

    // The loop rebuilds the request from its own transcript each iteration, so
    // the second call starts from the same base — not from the mutated first.
    let mut second = ModelRequest {
        messages: vec![TaMessage::user("hi")],
        ..Default::default()
    };
    mw.before_model(&mut ctx, &(), &mut second).await.unwrap();

    assert_eq!(first.messages.len(), 2);
    assert_eq!(
        second.messages.len(),
        2,
        "exactly one contents list per request"
    );
}

// ── the bounds CodeRabbit asked for on #6068 ────────────────────────────────

/// Restoration stops at the input allowance instead of overflowing it.
///
/// It runs after compression and before the trim, so an unbounded restore
/// pushes the request over and the trim evicts whole messages — strictly more
/// destructive than the blanking being undone, and able to discard the very
/// results just restored. Preventing the overflow beats repairing it.
#[tokio::test]
async fn wrap_up_stops_restoring_at_the_input_budget() {
    let big = "x".repeat(4_000);
    let mw = FinalCallWrapUpMiddleware::new(
        "CONCLUDE NOW",
        sink_with(&[("call-1", &big), ("call-2", &big), ("call-3", &big)]),
        // Room for roughly one of them, not three.
        1_200,
    );
    let mut ctx = RunContext::new(
        RunConfig::new("mw-test").with_max_model_calls(2),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    ctx.limits.record_model_call().unwrap();
    ctx.limits.record_model_call().unwrap();
    let mut request = ModelRequest {
        messages: vec![
            TaMessage::tool("call-1", CLEARED_PLACEHOLDER),
            TaMessage::tool("call-2", CLEARED_PLACEHOLDER),
            TaMessage::tool("call-3", CLEARED_PLACEHOLDER),
        ],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let restored = request
        .messages
        .iter()
        .filter(|m| m.text().starts_with("xxx"))
        .count();
    assert!(
        restored >= 1 && restored < 3,
        "some restored, not all — budget 1200 cannot hold three 4k bodies, got {restored}"
    );
    // Newest-first: the last tool result is the one the model never saw (the
    // cap is checked before the request is built). Not `messages.last()` —
    // that is the wrap-up instruction this middleware just appended.
    let newest_tool = request
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m, TaMessage::Tool(_)))
        .map(|m| m.text())
        .unwrap_or_default();
    assert!(
        newest_tool.starts_with("xxx"),
        "the newest cleared result is restored first, got: {}",
        &newest_tool[..newest_tool.len().min(60)]
    );
}

/// The contents list is capped, and says how many it left out.
///
/// It is a system message so the ladder cannot take it — which also means
/// nothing downstream can shrink it, so it has to bound itself.
#[tokio::test]
async fn toc_is_capped_and_reports_what_it_omitted() {
    let entries: Vec<(String, String, String, u64)> = (0..200)
        .map(|i| {
            (
                format!("call-{i}"),
                format!("tool_with_a_long_name_{i}"),
                format!("outputs/a-fairly-long-artifact-path-{i}.json"),
                1_000,
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, &str, u64)> = entries
        .iter()
        .map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), *d))
        .collect();
    let mut ctx = ctx_with_artifacts(&borrowed).await;
    // This middleware's share of the allowance, already split at the install
    // site — not the whole turn allowance.
    let mw = ArtifactIndexTocMiddleware::new(200);
    let mut request = ModelRequest {
        messages: vec![TaMessage::user("what did you find?")],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let text = request.messages.last().expect("a contents list").text();
    assert!(
        text.contains("not listed here"),
        "an omitted count must be disclosed, not silently dropped: {text}"
    );
    // The nominal share, not a loose multiple of it: the cap now seeds `used`
    // with the header and the reserved footer, so it bounds the whole message
    // rather than the rows alone (CodeRabbit on #6068).
    assert!(
        estimate_text_tokens(&text) <= 200,
        "the list must stay inside the share it was given: {}",
        estimate_text_tokens(&text)
    );
}

/// Neither share may ever be `0`, because `0` is the sentinel both middlewares
/// read as "unbounded" — so the allowances that round down into it are the ones
/// to pin (CodeRabbit on #6068, twice). A model advertising no window is the
/// sharp case: no window means no `ImageAwareMessageTrimMiddleware` either, so
/// these two additions are the ones nothing downstream would shrink.
#[test]
fn neither_share_is_ever_the_unbounded_sentinel() {
    for trim_allowance in [0_u64, 1, 5, 9, 99, 100, 2_000, 100_000] {
        let (toc, restore) = split_input_allowance(trim_allowance);
        assert!(toc > 0, "allowance {trim_allowance} disabled the TOC cap");
        assert!(
            restore > 0,
            "allowance {trim_allowance} left restoration unbounded"
        );
        // The same effective total the function computes — not
        // `trim_allowance.max(NO_WINDOW_ALLOWANCE)`, which accepted any
        // over-split below 5_120 (and whose `.min(u64::MAX)` was a no-op).
        let total = if trim_allowance == 0 {
            NO_WINDOW_ALLOWANCE
        } else {
            trim_allowance
        };
        // `1` stays excluded: the function deliberately keeps both shares
        // positive, which costs one token at that degenerate size.
        if trim_allowance > 1 {
            assert!(
                toc + restore <= total,
                "allowance {trim_allowance} split into more than it had: {toc} + {restore}"
            );
        }
    }
    // With no window the split is taken from the fallback total, not from zero.
    let (toc, restore) = split_input_allowance(0);
    assert_eq!(toc, 512);
    assert_eq!(toc + restore, NO_WINDOW_ALLOWANCE);
    // Proportional sizing is untouched where there is something to divide.
    assert_eq!(split_input_allowance(2_000), (200, 1_800));
    assert_eq!(split_input_allowance(100_000), (10_000, 90_000));
}

/// The end of the same argument: with no window, a run holding far more
/// artifacts than any list should carry still produces a bounded message.
#[tokio::test]
async fn the_contents_list_stays_bounded_when_no_window_is_advertised() {
    let entries: Vec<(String, String, String, u64)> = (0..400)
        .map(|i| {
            (
                format!("call-{i}"),
                "file_read".to_string(),
                format!("outputs/a-fairly-long-artifact-path-{i}.json"),
                1_000,
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, &str, u64)> = entries
        .iter()
        .map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), *d))
        .collect();
    let mut ctx = ctx_with_artifacts(&borrowed).await;
    let mw = ArtifactIndexTocMiddleware::new(split_input_allowance(0).0);
    let mut request = ModelRequest {
        messages: vec![TaMessage::user("what did you find?")],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let text = request.messages.last().expect("a contents list").text();
    assert!(
        text.contains("not listed here"),
        "an omitted count must be disclosed, not silently dropped: {text}"
    );
    assert!(
        estimate_text_tokens(&text) <= split_input_allowance(0).0,
        "the no-window fallback must bound the whole list message: {}",
        estimate_text_tokens(&text)
    );
}

/// The other half of the no-window split: restoration must stop too. Without a
/// window there is no trim middleware behind it, so an unbounded wrap-up would
/// hand the provider a request it rejects — losing the in-loop conclusion the
/// whole cap-checkpoint path exists to deliver (CodeRabbit on #6068).
#[tokio::test]
async fn wrap_up_restoration_stays_bounded_when_no_window_is_advertised() {
    let big = "x".repeat(8_000);
    let entries: Vec<(String, String)> = (0..40)
        .map(|i| (format!("call-{i}"), big.clone()))
        .collect();
    let borrowed: Vec<(&str, &str)> = entries
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_toc, restore) = split_input_allowance(0);
    let mw = FinalCallWrapUpMiddleware::new("CONCLUDE NOW", sink_with(&borrowed), restore);
    let mut ctx = RunContext::new(
        RunConfig::new("mw-test").with_max_model_calls(2),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    ctx.limits.record_model_call().unwrap();
    ctx.limits.record_model_call().unwrap();
    let mut request = ModelRequest {
        messages: borrowed
            .iter()
            .map(|(id, _)| TaMessage::tool(*id, CLEARED_PLACEHOLDER))
            .collect(),
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let used: u64 = request.messages.iter().map(estimate_message_tokens).sum();
    assert!(
        used <= NO_WINDOW_ALLOWANCE,
        "the no-window fallback must bound the whole request, not just the list: {used}"
    );
    let restored = request
        .messages
        .iter()
        .filter(|m| m.text().starts_with("xxx"))
        .count();
    assert!(
        restored >= 1 && restored < 40,
        "some restored, not all 40 — got {restored}"
    );
}

/// The two consumers share one allowance, so the bound that matters is the one
/// on the request they *both* wrote to (CodeRabbit on #6068). Runs them in
/// registration order — wrap-up, then contents list — on the no-window path,
/// where nothing downstream would trim what they overshoot by.
#[tokio::test]
async fn both_middlewares_together_stay_inside_the_no_window_allowance() {
    let big = "x".repeat(8_000);
    let outcomes: Vec<(String, String)> = (0..40)
        .map(|i| (format!("call-{i}"), big.clone()))
        .collect();
    let borrowed: Vec<(&str, &str)> = outcomes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let artifacts: Vec<(String, String, String, u64)> = (0..400)
        .map(|i| {
            (
                format!("call-{i}"),
                "file_read".to_string(),
                format!("outputs/a-fairly-long-artifact-path-{i}.json"),
                1_000,
            )
        })
        .collect();
    let artifact_refs: Vec<(&str, &str, &str, u64)> = artifacts
        .iter()
        .map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), *d))
        .collect();

    let mut ctx = ctx_with_artifacts_and_config(
        &artifact_refs,
        RunConfig::new("mw-test").with_max_model_calls(2),
    )
    .await;
    ctx.limits.record_model_call().unwrap();
    ctx.limits.record_model_call().unwrap();

    let (toc_allowance, restore_allowance) = split_input_allowance(0);
    let mut request = ModelRequest {
        messages: borrowed
            .iter()
            .map(|(id, _)| TaMessage::tool(*id, CLEARED_PLACEHOLDER))
            .collect(),
        ..Default::default()
    };

    // Registration order at the install site: wrap-up first, contents list after.
    FinalCallWrapUpMiddleware::new("CONCLUDE NOW", sink_with(&borrowed), restore_allowance)
        .before_model(&mut ctx, &(), &mut request)
        .await
        .unwrap();
    ArtifactIndexTocMiddleware::new(toc_allowance)
        .before_model(&mut ctx, &(), &mut request)
        .await
        .unwrap();

    let used: u64 = request.messages.iter().map(estimate_message_tokens).sum();
    assert!(
        used <= NO_WINDOW_ALLOWANCE,
        "the combined request must stay inside the shared allowance: {used} > {NO_WINDOW_ALLOWANCE}"
    );
    // Both actually contributed — otherwise the bound is met vacuously.
    assert!(
        request.messages.iter().any(|m| m.text().starts_with("xxx")),
        "the wrap-up restored nothing, so the bound proves nothing"
    );
    assert!(
        request
            .messages
            .iter()
            .any(|m| m.text().contains("Stored results from this turn")),
        "the contents list is absent, so the bound proves nothing"
    );
}

/// A share smaller than the fixed text renders no rows at all. Forcing one
/// would push a message nothing downstream can shrink over its bound, and the
/// header plus the omitted count already disclose that the results exist
/// (CodeRabbit on #6068).
#[tokio::test]
async fn a_share_smaller_than_the_header_renders_no_rows() {
    let entries: Vec<(String, String, String, u64)> = (0..10)
        .map(|i| {
            (
                format!("call-{i}"),
                "file_read".to_string(),
                format!("outputs/artifact-{i}.json"),
                1_000,
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, &str, u64)> = entries
        .iter()
        .map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), *d))
        .collect();
    let mut ctx = ctx_with_artifacts(&borrowed).await;
    // The floor a small window gets — below the header's own cost.
    let mw = ArtifactIndexTocMiddleware::new(64);
    let mut request = ModelRequest {
        messages: vec![TaMessage::user("what did you find?")],
        ..Default::default()
    };

    mw.before_model(&mut ctx, &(), &mut request).await.unwrap();

    let text = request.messages.last().expect("a contents list").text();
    assert!(
        !text.contains("file_read"),
        "no row fits the share, so none may be forced: {text}"
    );
    assert!(
        text.contains("10 tool result(s) were written to disk"),
        "the count must still be named so the results stay findable: {text}"
    );
    // The bound that matters: the whole rendered message inside the share.
    // Truncating to zero rows was not enough — the header and footer alone are
    // ~118 tokens against this 64-token floor, and the earlier form of this
    // assertion (`< 64 + FOOTER_ALLOWANCE + 64`) was loose enough to accept
    // exactly that overshoot (CodeRabbit on #6068).
    assert!(
        estimate_text_tokens(&text) <= 64,
        "the message must fit the share it was given: {}",
        estimate_text_tokens(&text)
    );
}
