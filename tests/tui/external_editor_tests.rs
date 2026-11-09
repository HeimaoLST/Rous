use super::*;
use crate::frontend::tui::external_editor;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("saki29-{}-{label}-{unique}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn script(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("my editor.sh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn launch(script: &Path, extra: &str) -> external_editor::Launch {
    external_editor::resolve(Some(&format!("\"{}\" {extra}", script.display())), None)
        .expect("parse launch")
}

#[test]
fn visual_precedes_editor_and_empty_values_fall_through() {
    let launch = external_editor::resolve(Some("vim -u NONE"), Some("nano")).unwrap();
    assert_eq!(launch.program, PathBuf::from("vim"));
    assert_eq!(launch.args, vec!["-u".to_owned(), "NONE".to_owned()]);

    let launch = external_editor::resolve(Some("   "), Some("nano -w")).unwrap();
    assert_eq!(launch.program, PathBuf::from("nano"));
    assert_eq!(launch.args, vec!["-w".to_owned()]);

    assert!(external_editor::resolve(None, Some("  ")).is_err());
    assert!(external_editor::resolve(Some("'unterminated"), None).is_err());
}

#[test]
fn quoted_executable_and_arguments_run_directly() {
    let dir = workspace("success");
    let script = script(
        &dir,
        r#"[ "$1" = "--flag" ] || exit 9
for last; do :; done
printf 'edited\ntext\n' > "$last""#,
    );
    let launch = launch(&script, "--flag");
    let output = external_editor::edit(&launch, "original").unwrap();
    assert_eq!(output, "edited\ntext\n");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn nonzero_exit_and_read_failures_are_errors() {
    let dir = workspace("fail");
    let failing = script(&dir, "exit 7");
    assert!(external_editor::edit(&launch(&failing, ""), "draft").is_err());

    let deleting = script(&dir, "for last; do :; done\nrm -f \"$last\"");
    assert!(external_editor::edit(&launch(&deleting, ""), "draft").is_err());

    let invalid = script(&dir, "for last; do :; done\nprintf '\\377' > \"$last\"");
    assert!(external_editor::edit(&launch(&invalid, ""), "draft").is_err());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn failed_result_preserves_exact_draft_cursor_images_and_history() {
    let dir = workspace("state");
    let mut app = App::new();
    app.last_width = 40;
    app.last_height = 8;
    let mut editor = Editor::new();
    editor.insert_str("keep\ndraft");
    editor.insert_image(crate::input::ImageAttachment::new(
        "a.png",
        "data:image/png;base64,AA",
    ));
    let text = editor.text();
    let cursor = editor.cursor();
    load_external_text(&mut app, &mut editor, Err("boom".to_owned()));
    assert_eq!(editor.text(), text);
    assert_eq!(editor.cursor(), cursor);
    assert_eq!(editor.images().len(), 1);
    assert!(app.feedback.is_some());

    load_external_text(&mut app, &mut editor, Ok("new\ntext".to_owned()));
    assert_eq!(editor.text(), "new\ntext");
    assert_eq!(editor.images().len(), 1);
    assert!(editor.undo());
    assert_eq!(editor.text(), text);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn replace_text_is_one_undo_noop_keeps_redo_and_ctrl_g_is_input_only() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("one");
    editor.insert_str("\ntwo");
    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('g'), KeyModifiers::CONTROL),
    );
    assert!(outcome.external_editor);
    assert!(outcome.send.is_none());
    assert_eq!(editor.text(), "one\ntwo");

    assert!(editor.replace_text("changed text"));
    assert_eq!(editor.text(), "changed text");
    assert_eq!(editor.cursor(), (0, "changed text".len()));
    assert!(editor.undo());
    assert_eq!(editor.text(), "one\ntwo");
    assert!(editor.redo());

    // Exact no-op never checkpoints and leaves the cursor where it was.
    assert!(editor.undo());
    editor.set_global_offset(2);
    let before = editor.cursor();
    assert!(!editor.replace_text("one\ntwo"));
    assert_eq!(editor.cursor(), before);
    assert!(editor.redo(), "a no-op must not discard redo history");

    // Overlays keep their precedence: help consumes Ctrl+G, search treats it
    // as a modal key, and an active path popup is dismissed before the editor.
    app.help_open = true;
    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('g'), KeyModifiers::CONTROL),
    );
    assert!(!outcome.external_editor);
    app.help_open = false;
    crate::frontend::tui::transcript_search::open(&mut app);
    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('g'), KeyModifiers::CONTROL),
    );
    assert!(!outcome.external_editor);
    crate::frontend::tui::transcript_search::cancel(&mut app);
}
