use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    text: &'static str,
    media: bool,
    mergeable: bool,
}

fn text(text: &'static str) -> Item {
    Item {
        text,
        media: false,
        mergeable: true,
    }
}

fn media(text: &'static str) -> Item {
    Item {
        text,
        media: true,
        mergeable: true,
    }
}

fn solo(text: &'static str) -> Item {
    Item {
        text,
        media: false,
        mergeable: false,
    }
}

impl DebounceItem for Item {
    fn chars(&self) -> usize {
        self.text.chars().count()
    }
    fn carries_media(&self) -> bool {
        self.media
    }
    fn mergeable(&self) -> bool {
        self.mergeable
    }
}

fn settings() -> DebounceSettings {
    DebounceSettings {
        debounce: Duration::from_millis(1500),
        media_debounce: Duration::from_millis(4000),
        max_wait: Duration::from_millis(10_000),
        max_batch_messages: 12,
        max_batch_chars: 50_000,
    }
}

fn ms(base: Instant, offset: u64) -> Instant {
    base + Duration::from_millis(offset)
}

fn texts(batch: &[Item]) -> Vec<&'static str> {
    batch.iter().map(|item| item.text).collect()
}

type Debouncer = KeyedDebouncer<&'static str, Item>;

#[test]
fn quiet_window_resets_on_every_push_and_flushes_once() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    assert_eq!(d.push("k", text("a"), &settings(), t0), Push::Opened);
    assert_eq!(
        d.push("k", text("b"), &settings(), ms(t0, 1000)),
        Push::Buffered
    );
    assert_eq!(d.next_wakeup(), Some(ms(t0, 2500)));
    assert!(d.poll_ready(ms(t0, 2499)).is_empty());
    let ready = d.poll_ready(ms(t0, 2500));
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].0, "k");
    assert_eq!(texts(&ready[0].1), ["a", "b"]);
    assert_eq!(d.tracked(), 0);
}

#[test]
fn max_wait_caps_a_burst_that_never_goes_quiet() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    for step in 0..12u64 {
        d.push("k", text("x"), &settings(), ms(t0, step * 900));
    }
    assert_eq!(d.next_wakeup(), Some(ms(t0, 10_000)));
    assert!(d.poll_ready(ms(t0, 9_999)).is_empty());
    assert_eq!(d.poll_ready(ms(t0, 10_000))[0].1.len(), 12);
}

#[test]
fn media_stretches_the_window_without_passing_the_deadline() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("k", media("photo"), &settings(), t0);
    assert_eq!(d.next_wakeup(), Some(ms(t0, 4000)));
    d.push("k", text("caption follow-up"), &settings(), ms(t0, 3000));
    assert_eq!(d.next_wakeup(), Some(ms(t0, 7000)));
    d.push("k", text("more"), &settings(), ms(t0, 8000));
    assert_eq!(d.next_wakeup(), Some(ms(t0, 10_000)));
    let ready = d.poll_ready(ms(t0, 10_000));
    assert_eq!(texts(&ready[0].1), ["photo", "caption follow-up", "more"]);
}

#[test]
fn max_wait_is_raised_to_the_longest_window() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    let short = DebounceSettings {
        max_wait: Duration::from_millis(100),
        ..settings()
    };
    d.push("k", media("photo"), &short, t0);
    assert_eq!(d.next_wakeup(), Some(ms(t0, 4000)));
}

#[test]
fn zero_window_is_ready_immediately_and_never_merges() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    let none = DebounceSettings::immediate();
    assert_eq!(d.push("k", text("a"), &none, t0), Push::Ready);
    assert_eq!(d.push("k", text("b"), &none, t0), Push::Ready);
    let first = d.poll_ready(t0);
    assert_eq!(first.len(), 1);
    assert_eq!(texts(&first[0].1), ["a"]);
    assert!(d.poll_ready(t0).is_empty(), "key stays serialized");
    d.complete(&"k");
    assert_eq!(texts(&d.poll_ready(t0)[0].1), ["b"]);
    d.complete(&"k");
    assert!(d.is_empty());
}

#[test]
fn non_mergeable_item_seals_pending_batch_and_runs_behind_it() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("k", text("a"), &settings(), t0);
    assert_eq!(
        d.push("k", solo("/cmd"), &settings(), ms(t0, 10)),
        Push::Ready
    );
    assert_eq!(d.next_wakeup(), Some(ms(t0, 10)));
    let first = d.poll_ready(ms(t0, 10));
    assert_eq!(texts(&first[0].1), ["a"]);
    d.complete(&"k");
    assert_eq!(texts(&d.poll_ready(ms(t0, 10))[0].1), ["/cmd"]);
}

#[test]
fn message_cap_seals_and_starts_a_new_batch() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    let capped = DebounceSettings {
        max_batch_messages: 2,
        ..settings()
    };
    d.push("k", text("a"), &capped, t0);
    d.push("k", text("b"), &capped, t0);
    assert_eq!(d.push("k", text("c"), &capped, t0), Push::Ready);
    assert_eq!(texts(&d.poll_ready(t0)[0].1), ["a", "b"]);
    d.complete(&"k");
    assert!(d.poll_ready(t0).is_empty());
    assert_eq!(texts(&d.poll_ready(ms(t0, 1500))[0].1), ["c"]);
}

#[test]
fn char_cap_seals_before_the_overflowing_item() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    let capped = DebounceSettings {
        max_batch_chars: 5,
        ..settings()
    };
    d.push("k", text("abc"), &capped, t0);
    assert_eq!(d.push("k", text("def"), &capped, t0), Push::Ready);
    assert_eq!(texts(&d.poll_ready(t0)[0].1), ["abc"]);
}

#[test]
fn items_pushed_while_running_are_collected_into_one_follow_up() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("k", text("first"), &settings(), t0);
    assert_eq!(d.poll_ready(ms(t0, 1500)).len(), 1);
    assert!(d.is_running(&"k"));
    assert_eq!(
        d.push("k", text("b"), &settings(), ms(t0, 2000)),
        Push::Opened
    );
    d.push("k", text("c"), &settings(), ms(t0, 2100));
    assert_eq!(d.next_wakeup(), None, "running keys do not arm a wakeup");
    assert!(d.poll_ready(ms(t0, 9000)).is_empty());
    d.complete(&"k");
    let follow_up = d.poll_ready(ms(t0, 9000));
    assert_eq!(texts(&follow_up[0].1), ["b", "c"]);
}

#[test]
fn complete_keeps_waiting_while_the_user_is_still_typing() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("k", text("first"), &settings(), t0);
    d.poll_ready(ms(t0, 1500));
    d.push("k", text("b"), &settings(), ms(t0, 2000));
    d.complete(&"k");
    assert!(d.poll_ready(ms(t0, 2100)).is_empty());
    assert_eq!(d.next_wakeup(), Some(ms(t0, 3500)));
    assert_eq!(texts(&d.poll_ready(ms(t0, 3500))[0].1), ["b"]);
}

#[test]
fn complete_with_nothing_pending_forgets_the_key() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("k", text("a"), &settings(), t0);
    d.poll_ready(ms(t0, 1500));
    assert!(!d.is_empty());
    d.complete(&"k");
    assert!(d.is_empty());
    d.complete(&"unknown");
}

#[test]
fn overflow_past_tracked_limit_skips_window_but_stays_serialized() {
    let t0 = Instant::now();
    let mut d = Debouncer::with_max_tracked(1);
    assert_eq!(d.push("a", text("1"), &settings(), t0), Push::Opened);
    assert_eq!(d.tracked(), 1);
    assert_eq!(d.push("b", text("2"), &settings(), t0), Push::Ready);
    assert_eq!(d.push("b", text("3"), &settings(), t0), Push::Ready);
    assert_eq!(d.tracked(), 1);
    let ready = d.poll_ready(t0);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].0, "b");
    assert_eq!(texts(&ready[0].1), ["2"]);
    assert!(d.poll_ready(t0).is_empty());
    d.complete(&"b");
    assert_eq!(texts(&d.poll_ready(t0)[0].1), ["3"]);
}

#[test]
fn keys_are_independent_and_wakeup_is_the_minimum() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("a", text("1"), &settings(), t0);
    d.push("b", media("2"), &settings(), ms(t0, 100));
    assert_eq!(d.next_wakeup(), Some(ms(t0, 1500)));
    let ready = d.poll_ready(ms(t0, 1500));
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].0, "a");
    assert_eq!(d.next_wakeup(), Some(ms(t0, 4100)));
    assert_eq!(d.poll_ready(ms(t0, 4100))[0].0, "b");
}

#[test]
fn ready_batches_come_out_oldest_first() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    for key in ["c", "a", "b"] {
        d.push(key, solo("x"), &settings(), t0);
    }
    let keys: Vec<_> = d.poll_ready(t0).into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, ["c", "a", "b"]);
}

#[test]
fn flush_all_closes_windows_but_respects_running_keys() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("a", text("1"), &settings(), t0);
    d.poll_ready(ms(t0, 1500));
    d.push("a", text("2"), &settings(), ms(t0, 1600));
    d.push("b", text("3"), &settings(), ms(t0, 1600));
    d.flush_all(ms(t0, 1600));
    assert_eq!(d.tracked(), 0);
    let ready = d.poll_ready(ms(t0, 1600));
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].0, "b");
    d.complete(&"a");
    assert_eq!(texts(&d.poll_ready(ms(t0, 1600))[0].1), ["2"]);
}

#[test]
fn drain_all_returns_everything_in_order() {
    let t0 = Instant::now();
    let mut d = Debouncer::new();
    d.push("a", text("1"), &settings(), t0);
    d.push("a", solo("2"), &settings(), t0);
    d.push("a", text("3"), &settings(), t0);
    let drained = d.drain_all();
    let flat: Vec<_> = drained.iter().map(|(_, b)| texts(b)).collect();
    assert_eq!(flat, [vec!["1"], vec!["2"], vec!["3"]]);
    assert!(d.is_empty());
    assert_eq!(d.tracked(), 0);
}

#[test]
fn immediate_settings_and_default_constructor() {
    let none = DebounceSettings::immediate();
    assert!(none.debounce.is_zero());
    let d: Debouncer = KeyedDebouncer::default();
    assert!(d.is_empty());
    assert_eq!(d.next_wakeup(), None);
}

#[test]
fn oversized_windows_are_clamped_instead_of_overflowing() {
    let t0 = Instant::now();
    let huge = DebounceSettings {
        debounce: Duration::MAX,
        media_debounce: Duration::MAX,
        max_wait: Duration::MAX,
        ..settings()
    };
    let mut d = Debouncer::new();
    assert_eq!(d.push("k", text("a"), &huge, t0), Push::Opened);
    assert_eq!(d.push("k", media("b"), &huge, t0), Push::Buffered);
    let wakeup = d.next_wakeup().expect("open window has a wakeup");
    assert!(wakeup > t0 && wakeup <= t0 + MAX_WINDOW);
    assert!(d.poll_ready(ms(t0, 60_000)).is_empty());
    d.flush_all(t0);
    let ready = d.poll_ready(t0);
    assert_eq!(ready.len(), 1);
    assert_eq!(texts(&ready[0].1), ["a", "b"]);
}
