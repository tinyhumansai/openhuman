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

#[test]
fn a_render_older_than_a_block_already_taken_does_not_store_its_own() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let older = ready.ticket(Path::new("/workspace"));
    let newer = ready.ticket(Path::new("/workspace"));
    ready.put_ticketed_at(key("a", 500), Some("new".into()), newer, now);
    assert_eq!(ready.take_at(&key("a", 500), now), Some(Some("new".into())));
    ready.put_ticketed_at(key("a", 500), Some("old".into()), older, now);
    assert!(!ready.contains_at(&key("a", 500), now));
}

#[test]
fn invalidating_idle_workspaces_leaves_nothing_behind() {
    let ready = ReadyBlocks::default();
    for n in 0..100 {
        ready.invalidate(Some(Path::new(&format!("/workspace-{n}"))));
    }
    ready.invalidate(None);
    let state = ready.lock();
    assert!(state.entries.is_empty());
    assert!(state.in_flight.is_empty());
}

#[test]
fn a_render_that_ends_without_a_block_is_forgotten() {
    let ready = ReadyBlocks::default();
    drop(ready.ticket(Path::new("/workspace")));
    assert!(ready.lock().in_flight.is_empty());
}

#[test]
fn a_render_that_stored_its_block_is_forgotten() {
    let ready = ReadyBlocks::default();
    let ticket = ready.ticket(Path::new("/workspace"));
    ready.put_ticketed_at(key("a", 500), None, ticket, Instant::now());
    assert!(ready.lock().in_flight.is_empty());
}

#[test]
fn a_voided_render_that_ends_is_forgotten_without_harm() {
    let ready = ReadyBlocks::default();
    let voided = ready.ticket(Path::new("/workspace"));
    let live = ready.ticket(Path::new("/other"));
    ready.invalidate(Some(Path::new("/workspace")));
    drop(voided);
    assert_eq!(ready.lock().in_flight.len(), 1, "the live ticket stays");
    let now = Instant::now();
    ready.put_ticketed_at(key_in("/other", "a", 500), Some("b".into()), live, now);
    assert!(ready.contains_at(&key_in("/other", "a", 500), now));
}

#[test]
fn an_older_render_does_not_store_over_one_a_generation_took_directly() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let older = ready.ticket(Path::new("/workspace"));
    let newer = ready.ticket(Path::new("/workspace"));
    ready.delivered_at(key("a", 500), newer, now);
    ready.put_ticketed_at(key("a", 500), Some("old".into()), older, now);
    assert!(!ready.contains_at(&key("a", 500), now));
}

#[test]
fn a_ticket_is_live_until_an_invalidation_voids_it() {
    let ready = ReadyBlocks::default();
    let one = ready.ticket(Path::new("/one"));
    let two = ready.ticket(Path::new("/two"));
    assert!(one.is_live() && two.is_live());
    ready.invalidate(Some(Path::new("/one")));
    assert!(!one.is_live(), "voided by an invalidation of its workspace");
    assert!(two.is_live(), "another workspace's render is untouched");
    ready.invalidate(None);
    assert!(!two.is_live());
}

#[test]
fn storing_reports_whether_the_block_was_kept() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let voided = ready.ticket(Path::new("/workspace"));
    ready.invalidate(Some(Path::new("/workspace")));
    assert!(!ready.put_ticketed_at(key("a", 500), Some("x".into()), voided, now));
    let older = ready.ticket(Path::new("/workspace"));
    let newer = ready.ticket(Path::new("/workspace"));
    assert!(ready.put_ticketed_at(key("a", 500), Some("new".into()), newer, now));
    assert!(!ready.put_ticketed_at(key("a", 500), Some("old".into()), older, now));
    let fresh = ready.ticket(Path::new("/workspace"));
    assert!(ready.put_ticketed_at(key("a", 500), None, fresh, now));
}

#[test]
fn a_ticket_watch_follows_the_ticket_it_was_taken_from() {
    let ready = ReadyBlocks::default();
    let ticket = ready.ticket(Path::new("/workspace"));
    let watch = ticket.watch();
    assert!(watch.is_live());
    ready.invalidate(None);
    assert!(!watch.is_live());
    drop(ticket);
    assert!(!watch.is_live(), "and stays void once the ticket is gone");
}

#[test]
fn a_delivery_watermark_outlives_the_eviction_of_blocks_for_other_keys() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    let older = ready.ticket(Path::new("/workspace"));
    let newer = ready.ticket(Path::new("/workspace"));
    ready.delivered_at(key("a", 500), newer, now);
    // Enough other keys to evict every block the store can hold.
    for tokens in 0..(READY_CAPACITY as u32 + 8) {
        ready.put_at(key("b", 1_000 + tokens), Some("other".into()), now);
    }
    assert!(
        !ready.put_ticketed_at(key("a", 500), Some("old".into()), older, now),
        "the older render stored its superseded block"
    );
    assert!(ready.lock().marks.len() <= 1, "and the marks stay bounded");
}

#[test]
fn watermarks_are_forgotten_once_no_older_render_is_in_flight() {
    let ready = ReadyBlocks::default();
    let now = Instant::now();
    for tokens in 0..32 {
        ready.put_at(key("a", tokens), Some("b".into()), now);
    }
    assert!(ready.lock().marks.is_empty());
}
