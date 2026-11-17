use super::*;

#[test]
fn overlay_targets_take_precedence_and_hidden_controls_have_no_target() {
    let hits = vec![
        Hit {
            area: Rect::new(0, 0, 80, 24),
            action: Action::Composer,
        },
        Hit {
            area: Rect::new(10, 5, 20, 1),
            action: Action::OverlayRow(2),
        },
    ];
    assert_eq!(hit_at(&hits, 12, 5), Some(Action::OverlayRow(2)));
    assert_eq!(hit_at(&hits, 80, 24), None);
}
