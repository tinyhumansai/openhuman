use super::*;

use crate::config::Config;

fn key(agent: &str, max_tokens: u32) -> ReadyKey {
    ReadyKey {
        workspace: PathBuf::from("/workspace"),
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
