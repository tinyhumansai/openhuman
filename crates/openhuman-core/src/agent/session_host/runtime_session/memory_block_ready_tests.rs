use super::*;

use std::path::Path;

use crate::config::Config;

fn key(agent: &str, max_tokens: u32) -> ReadyKey {
    key_in("/workspace", agent, max_tokens)
}

fn key_in(workspace: &str, agent: &str, max_tokens: u32) -> ReadyKey {
    ReadyKey {
        workspace: PathBuf::from(workspace),
        identity: tinymemory_tools::layout::identity::Identity::agent(agent)
            .resolve(&crate::memory::scope::identity_pins(&Config::default())),
        max_tokens,
    }
}

#[test]
fn a_ready_block_is_taken_once() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key("a", 500), Some("block".into()), now);
    assert!(ready.contains_at(&key("a", 500), now));
    assert_eq!(
        ready.take_at(&key("a", 500), now),
        Some(Some("block".into()))
    );
    assert_eq!(ready.take_at(&key("a", 500), now), None);
    assert!(!ready.contains_at(&key("a", 500), now));
}

#[test]
fn a_render_that_found_nothing_is_ready_too() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key("a", 500), None, now);
    assert_eq!(ready.take_at(&key("a", 500), now), Some(None));
}

#[test]
fn a_block_is_not_taken_for_another_identity_or_budget() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key("a", 500), Some("block".into()), now);
    assert_eq!(ready.take_at(&key("b", 500), now), None);
    assert_eq!(ready.take_at(&key("a", 400), now), None);
    assert!(ready.contains_at(&key("a", 500), now));
}

#[test]
fn a_stale_block_is_dropped() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key("a", 500), Some("block".into()), now);
    let later = now + READY_MAX_AGE + Duration::from_secs(1);
    assert!(!ready.contains_at(&key("a", 500), later));
    assert_eq!(ready.take_at(&key("a", 500), later), None);
    assert_eq!(
        ready.take_at(&key("a", 500), now),
        None,
        "and gone for good"
    );
}

#[test]
fn a_newer_block_replaces_an_older_one() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key("a", 500), Some("old".into()), now);
    ready.put_at(key("a", 500), Some("new".into()), now);
    assert_eq!(ready.take_at(&key("a", 500), now), Some(Some("new".into())));
}

#[test]
fn the_oldest_block_makes_room_at_capacity() {
    let ready = ReadyBlocks::default();
    let start = Instant::now();
    for n in 0..=READY_CAPACITY {
        let at = start + Duration::from_millis(n as u64);
        ready.put_at(key(&format!("agent-{n}"), 500), Some("block".into()), at);
    }
    let now = start + Duration::from_secs(1);
    assert!(
        !ready.contains_at(&key("agent-0", 500), now),
        "the oldest left"
    );
    assert!(ready.contains_at(&key("agent-1", 500), now));
    assert!(ready.contains_at(&key(&format!("agent-{READY_CAPACITY}"), 500), now));
}

#[test]
fn an_older_render_finishing_later_does_not_replace_a_newer_block() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let older = ready.ticket(Path::new("/workspace"));
    let newer = ready.ticket(Path::new("/workspace"));
    ready.put_ticketed_at(key("a", 500), Some("new".into()), newer, now);
    ready.put_ticketed_at(key("a", 500), Some("old".into()), older, now);
    assert_eq!(ready.take_at(&key("a", 500), now), Some(Some("new".into())));
}

#[test]
fn a_newer_render_replaces_the_block_of_an_older_one() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let older = ready.ticket(Path::new("/workspace"));
    let newer = ready.ticket(Path::new("/workspace"));
    ready.put_ticketed_at(key("a", 500), Some("old".into()), older, now);
    ready.put_ticketed_at(key("a", 500), Some("new".into()), newer, now);
    assert_eq!(ready.take_at(&key("a", 500), now), Some(Some("new".into())));
}

#[test]
fn an_invalidation_drops_every_waiting_block() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key_in("/one", "a", 500), Some("block".into()), now);
    ready.put_at(key_in("/two", "a", 500), None, now);
    ready.invalidate(None);
    assert!(!ready.contains_at(&key_in("/one", "a", 500), now));
    assert!(!ready.contains_at(&key_in("/two", "a", 500), now));
}

#[test]
fn an_invalidation_of_one_workspace_keeps_the_blocks_of_others() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.put_at(key_in("/one", "a", 500), Some("block".into()), now);
    ready.put_at(key_in("/two", "a", 500), Some("block".into()), now);
    ready.invalidate(Some(std::path::Path::new("/one")));
    assert!(!ready.contains_at(&key_in("/one", "a", 500), now));
    assert!(ready.contains_at(&key_in("/two", "a", 500), now));
}

#[test]
fn an_invalidation_of_one_workspace_does_not_void_renders_in_flight_for_others() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let in_flight = ready.ticket(Path::new("/two"));
    ready.invalidate(Some(Path::new("/one")));
    ready.put_ticketed_at(
        key_in("/two", "a", 500),
        Some("block".into()),
        in_flight,
        now,
    );
    assert!(ready.contains_at(&key_in("/two", "a", 500), now));
}

#[test]
fn an_invalidation_of_one_workspace_voids_renders_in_flight_for_it() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let in_flight = ready.ticket(Path::new("/one"));
    ready.invalidate(Some(Path::new("/one")));
    ready.put_ticketed_at(
        key_in("/one", "a", 500),
        Some("block".into()),
        in_flight,
        now,
    );
    assert!(!ready.contains_at(&key_in("/one", "a", 500), now));
}

#[test]
fn an_invalidation_voids_a_render_already_in_flight() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let in_flight = ready.ticket(Path::new("/workspace"));
    ready.invalidate(None);
    ready.put_ticketed_at(key("a", 500), Some("read before".into()), in_flight, now);
    assert!(!ready.contains_at(&key("a", 500), now));
}

#[test]
fn a_render_started_after_an_invalidation_may_store_its_block() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    ready.invalidate(None);
    let after = ready.ticket(Path::new("/workspace"));
    ready.put_ticketed_at(key("a", 500), Some("block".into()), after, now);
    assert!(ready.contains_at(&key("a", 500), now));
}

#[test]
fn background_renders_are_admitted_up_to_the_limit_and_freed_when_done() {
    static SLOTS: RenderSlots = RenderSlots::new(2);
    let first = SLOTS.acquire().expect("first");
    let _second = SLOTS.acquire().expect("second");
    assert!(SLOTS.acquire().is_none(), "the limit holds");
    drop(first);
    assert!(
        SLOTS.acquire().is_some(),
        "a finished render frees its slot"
    );
}

#[test]
fn background_renders_run_at_most_eight_at_once_for_at_most_a_minute() {
    assert_eq!(MAX_BACKGROUND_RENDERS, 8);
    assert_eq!(BACKGROUND_RENDER_DEADLINE, Duration::from_secs(60));
}
