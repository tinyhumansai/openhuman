use super::super::actions::Hit;
use super::*;
use crossterm::event::KeyModifiers;
use ratatui::layout::Rect;
#[test]
fn release_and_drag_do_not_activate_and_wheel_unfollows() {
    let mut ui = UiState::new("t".into(), "c".into());
    ui.transcript_area = Rect::new(0, 2, 80, 15);
    ui.hits.push(Hit {
        area: Rect::new(0, 2, 80, 1),
        action: Action::Entry(3),
    });
    let mut mouse = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 2,
        row: 2,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(mouse_action(mouse, &mut ui), None);
    mouse.kind = MouseEventKind::ScrollUp;
    mouse_action(mouse, &mut ui);
    assert_eq!(ui.scroll_from_bottom, 3);
    mouse.kind = MouseEventKind::Down(MouseButton::Left);
    assert_eq!(mouse_action(mouse, &mut ui), Some(Action::Entry(3)));
}
