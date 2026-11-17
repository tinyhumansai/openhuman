use super::*;
use crate::core::runtime::{context::CoreContext, ContextOverlay, DispatchCtx, DomainSet};
use std::collections::HashSet;
use std::sync::Arc;
use tinymemory_api::{ItemKind, MemoryMeta};

fn agent(parent: &Arc<CoreContext>, id: &str) -> Arc<CoreContext> {
    parent.derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent(id),
    )
}

fn citation(id: &str) -> TurnCitation {
    TurnCitation {
        id: id.to_string(),
        key: "learning".to_string(),
        namespace: None,
        score: None,
        timestamp: String::new(),
        snippet: String::new(),
    }
}

#[tokio::test]
async fn turn_citations_stay_with_the_agent_that_recorded_them() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = agent(&root, "alpha");
    let beta = agent(&root, "beta");
    let thread = "shared-thread";

    {
        let scope_ctx = DispatchCtx::new(Arc::clone(&alpha));
        record_pack_citations(&scope_ctx, thread, vec![citation("alpha-cite")]);
    }
    {
        let scope_ctx = DispatchCtx::new(Arc::clone(&beta));
        record_pack_citations(&scope_ctx, thread, vec![citation("beta-cite")]);
    }

    let from_beta = take_turn_citations(&DispatchCtx::new(beta), thread);
    let from_alpha = take_turn_citations(&DispatchCtx::new(alpha), thread);

    assert_eq!(
        from_alpha.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["alpha-cite"]
    );
    assert_eq!(
        from_beta.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["beta-cite"]
    );
}

#[test]
fn turn_citations_are_capped_and_deduplicated() {
    let citation = |n: usize| tinymemory_api::Citation {
        id: tinymemory_api::ItemId(format!("cite-{n}")),
        kind: ItemKind::Learning,
        snippet: "s".repeat(1000),
        score: Some(0.5),
        meta: MemoryMeta::default(),
    };
    let thread = "thread-cap";
    let many: Vec<_> = (0..MAX_TURN_CITATIONS + 10).map(citation).collect();
    let ctx = DispatchCtx::ambient();
    record_recall_citations(&ctx, thread, &many);
    record_recall_citations(&ctx, thread, &many[..2]);
    record_recall_citations(&ctx, thread, &[]);
    let drained = take_turn_citations(&ctx, thread);
    assert_eq!(drained.len(), MAX_TURN_CITATIONS);
    assert_eq!(
        drained
            .iter()
            .map(|c| c.id.clone())
            .collect::<HashSet<_>>()
            .len(),
        MAX_TURN_CITATIONS
    );
    assert!(drained[0].snippet.chars().count() <= TURN_CITATION_SNIPPET_CHARS);
    assert_eq!(drained[0].key, "learning");
}

#[test]
fn citations_drain_per_thread_and_merge_pack_and_recall_sources() {
    let ctx = DispatchCtx::ambient();
    record_pack_citations(&ctx, "thread-a", vec![citation("pack"), citation("shared")]);
    record_pack_citations(&ctx, "thread-b", vec![citation("other-thread")]);
    let recall = tinymemory_api::Citation {
        id: tinymemory_api::ItemId("shared".to_string()),
        kind: ItemKind::Learning,
        snippet: "duplicate".to_string(),
        score: Some(0.5),
        meta: MemoryMeta::default(),
    };
    record_recall_citations(&ctx, "thread-a", &[recall]);

    let thread_a = take_turn_citations(&ctx, "thread-a");
    assert_eq!(
        thread_a
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["pack", "shared"]
    );
    assert_eq!(take_turn_citations(&ctx, "thread-a"), []);
    assert_eq!(take_turn_citations(&ctx, "thread-b")[0].id, "other-thread");
}
