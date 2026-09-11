use super::MAX_PENDING_EVENTS;
use std::collections::VecDeque;

fn push_event_local(queue: &mut VecDeque<String>, event: String) {
    queue.push_back(event);
    while queue.len() > MAX_PENDING_EVENTS {
        let _ = queue.pop_front();
    }
}

#[test]
fn event_queue_keeps_latest_events() {
    let mut queue = VecDeque::new();
    for index in 0..(MAX_PENDING_EVENTS + 5) {
        push_event_local(&mut queue, format!("event-{index}"));
    }

    assert_eq!(queue.len(), MAX_PENDING_EVENTS);
    assert_eq!(queue.front().map(String::as_str), Some("event-5"));
    let expected_last = format!("event-{}", MAX_PENDING_EVENTS + 4);
    assert_eq!(
        queue.back().map(String::as_str),
        Some(expected_last.as_str())
    );
}
