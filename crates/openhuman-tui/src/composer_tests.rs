use super::*;

#[test]
fn mouse_cursor_uses_wrapped_cells_and_combining_marks() {
    let mut composer = Composer::default();
    composer.set_text("界e\u{301}界");
    composer.click(1, 0, 3);
    let (_, row, col) = composer.display(3);
    assert_eq!((row, col), (1, 0));
    composer.insert_char('X');
    assert_eq!(composer.text(), "界e\u{301}X界");
}

#[test]
fn unicode_editing_uses_character_indices() {
    let mut c = Composer::default();
    c.insert_str("a🦀b");
    c.move_left();
    c.backspace();
    assert_eq!(c.text(), "ab");
    assert_eq!(c.cursor(), 1);
}

#[test]
fn multiline_home_end_and_wrapping() {
    let mut c = Composer::default();
    c.set_text("one\ntwo long");
    c.move_home();
    assert_eq!(c.cursor(), 4);
    c.move_end();
    assert_eq!(c.cursor(), 12);
    let (rows, row, col) = c.display(5);
    assert_eq!(rows, vec!["one", "two l", "ong"]);
    assert_eq!((row, col), (2, 3));
}

#[test]
fn history_preserves_draft_and_deduplicates() {
    let mut c = Composer::default();
    c.set_text("first");
    assert_eq!(c.take_for_send().as_deref(), Some("first"));
    c.set_text("first");
    c.take_for_send();
    c.set_text("draft");
    c.history_previous();
    assert_eq!(c.text(), "first");
    c.history_next();
    assert_eq!(c.text(), "draft");
}

#[test]
fn slash_commands_filter_fuzzily() {
    let mut c = Composer::default();
    c.set_text("/perm");
    assert_eq!(c.command_matches()[0].0, "permissions");
    c.set_text("/rsm");
    assert_eq!(c.command_matches()[0].0, "resume");
}

#[test]
fn file_mentions_replace_only_the_current_token() {
    let mut composer = Composer::default();
    composer.set_text("review @src/ma");
    assert_eq!(composer.file_query().as_deref(), Some("src/ma"));
    composer.replace_current_token("@src/main.rs");
    assert_eq!(composer.text(), "review @src/main.rs");
}
