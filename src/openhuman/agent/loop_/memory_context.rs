use crate::openhuman::memory::Memory;
use std::collections::HashSet;
use std::fmt::Write;

const WORKING_MEMORY_KEY_PREFIX: &str = "working.user.";
const WORKING_MEMORY_LIMIT: usize = 3;

/// Build context preamble by searching memory for relevant entries.
/// Entries with a hybrid score below `min_relevance_score` are dropped to
/// prevent unrelated memories from bleeding into the conversation.
pub(crate) async fn build_context(
    mem: &dyn Memory,
    user_msg: &str,
    min_relevance_score: f64,
) -> String {
    let mut context = String::new();
    let mut seen_keys = HashSet::new();

    // Pull relevant memories for this message
    if let Ok(entries) = mem.recall(user_msg, 5, None).await {
        let relevant: Vec<_> = entries
            .iter()
            .filter(|e| match e.score {
                Some(score) => score >= min_relevance_score,
                None => true,
            })
            .collect();

        if !relevant.is_empty() {
            context.push_str("[Memory context]\n");
            for entry in &relevant {
                seen_keys.insert(entry.key.clone());
                let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
            }
            context.push('\n');
        }
    }

    // Explicitly load bounded user working memory entries so sync-derived profile
    // facts can influence the turn in a controlled way.
    let working_query = format!("working.user {user_msg}");
    if let Ok(entries) = mem
        .recall(&working_query, WORKING_MEMORY_LIMIT + 2, None)
        .await
    {
        let working: Vec<_> = entries
            .iter()
            .filter(|entry| entry.key.starts_with(WORKING_MEMORY_KEY_PREFIX))
            .filter(|entry| !seen_keys.contains(&entry.key))
            .filter(|entry| match entry.score {
                Some(score) => score >= min_relevance_score,
                None => true,
            })
            .take(WORKING_MEMORY_LIMIT)
            .collect();

        if !working.is_empty() {
            context.push_str("[User working memory]\n");
            for entry in &working {
                let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
            }
            context.push('\n');
        }
    }

    context
}
