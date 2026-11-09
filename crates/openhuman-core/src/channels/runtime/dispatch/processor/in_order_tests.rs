use super::InOrder;

#[test]
fn releases_in_receive_order_when_admissions_finish_out_of_order() {
    let mut order = InOrder::new();
    assert!(order.finish(1, Some("two")).is_empty());
    assert!(order.finish(2, Some("three")).is_empty());
    assert_eq!(order.waiting(), 2);
    assert_eq!(
        order.finish(0, Some("one")),
        vec![(0, "one"), (1, "two"), (2, "three")]
    );
    assert_eq!(order.waiting(), 0);
}

#[test]
fn rejected_admissions_do_not_leave_a_gap() {
    let mut order = InOrder::new();
    assert!(order.finish(1, Some("two")).is_empty());
    assert_eq!(order.finish(0, None), vec![(1, "two")]);
    assert_eq!(order.finish(2, Some("three")), vec![(2, "three")]);
    assert_eq!(order.finish(3, None), Vec::<(u64, &str)>::new());
    assert_eq!(order.finish(4, Some("five")), vec![(4, "five")]);
}

#[test]
fn stale_sequence_numbers_are_ignored() {
    let mut order = InOrder::new();
    assert_eq!(order.finish(0, Some("one")), vec![(0, "one")]);
    assert!(order.finish(0, Some("again")).is_empty());
    assert_eq!(order.waiting(), 0);
}
