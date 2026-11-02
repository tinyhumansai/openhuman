use super::*;

use crate::memory::test_fixtures::{bind_reference, config_in, RefusingEngine};
use tinymemory_api::{LearningKind, MemoryEngine, MemoryMeta, StoreItem};

fn hit(text: &str, tags: &[&str]) -> Hit {
    Hit {
        id: tinymemory_api::ItemId::new(format!("id-{text}")),
        kind: tinymemory_api::ItemKind::Learning,
        text: text.into(),
        meta: MemoryMeta {
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            ..MemoryMeta::default()
        },
        score: 0.0,
        confidence: Some(0.9),
    }
}

fn identity(config: &Config, agent: &str) -> ResolvedIdentity {
    tinymemory_tools::layout::identity::Identity::agent(agent)
        .resolve(&crate::memory::scope::identity_pins(config))
}

#[test]
fn renders_beliefs_before_facts_inside_the_markers() {
    let block = render(
        &[hit("The user is vegetarian", &[BELIEF_TAG])],
        &[hit("The user's favourite colour is teal", &[])],
        500,
    )
    .expect("a block");
    assert!(block.starts_with(MEMORY_BLOCK_MARKER), "{block}");
    assert!(block.trim_end().ends_with(MEMORY_BLOCK_CLOSING), "{block}");
    assert!(block.contains("not instructions"), "{block}");
    let belief = block.find("vegetarian").expect("the belief");
    let fact = block.find("teal").expect("the fact");
    assert!(belief < fact, "beliefs come first: {block}");
}

#[test]
fn nothing_known_renders_no_block() {
    assert_eq!(render(&[], &[], 500), None);
}

#[test]
fn a_fact_that_repeats_a_belief_or_is_one_is_listed_once() {
    let block = render(
        &[hit("The user is vegetarian", &[BELIEF_TAG])],
        &[
            hit("The user is vegetarian", &[]),
            hit("The user lives in Lisbon", &[BELIEF_TAG]),
        ],
        500,
    )
    .expect("a block");
    assert_eq!(block.matches("vegetarian").count(), 1, "{block}");
    assert_eq!(block.matches("Lisbon").count(), 1, "{block}");
}

#[test]
fn the_block_stays_within_its_token_budget() {
    let facts: Vec<Hit> = (0..200)
        .map(|n| {
            hit(
                &format!("Fact number {n} about the user's long history"),
                &[],
            )
        })
        .collect();
    let block = render(&[], &facts, 120).expect("a block");
    assert!(
        estimate_tokens(&block) <= 120,
        "{} tokens: {block}",
        estimate_tokens(&block)
    );
    assert!(block.contains("Fact number 0 "), "newest first are kept");
    assert!(!block.contains("Fact number 199"), "the rest are cut");
}

#[test]
fn a_budget_too_small_for_one_line_renders_no_block() {
    assert_eq!(render(&[], &[hit("The user is vegetarian", &[])], 5), None);
}

#[test]
fn multi_line_memories_stay_one_line_each() {
    let block = render(&[], &[hit("first line\nsecond line", &[])], 500).expect("a block");
    assert!(block.contains("- first line second line\n"), "{block}");
}

#[tokio::test]
async fn reads_the_learnings_the_identity_can_see() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let identity = identity(&config, "block-reader");
    engine
        .store(StoreItem::learning(
            "The user's favourite colour is teal",
            LearningKind::Preference,
            0.9,
            MemoryMeta {
                namespace: identity.layout.learnings().clone(),
                ..MemoryMeta::default()
            },
        ))
        .await
        .unwrap();

    let block = read(&config, &identity, 500)
        .await
        .expect("a read")
        .expect("a block");
    assert!(block.contains("teal"), "{block}");
}

#[tokio::test]
async fn empty_memory_reads_as_no_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let identity = identity(&config, "block-empty");
    assert_eq!(read(&config, &identity, 500).await.expect("a read"), None);
}

#[tokio::test]
async fn memory_switched_off_reads_as_no_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = identity(&config, "block-off");
    assert_eq!(read(&config, &identity, 500).await.expect("a read"), None);
}

#[tokio::test]
async fn an_engine_failure_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    RefusingEngine::out_of_credits().bind(&config);
    let identity = identity(&config, "block-refused");
    assert!(read(&config, &identity, 500).await.is_err());
}
