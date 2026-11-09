use super::*;
use crate::frontend::tui::sessions::Sessions;
use crate::frontend::tui::theme::Theme;
use crate::input::{ImageAttachment, UserInput};
use ratatui::style::Color;

#[test]
fn palette_update_repaints_and_preserves_reading_state() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.theme = Theme::light();
    app.push(Record::Assistant {
        text: "current answer".to_owned(),
    });
    editor.insert_str("kept draft");
    editor.insert_image(ImageAttachment::new("a.png", "data:image/png;base64,AA"));
    app.history.push(UserInput {
        context: Vec::new(),
        text: "earlier".to_owned(),
        images: Vec::new(),
    });
    app.focus = Focus::Transcript;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    app.selected = 0;
    let _ = view::record_layout(&mut app, 0, 40);
    assert!(app.layouts[0].is_some());

    app.set_theme(Theme::dark());
    assert!(
        app.layouts.iter().all(|layout| layout.is_none()),
        "cached layouts bake colors and must be invalidated"
    );

    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let text = buffer_text(terminal.backend().buffer());
    let rows: Vec<&str> = text.lines().collect();
    let answer = rows
        .iter()
        .position(|row| row.contains("current answer"))
        .expect("answer visible");
    let x = rows[answer].find("current answer").unwrap() as u16;
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .cell((x, answer as u16))
            .unwrap()
            .fg,
        Theme::dark().text,
        "the new palette reaches the rendered body"
    );
    assert_ne!(Theme::light().text, Theme::dark().text);

    assert_eq!(editor.text(), "kept draft");
    assert_eq!(editor.images().len(), 1);
    assert!(!app.history.is_empty());
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        })
    );
    assert_eq!(app.selected, 0);
    assert_eq!(app.focus, Focus::Transcript);
    assert!(!app.follow);
    assert_eq!(app.records.len(), 1);
}

#[test]
fn native_fallback_renders_before_detection() {
    let mut app = App::new();
    let editor = Editor::new();
    app.theme = Theme::fallback();
    app.push(Record::User {
        text: "question".to_owned(),
        images: Vec::new(),
    });
    app.push(Record::Assistant {
        text: "answer".to_owned(),
    });
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(40)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let answer_row = rows.iter().position(|row| row.contains("answer")).unwrap();
    let x = rows[answer_row].find("answer").unwrap() as u16;
    assert_eq!(
        buffer.cell((x, answer_row as u16)).unwrap().fg,
        Color::Reset,
        "body text follows the terminal foreground instead of guessing"
    );
    let user_row = rows
        .iter()
        .position(|row| row.contains("question"))
        .unwrap();
    for column in 0..40u16 {
        let cell = buffer.cell((column, user_row as u16)).unwrap();
        assert_eq!(cell.fg, Theme::fallback().user_fg, "paired user foreground");
        assert_eq!(cell.bg, Theme::fallback().user_bg, "paired user background");
    }
    // Selection stays a known readable pair even though accent is Reset.
    let selection = Theme::fallback().selection();
    assert_ne!(selection.fg.unwrap(), Color::Reset);
    assert_ne!(selection.bg.unwrap(), Color::Reset);
}

#[test]
fn session_switching_keeps_the_live_automatic_theme() {
    let (a, a_history) = super::sessions::snapshot("Theme A");
    let (b, b_history) = super::sessions::snapshot("Theme B");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    // The front end already detected a light terminal before switching.
    app.theme = Theme::light();
    sessions.apply_event(
        &mut app,
        &mut editor,
        super::sessions::opened(a.clone(), a_history.clone()),
    );
    assert_eq!(app.theme, Theme::light(), "open transfers the live palette");
    sessions.apply_event(&mut app, &mut editor, super::sessions::opened(b, b_history));
    assert_eq!(
        app.theme,
        Theme::light(),
        "switch transfers the live palette"
    );

    // A later appearance change updates the live palette; reopening a cached
    // session must not resurrect the palette it was cached with.
    app.set_theme(Theme::dark());
    sessions.apply_event(&mut app, &mut editor, super::sessions::opened(a, a_history));
    assert_eq!(
        app.theme,
        Theme::dark(),
        "cached sessions never restore a stale palette"
    );
}
