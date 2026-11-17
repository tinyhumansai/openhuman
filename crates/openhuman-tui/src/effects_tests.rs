use super::*;
#[test]
fn stale_catalog_cannot_replace_another_thread_or_closed_overlay() {
    let mut ui = UiState::new("new".into(), "client".into());
    ui.overlay_generation = 2;
    ui.overlay = Some(Overlay::new(OverlayKind::Model, "current"));
    apply(
        OverlayReply {
            thread_id: "old".into(),
            generation: 2,
            overlay: Overlay::new(OverlayKind::Model, "stale"),
        },
        &mut ui,
    );
    assert_eq!(ui.overlay.as_ref().unwrap().title, "current");
    ui.overlay = None;
    apply(
        OverlayReply {
            thread_id: "new".into(),
            generation: 2,
            overlay: Overlay::new(OverlayKind::Model, "stale"),
        },
        &mut ui,
    );
    assert!(ui.overlay.is_none());
}
#[test]
fn refreshing_catalog_preserves_typed_filter() {
    let mut ui = UiState::new("t".into(), "c".into());
    let mut current = Overlay::new(OverlayKind::Model, "models");
    current.filter = "fast".into();
    ui.overlay = Some(current);
    apply(
        OverlayReply {
            thread_id: "t".into(),
            generation: 0,
            overlay: Overlay::new(OverlayKind::Model, "models"),
        },
        &mut ui,
    );
    assert_eq!(ui.overlay.as_ref().unwrap().filter, "fast");
}
