use super::*;
use crate::input::{ImageAttachment, UserInput};

fn image(name: &str) -> ImageAttachment {
    ImageAttachment::new(name, format!("data:image/png;base64,{}", "QUJD"))
}

#[test]
fn attachment_names_render_in_the_user_record_and_input() {
    let mut app = App::new();
    app.push(Record::User {
        text: "look".to_owned(),
        images: vec![image("photo.png")],
    });
    let layout = view::record_layout(&mut app, 0, 40);
    let text = layout
        .iter()
        .map(|visual| visual.line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("[image] photo.png"), "{text}");
    assert!(!text.contains("QUJD"), "base64 must not be shown: {text}");

    // The input area shows pending attachment names too.
    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut editor = Editor::new();
    editor.insert_image(image("pending.png"));
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let rendered = buffer_text(terminal.backend().buffer());
    assert!(
        rendered.replace(' ', "").contains("[image]pending.png"),
        "{rendered}"
    );
}

#[test]
fn empty_backspace_removes_the_last_attachment() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_image(image("one.png"));
    editor.insert_image(image("two.png"));

    handle_key(&mut app, &mut editor, key(KeyCode::Backspace));
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.images()[0].name, "one.png");
    handle_key(&mut app, &mut editor, key(KeyCode::Backspace));
    assert!(editor.images().is_empty());
    handle_key(&mut app, &mut editor, key(KeyCode::Backspace));
    assert!(editor.images().is_empty());
}

#[test]
fn ctrl_c_clear_then_up_restores_text_and_all_attachments() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("hello");
    editor.insert_image(image("one.png"));
    editor.insert_image(image("two.png"));

    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(!outcome.quit);
    assert!(editor.is_empty());
    assert!(editor.images().is_empty());

    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "hello");
    assert_eq!(editor.images().len(), 2);
    assert_eq!(editor.images()[0].name, "one.png");
    assert_eq!(editor.images()[1].name, "two.png");
}

#[test]
fn busy_enter_keeps_the_draft_and_attachments() {
    let mut app = App::new();
    app.running = true;
    let mut editor = Editor::new();
    editor.insert_str("draft");
    editor.insert_image(image("busy.png"));

    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none());
    assert!(app.status.is_some());
    assert_eq!(editor.text(), "draft");
    assert_eq!(editor.images().len(), 1);
}

#[test]
fn an_added_attachment_prevents_up_from_replacing_the_draft() {
    let mut app = App::new();
    app.history.push("old".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "old");

    // Adding an attachment makes the draft differ from the recalled entry, so
    // Up must move the cursor instead of navigating history.
    editor.insert_image(image("new.png"));
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "old");
    assert_eq!(editor.images().len(), 1);
}

#[test]
fn failed_image_path_keeps_text_and_existing_attachments() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_image(image("keep.png"));

    crate::frontend::tui::handle_paste_text(&mut app, &mut editor, "/nonexistent/saki/missing.png");
    assert!(app.status.is_some(), "a bad explicit path reports an error");
    assert_eq!(editor.text(), "/nonexistent/saki/missing.png");
    assert_eq!(editor.images().len(), 1, "attachments are not cleared");
    assert_eq!(editor.images()[0].name, "keep.png");

    crate::frontend::tui::handle_paste_text(&mut app, &mut editor, "just some text");
    assert!(editor.text().ends_with("just some text"));
    assert_eq!(editor.images().len(), 1);
}

fn png_bytes() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    cursor.into_inner()
}

#[test]
fn loaded_image_snapshot_survives_source_changes_and_history_round_trip() {
    let bytes = png_bytes();
    // A PNG saved with a .jpg name must be sniffed as image/png.
    let path = std::env::temp_dir().join(format!("saki-snapshot-{}.jpg", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let mut app = App::new();
    let mut editor = Editor::new();
    crate::frontend::tui::handle_paste_text(&mut app, &mut editor, &path.to_string_lossy());
    assert!(app.status.is_none(), "{:?}", app.status);
    assert_eq!(editor.images().len(), 1);
    assert!(
        editor.images()[0]
            .data_url
            .starts_with("data:image/png;base64,")
    );
    assert!(editor.images()[0].name.ends_with(".jpg"));
    let snapshot = editor.images()[0].clone();

    // The snapshot is immutable: deleting the source and recalling from history
    // returns the original bytes, not a fresh read.
    std::fs::remove_file(&path).unwrap();
    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.images()[0], snapshot);
}

#[test]
fn truncated_image_failure_keeps_the_existing_draft_and_attachments() {
    let bytes = png_bytes();
    let path = std::env::temp_dir().join(format!("saki-truncated-{}.png", std::process::id()));
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("keep");
    editor.insert_image(image("keep.png"));
    crate::frontend::tui::handle_paste_text(&mut app, &mut editor, &path.to_string_lossy());
    assert!(app.status.is_some(), "a truncated image reports an error");
    assert!(editor.text().contains("keep"));
    assert_eq!(editor.images().len(), 1, "existing attachment is kept");
    assert_eq!(editor.images()[0].name, "keep.png");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn user_message_history_keeps_attachments_across_recall() {
    let mut app = App::new();
    let message = UserInput {
        context: Vec::new(),
        text: "text".to_owned(),
        images: vec![image("history.png")],
    };
    app.submit(message);
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "text");
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.images()[0].name, "history.png");
}
