use super::*;

fn ctrl(code: KeyCode) -> KeyEvent {
    key_with(code, KeyModifiers::CONTROL)
}

fn alt(code: KeyCode) -> KeyEvent {
    key_with(code, KeyModifiers::ALT)
}

#[test]
fn ctrl_b_f_and_alt_letters_move_by_grapheme_and_word() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("a👨\u{200d}👩b");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('b')));
    assert_eq!(
        editor.cursor(),
        (0, 2),
        "Ctrl+B keeps the ZWJ grapheme intact"
    );
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('b')));
    assert_eq!(editor.cursor(), (0, 1));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('f')));
    assert_eq!(editor.cursor(), (0, 2));

    let before = editor.text();
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('b')));
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('f')));
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('x')));
    assert_eq!(
        editor.text(),
        before,
        "Alt letters are shortcuts, never text"
    );
}

#[test]
fn ctrl_a_e_and_home_end_have_distinct_line_boundaries() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("ab\ncd");
    editor.set_global_offset(1);
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('a')));
    assert_eq!(editor.cursor(), (0, 0));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('a')));
    assert_eq!(editor.cursor(), (0, 0), "already at document start");
    handle_key(&mut app, &mut editor, key(KeyCode::End));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('e')));
    assert_eq!(
        editor.cursor(),
        (1, 2),
        "Ctrl+E at line end moves to next line end"
    );
    handle_key(&mut app, &mut editor, key(KeyCode::Home));
    assert_eq!(editor.cursor(), (1, 0), "Home stays on the current line");
    handle_key(&mut app, &mut editor, key(KeyCode::End));
    assert_eq!(editor.cursor(), (1, 2));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('a')));
    assert_eq!(
        editor.cursor(),
        (1, 0),
        "Ctrl+A moves to the current line start"
    );
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('a')));
    assert_eq!(
        editor.cursor(),
        (0, 0),
        "Ctrl+A at line start moves to previous line start"
    );
}

#[test]
fn word_motion_handles_unicode_words_and_punctuation() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("foo,bar 世界 baz");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_eq!(editor.cursor(), (0, 11), "previous word start");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_eq!(
        editor.cursor(),
        (0, 9),
        "Unicode word boundary between CJK ideographs"
    );
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_eq!(editor.cursor(), (0, 8));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Right));
    assert_eq!(editor.cursor(), (0, 9), "end of the next word");
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('b')));
    assert_eq!(editor.cursor(), (0, 8), "Alt+B mirrors Ctrl+Left");

    editor.set_draft("foo,bar");
    editor.set_global_offset(7);
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_eq!(
        editor.cursor(),
        (0, 4),
        "punctuation trailing a word is one stop"
    );
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_eq!(editor.cursor(), (0, 3));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_eq!(editor.cursor(), (0, 0));
}

#[test]
fn word_delete_and_yank_round_trip_across_lines() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("foo bar baz");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('w')));
    assert_eq!(editor.text(), "foo bar ");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(editor.text(), "foo bar baz");

    editor.set_draft("foo\nbar");
    handle_key(&mut app, &mut editor, key(KeyCode::Home));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('w')));
    assert_eq!(editor.text(), "bar", "word kill crosses the newline");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(editor.text(), "foo\nbar");
}

#[test]
fn ctrl_u_k_kill_line_edges_and_newlines() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("hello world");
    editor.set_global_offset(5);
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('k')));
    assert_eq!(editor.text(), "hello");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('u')));
    assert_eq!(editor.text(), "");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(editor.text(), "hello");

    editor.set_draft("ab\ncd");
    handle_key(&mut app, &mut editor, key(KeyCode::Home));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('u')));
    assert_eq!(
        editor.text(),
        "abcd",
        "Ctrl+U at line start kills the newline"
    );
}

#[test]
fn kill_buffer_survives_send_and_history_recall() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("keep me");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('w')));
    assert_eq!(
        editor.take().text,
        "keep ",
        "the draft is submitted without the word"
    );
    editor.set_draft("recalled");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(
        editor.text(),
        "recalledme",
        "kill buffer survives take and recall"
    );

    // Ordinary character deletion must not overwrite the kill buffer.
    editor.clear();
    editor.insert_str("xy");
    handle_key(&mut app, &mut editor, key(KeyCode::Backspace));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(editor.text(), "xme");

    // Empty-input kills with nothing to delete must not overwrite it either.
    let mut editor = Editor::new();
    editor.insert_str("ab");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('w')));
    assert!(editor.is_empty());
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('w')));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('u')));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('k')));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(editor.text(), "ab", "no-op kills keep the previous buffer");
}

#[test]
fn ctrl_h_and_ctrl_d_edit_and_ctrl_d_empty_uses_exit_path() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("abc");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('h')));
    assert_eq!(editor.text(), "ab");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('a')));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('d')));
    assert_eq!(editor.text(), "b");

    editor.clear();
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('d')));
    assert!(outcome.quit, "idle empty Ctrl+D quits");
    app.running = true;
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('d')));
    assert!(!outcome.quit);
    assert!(app.quit_after_turn);
    assert!(app.status.is_some());
}

#[test]
fn ctrl_j_inserts_a_newline() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("a");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('j')));
    assert_eq!(editor.text(), "a\n");
    assert_eq!(editor.cursor(), (1, 0));
}

#[test]
fn display_line_soft_wrap_end_does_not_claim_the_next_fragment() {
    let editor = {
        let mut editor = Editor::new();
        editor.insert_str("abcd界abcd");
        editor
    };
    let layout = editor.layout(5);
    let wrapped = &layout.lines[1];
    assert!(!wrapped.is_logical_end);
    assert_eq!(wrapped.text, "界abc");
    assert_eq!(
        wrapped.position_for_cell(5),
        (0, 7),
        "a soft-wrap fragment clamps into its last grapheme"
    );
    let last = layout.lines.last().unwrap();
    assert!(last.is_logical_end);
    assert_eq!(
        last.position_for_cell(5),
        (0, 9),
        "the logical end may use EOL"
    );
}

#[test]
fn visual_navigation_wraps_and_restores_the_preferred_column() {
    let mut editor = {
        let mut editor = Editor::new();
        editor.insert_str("abcd界abcd");
        editor
    };
    assert_eq!(editor.layout(5).cursor_line, 2);
    editor.move_visual_up(5);
    assert_eq!(
        editor.cursor(),
        (0, 4),
        "up lands inside the previous fragment"
    );
    editor.move_visual_up(5);
    assert_eq!(editor.cursor(), (0, 1));

    let mut editor = {
        let mut editor = Editor::new();
        editor.insert_str("abcdef\nxy\nabcdef");
        editor
    };
    editor.move_visual_up(10);
    assert_eq!(editor.cursor(), (1, 2));
    editor.move_visual_up(10);
    assert_eq!(
        editor.cursor(),
        (0, 6),
        "preferred column restored on the longer line"
    );
    editor.move_visual_down(10);
    assert_eq!(editor.cursor(), (1, 2));
    editor.move_visual_down(10);
    assert_eq!(editor.cursor(), (2, 6));
}

#[test]
fn visual_navigation_reaches_document_head_and_tail() {
    let mut editor = Editor::new();
    editor.insert_str("hello");
    editor.set_global_offset(3);
    editor.move_visual_up(80);
    assert_eq!(
        editor.cursor(),
        (0, 0),
        "Up on the first visual line goes to the head"
    );
    editor.move_visual_down(80);
    assert_eq!(
        editor.cursor(),
        (0, 5),
        "Down on the last visual line goes to the tail"
    );
}

#[test]
fn history_navigation_replaces_and_restores_drafts() {
    let mut app = App::new();
    app.history.push("first".to_owned());
    app.history.push("second".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "second");
    assert_eq!(editor.cursor(), (0, 6), "recall puts the cursor at the end");
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "first");
    handle_key(&mut app, &mut editor, key(KeyCode::Down));
    assert_eq!(editor.text(), "second");
    handle_key(&mut app, &mut editor, key(KeyCode::Down));
    assert_eq!(
        editor.text(),
        "",
        "down past the newest restores an empty draft"
    );
}

#[test]
fn history_navigation_yields_to_cursor_movement_after_edits() {
    let mut app = App::new();
    app.history.push("first".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "first");
    handle_key(&mut app, &mut editor, key(KeyCode::Char('x')));
    assert_eq!(editor.text(), "firstx");
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(
        editor.text(),
        "firstx",
        "edited recall does not navigate history"
    );
}

#[test]
fn history_dedupes_adjacent_entries() {
    let mut app = App::new();
    app.history.push("a".to_owned());
    app.history.push("a".to_owned());
    app.history.push("b".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "b");
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "a");
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(
        editor.text(),
        "a",
        "oldest entry does not advance past itself"
    );
}

#[test]
fn ctrl_c_saves_and_clears_a_draft_even_while_busy() {
    let mut app = App::new();
    app.running = true;
    let mut editor = Editor::new();
    editor.insert_str("draft");
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('c')));
    assert!(!outcome.quit);
    assert!(
        !app.quit_after_turn,
        "clearing a busy draft does not request exit"
    );
    assert!(editor.is_empty());
    assert_eq!(app.focus, Focus::Input);
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(
        editor.text(),
        "draft",
        "cleared draft is recalled from history"
    );
}

#[test]
fn word_delete_keeps_combining_clusters_intact() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("a:\u{301}");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('w')));
    assert_eq!(editor.text(), "a", "backward word kill keeps the cluster");

    editor.set_draft("");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('y')));
    assert_eq!(
        editor.text(),
        ":\u{301}",
        "kill buffer holds the whole cluster, not a stray mark"
    );

    let mut editor = Editor::new();
    editor.insert_str(":\u{301}b");
    handle_key(&mut app, &mut editor, key(KeyCode::Home));
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('d')));
    assert_eq!(editor.text(), "b", "forward word kill keeps the cluster");
}

#[test]
fn word_motion_does_not_stick_inside_a_combining_cluster() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str(":\u{301}b");
    handle_key(&mut app, &mut editor, key(KeyCode::Home));
    assert_eq!(editor.cursor(), (0, 0));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Right));
    assert_ne!(editor.cursor(), (0, 0), "word-right must progress");
    let after_right = editor.cursor();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Left));
    assert_ne!(editor.cursor(), after_right, "word-left must progress");
    assert_eq!(editor.cursor(), (0, 0));
}

#[test]
fn preferred_column_survives_resize_and_reaches_the_previous_display_line() {
    let mut editor = Editor::new();
    editor.insert_str("abcdefghijklmnopqrstuvwxyz\nsecond");
    editor.set_global_offset(20);
    // Wide layout: one display line per logical line; a vertical move records
    // the preferred column.
    editor.move_visual_down(40);
    assert_eq!(editor.cursor(), (1, 6));
    editor.move_visual_up(40);
    assert_eq!(editor.cursor(), (0, 20));

    // Narrow layout: the first logical line wraps into fragments. Continuing to
    // move up must step through the previous display lines, not skip them.
    assert_eq!(editor.layout(6).cursor_line, 3);
    editor.move_visual_up(6);
    assert_eq!(editor.cursor(), (0, 17));
    assert_eq!(editor.layout(6).cursor_line, 2);
    editor.move_visual_up(6);
    assert_eq!(editor.layout(6).cursor_line, 1);
    editor.move_visual_up(6);
    assert_eq!(editor.layout(6).cursor_line, 0);
    editor.move_visual_up(6);
    assert_eq!(editor.cursor(), (0, 0), "reaches the document head");
}

#[test]
fn narrowing_then_widening_keeps_the_draft_and_allows_renavigation() {
    let mut editor = Editor::new();
    let text = "你好世界你好世界\nsecond";
    editor.insert_str(text);
    editor.set_global_offset(0);

    editor.move_visual_down(6);
    let narrow_line = editor.layout(6).cursor_line;
    assert!(narrow_line > 0, "narrow layout wraps the first line");
    assert_eq!(editor.text(), text, "navigation never edits the draft");

    let wide_line = editor.layout(40).cursor_line;
    editor.move_visual_up(40);
    assert!(editor.layout(40).cursor_line <= wide_line);
    assert_eq!(editor.text(), text, "draft preserved across resize");
}

#[test]
fn ctrl_c_closes_the_log_before_touching_the_draft() {
    let path = std::env::temp_dir().join(format!("saki-input-log-{}.txt", std::process::id()));
    std::fs::write(&path, "line one\nline two\n").unwrap();
    let mut app = App::new();
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".to_owned(),
        name: "bash".to_owned(),
        arguments: "{}".to_owned(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".to_owned(),
        name: "bash".to_owned(),
        output: json!({
            "output": "line two\n",
            "exit_code": 0,
            "output_path": path.to_string_lossy(),
        }),
    });
    app.selected = 0;
    assert!(log::open(&mut app));
    app.running = true;
    let mut editor = Editor::new();
    editor.insert_str("draft");

    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('c')));
    assert!(app.log_view.is_none(), "Ctrl+C closes the log first");
    assert_eq!(editor.text(), "draft", "the draft is preserved");
    assert!(!app.quit_after_turn, "no exit flag yet");
    assert!(!outcome.quit);

    // Only the next Ctrl+C saves and clears the draft.
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('c')));
    assert!(editor.is_empty());
    assert!(!outcome.quit);
    assert!(!app.quit_after_turn, "busy clear does not request exit");
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "draft", "the cleared draft is in history");
    let _ = std::fs::remove_file(&path);
}

fn image(name: &str) -> crate::input::ImageAttachment {
    crate::input::ImageAttachment::new(name, "data:image/png;base64,AA")
}

fn render_history(app: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
    terminal
        .draw(|frame| crate::frontend::tui::history::draw(frame, app))
        .unwrap();
    buffer_text(terminal.backend().buffer())
}

#[test]
fn ctrl_r_opens_history_search_and_leaves_the_draft_untouched() {
    let mut app = App::new();
    app.history.push("older prompt".to_owned());
    app.history.push("newer prompt".to_owned());
    let mut editor = Editor::new();
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "keep me".into(),
        images: vec![image("a.png")],
    });
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    assert!(app.history.searching());
    assert!(outcome.send.is_none() && !outcome.cancel);
    assert_eq!(editor.text(), "keep me");
    assert_eq!(editor.images().len(), 1);
}

#[test]
fn history_search_accept_inserts_into_the_composer_without_sending() {
    let mut app = App::new();
    app.history.push("first prompt".to_owned());
    app.history.push("second prompt".to_owned());
    let mut editor = Editor::new();
    editor.insert_str("draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "second".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(!app.history.searching());
    assert_eq!(editor.text(), "second prompt");
    assert!(outcome.send.is_none(), "accept never sends");
}

#[test]
fn history_search_cancel_restores_the_exact_editor_state() {
    let mut app = App::new();
    app.history.push("history one".to_owned());
    let mut editor = Editor::new();
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "keep\ndraft".into(),
        images: vec![image("a.png")],
    });
    editor.set_global_offset(2);
    let cursor = editor.cursor();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "history".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!app.history.searching());
    assert_eq!(editor.text(), "keep\ndraft");
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.cursor(), cursor);
    assert!(!outcome.cancel);
}

#[test]
fn history_search_ctrl_c_cancels_without_clearing_the_draft() {
    let mut app = App::new();
    app.history.push("saved".to_owned());
    let mut editor = Editor::new();
    editor.insert_str("composer draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('c')));
    assert!(!app.history.searching());
    assert_eq!(editor.text(), "composer draft");
    assert!(!outcome.quit, "Ctrl+C cancels search, not the app");
}

#[test]
fn history_search_moves_older_and_newer() {
    let mut app = App::new();
    app.history.push("alpha one".to_owned());
    app.history.push("alpha two".to_owned());
    app.history.push("alpha three".to_owned());
    let mut editor = Editor::new();
    let search = |app: &mut App, editor: &mut Editor| {
        handle_key(app, editor, ctrl(KeyCode::Char('r')));
        for character in "alpha".chars() {
            handle_key(app, editor, key(KeyCode::Char(character)));
        }
    };
    // Newest match is selected first.
    search(&mut app, &mut editor);
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "alpha three");
    // Up then repeated Ctrl+R steps to older matches.
    search(&mut app, &mut editor);
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "alpha one");
    // Down moves newer again.
    search(&mut app, &mut editor);
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    handle_key(&mut app, &mut editor, key(KeyCode::Down));
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "alpha three");
}

#[test]
fn history_search_no_match_keeps_the_mode_open_without_accepting_or_sending() {
    // Empty history.
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    assert!(app.history.searching());
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none());
    assert_eq!(editor.text(), "draft");
    assert!(
        app.history.searching(),
        "empty history keeps the search open"
    );

    // Non-empty history with no match: repeated Enter neither accepts nor
    // sends, and the local mode stays visible.
    let mut app = App::new();
    app.history.push("other".to_owned());
    let mut editor = Editor::new();
    editor.insert_str("draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "zzz".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    for _ in 0..2 {
        let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
        assert!(outcome.send.is_none(), "no-match Enter never sends");
        assert!(app.history.searching(), "no-match Enter keeps the mode");
        assert_eq!(editor.text(), "draft");
    }
}

#[test]
fn history_search_f1_closes_search_and_opens_help() {
    let mut app = App::new();
    app.history.push("saved".to_owned());
    let mut editor = Editor::new();
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "draft".into(),
        images: vec![image("a.png")],
    });
    editor.set_global_offset(1);
    let cursor = editor.cursor();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    handle_key(&mut app, &mut editor, key(KeyCode::Char('s')));
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
    assert!(!app.history.searching());
    assert!(app.help_open, "F1 opens help after closing search");
    assert!(outcome.send.is_none() && !outcome.cancel);
    // Esc closes help; the draft, images and cursor are retained.
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!app.help_open);
    assert_eq!(editor.text(), "draft");
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.cursor(), cursor);
}

#[test]
fn history_search_consumes_mouse_except_a_help_click() {
    let mut app = App::new();
    app.theme = crate::frontend::tui::theme::Theme::dark();
    app.history.push("saved".to_owned());
    app.transcript_area = Rect::new(0, 0, 80, 20);
    app.help_button_area = Rect::new(70, 20, 10, 1);
    app.session_button_area = Rect::new(0, 20, 10, 1);
    app.input_area = Rect::new(0, 14, 80, 4);
    let mut editor = Editor::new();
    editor.insert_str("draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    // Wheel and background clicks are consumed; the mode and focus stay put.
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp, 5, 5));
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 5, 5),
    );
    assert!(app.history.searching());
    assert_eq!(app.focus, Focus::Input);
    assert_eq!(editor.text(), "draft");
    // Ctrl+C still cancels the search without clearing the draft.
    let outcome = handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('c')));
    assert!(!app.history.searching());
    assert_eq!(editor.text(), "draft");
    assert!(!outcome.quit);
    // A Help click is global and closes search before opening help.
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 72, 20),
    );
    assert!(!app.history.searching());
    assert!(app.help_open);
}

#[test]
fn history_search_overlay_keeps_the_hint_with_a_tall_composer() {
    let mut app = App::new();
    app.theme = crate::frontend::tui::theme::Theme::dark();
    app.history
        .push("first line\nsecond line\nthird line".to_owned());
    app.history.push(crate::input::UserInput {
        context: Vec::new(),
        text: "target one is a long prompt that does not fit one row\nsecond line of that prompt"
            .into(),
        images: vec![image("a.png"), image("b.png")],
    });
    // A five-line composer leaves only two rows above the input.
    app.input_area = Rect::new(0, 6, 40, 6);
    let mut editor = Editor::new();
    editor.insert_str("draft line 1\ndraft line 2\ndraft line 3\ndraft line 4\ndraft line 5");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "target".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    let mut terminal = Terminal::new(TestBackend::new(40, 15)).unwrap();
    terminal
        .draw(|frame| crate::frontend::tui::history::draw(frame, &app))
        .unwrap();
    let screen = buffer_text(terminal.backend().buffer());
    assert!(screen.contains("History search"), "{screen}");
    assert!(screen.contains("Search: target"), "{screen}");
    assert!(
        screen.contains("Esc cancel"),
        "the exit hint stays visible: {screen}"
    );
    assert!(
        screen.contains("…"),
        "clipped preview shows omission: {screen}"
    );
    assert!(
        screen.contains("[2 images]"),
        "the attachment count stays apparent: {screen}"
    );
    // The real composer is untouched.
    assert!(editor.text().contains("draft line 5"));
    assert!(editor.images().is_empty());
}

#[test]
fn history_search_esc_does_not_cancel_a_running_task_first() {
    let mut app = App::new();
    app.running = true;
    app.history.push("previous".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    assert!(app.history.searching());
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel, "first Esc closes the local search");
    assert!(!app.history.searching());
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(outcome.cancel, "a later Esc cancels the task");
}

#[test]
fn history_search_busy_accept_does_not_send() {
    let mut app = App::new();
    app.running = true;
    app.history.push("queued prompt".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none());
    assert_eq!(editor.text(), "queued prompt");
}

#[test]
fn history_search_query_backspace_is_grapheme_safe() {
    let mut app = App::new();
    app.history.push("family a👨\u{200d}👩b".to_owned());
    app.history.push("plain".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "👨\u{200d}👩".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    // A grapheme-safe Backspace removes the whole ZWJ sequence, leaving an
    // empty query whose newest match is "plain".
    handle_key(&mut app, &mut editor, key(KeyCode::Backspace));
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "plain");
}

#[test]
fn history_search_paste_edits_only_the_query() {
    let mut app = App::new();
    app.history.push("pasted prompt".to_owned());
    let mut editor = Editor::new();
    editor.insert_str("composer draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    crate::frontend::tui::handle_paste_text(&mut app, &mut editor, "pasted\nprompt");
    assert_eq!(
        editor.text(),
        "composer draft",
        "paste never mutates the hidden draft"
    );
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none());
    assert_eq!(editor.text(), "pasted prompt");
}

#[test]
fn history_search_accept_aligns_ordinary_navigation() {
    let mut app = App::new();
    app.history.push("one".to_owned());
    app.history.push("two".to_owned());
    app.history.push("three".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "two".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "two");
    // Ordinary recall continues from the accepted entry.
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "one");
}

#[test]
fn history_search_cancel_leaves_navigation_unchanged() {
    let mut app = App::new();
    app.history.push("one".to_owned());
    app.history.push("two".to_owned());
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "two");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    handle_key(&mut app, &mut editor, key(KeyCode::Down));
    assert_eq!(
        editor.text(),
        "",
        "cancel kept the prior navigation position"
    );
}

#[test]
fn history_search_overlay_renders_preview_and_states() {
    let mut app = App::new();
    app.history.push("first\nsecond".to_owned());
    app.history.push("target prompt".to_owned());
    app.input_area = Rect::new(0, 12, 60, 4);
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    for character in "target".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    let screen = render_history(&app, 60);
    assert!(screen.contains("History search"), "{screen}");
    assert!(screen.contains("Search: target"), "{screen}");
    assert!(screen.contains("target prompt"), "{screen}");
    for character in "zzz".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    let screen = render_history(&app, 60);
    assert!(screen.contains("No matching history"), "{screen}");

    let mut app = App::new();
    app.input_area = Rect::new(0, 12, 40, 4);
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('r')));
    let screen = render_history(&app, 40);
    assert!(screen.contains("History search"), "{screen}");
    assert!(screen.contains("No history yet"), "{screen}");
}

#[test]
fn undo_redo_coalesces_consecutive_typing() {
    let mut editor = Editor::new();
    editor.insert_typed('a');
    editor.insert_typed('b');
    editor.insert_typed('c');
    assert_eq!(editor.text(), "abc");
    assert!(editor.undo(), "one step undoes the whole typing run");
    assert_eq!(editor.text(), "");
    assert!(editor.redo());
    assert_eq!(editor.text(), "abc");
    assert_eq!(editor.cursor(), (0, 3));
}

#[test]
fn navigation_breaks_typing_groups() {
    let mut editor = Editor::new();
    editor.insert_typed('a');
    editor.insert_typed('b');
    editor.move_left();
    editor.insert_typed('c');
    assert_eq!(editor.text(), "acb");
    assert!(editor.undo(), "movement starts a new group");
    assert_eq!(editor.text(), "ab");
    assert_eq!(
        editor.cursor(),
        (0, 1),
        "group snapshot restores the cursor"
    );
    assert!(editor.undo());
    assert_eq!(editor.text(), "");

    let mut editor = Editor::new();
    editor.insert_typed('a');
    editor.insert_typed('b');
    editor.set_global_offset(0);
    editor.insert_typed('x');
    assert_eq!(editor.text(), "xab");
    assert!(editor.undo(), "set_global_offset breaks coalescing");
    assert_eq!(editor.text(), "ab");
    assert_eq!(editor.cursor(), (0, 0));
}

#[test]
fn paste_is_atomic_and_kill_yank_are_single_steps() {
    let mut editor = Editor::new();
    editor.insert_str("hello\nworld");
    assert_eq!(editor.text(), "hello\nworld");
    assert!(editor.undo(), "a whole paste is one step");
    assert_eq!(editor.text(), "");

    let mut editor = Editor::new();
    editor.insert_str("abc");
    editor.kill_to_line_start();
    assert_eq!(editor.text(), "");
    assert!(editor.undo());
    assert_eq!(editor.text(), "abc");
    editor.home();
    editor.yank();
    assert_eq!(editor.text(), "abcabc", "yank inserts at the cursor");
    assert!(editor.undo(), "yank is one step");
    assert_eq!(editor.text(), "abc");
    editor.yank();
    assert_eq!(editor.text(), "abcabc", "kill buffer survives undo");
}

#[test]
fn undo_restores_unicode_graphemes_and_cursor() {
    let mut editor = Editor::new();
    editor.insert_typed('e');
    editor.insert_typed('\u{301}');
    assert_eq!(editor.text(), "e\u{301}");
    assert_eq!(editor.cursor(), (0, 1), "combining mark is one grapheme");
    assert!(editor.undo());
    assert_eq!(editor.text(), "");
    assert!(editor.redo());
    assert_eq!(editor.text(), "e\u{301}");
    assert_eq!(editor.cursor(), (0, 1));

    let mut editor = Editor::new();
    for character in "a👨\u{200d}👩b".chars() {
        editor.insert_typed(character);
    }
    assert_eq!(editor.text(), "a👨\u{200d}👩b");
    assert_eq!(editor.cursor(), (0, 3), "ZWJ sequence is one grapheme");
    assert!(editor.undo());
    assert_eq!(editor.text(), "");
    assert!(editor.redo());
    assert_eq!(editor.text(), "a👨\u{200d}👩b");
    assert_eq!(editor.cursor(), (0, 3));
}

#[test]
fn undo_restores_multiline_cursor() {
    let mut editor = Editor::new();
    editor.insert_typed('a');
    editor.insert_newline();
    editor.insert_typed('b');
    assert_eq!(editor.text(), "a\nb");
    assert_eq!(editor.cursor(), (1, 1));
    assert!(editor.undo(), "newline edit is its own step");
    assert_eq!(editor.text(), "a\n");
    assert_eq!(editor.cursor(), (1, 0));
    assert!(editor.undo());
    assert_eq!(editor.text(), "a");
    assert_eq!(editor.cursor(), (0, 1));
    assert!(editor.redo());
    assert!(editor.redo());
    assert_eq!(editor.text(), "a\nb");
    assert_eq!(editor.cursor(), (1, 1));
}

#[test]
fn undo_restores_attachments_and_shares_payload() {
    let mut editor = Editor::new();
    editor.insert_image(crate::input::ImageAttachment::new(
        "a.png",
        "data:image/png;base64,AAA",
    ));
    editor.insert_image(crate::input::ImageAttachment::new(
        "b.png",
        "data:image/png;base64,BBB",
    ));
    assert_eq!(editor.images_shared_count(), 1);
    editor.insert_typed('x');
    assert_eq!(
        editor.images_shared_count(),
        2,
        "text checkpoints share the attachment Arc instead of cloning base64"
    );
    assert!(editor.undo());
    assert_eq!(editor.text(), "");
    assert_eq!(
        editor.images().len(),
        2,
        "attachments restore with the draft"
    );
    assert_eq!(editor.images()[1].name, "b.png");
    assert!(editor.redo());
    assert_eq!(editor.text(), "x");
    assert_eq!(editor.images().len(), 2);

    // Removing the last image is one undoable collection change.
    assert!(editor.remove_last_image());
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.images()[0].data_url, "data:image/png;base64,AAA");
    assert!(editor.undo(), "image removal is undoable");
    assert_eq!(editor.images().len(), 2);
    assert_eq!(editor.images()[0].data_url, "data:image/png;base64,AAA");
    assert_eq!(editor.images()[1].data_url, "data:image/png;base64,BBB");
    assert!(editor.redo(), "redo removes the image again");
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.images()[0].data_url, "data:image/png;base64,AAA");
}

#[test]
fn noop_edits_preserve_redo_and_new_edits_discard_it() {
    let mut editor = Editor::new();
    editor.insert_str("ab");
    assert!(editor.undo());
    assert_eq!(editor.text(), "");
    editor.backspace();
    assert_eq!(editor.text(), "", "no-op deletion at the start");
    assert!(editor.redo(), "a no-op must not discard redo");
    assert_eq!(editor.text(), "ab");
    assert_eq!(editor.cursor(), (0, 2));
    editor.delete();
    assert_eq!(editor.text(), "ab", "no-op deletion at the end");

    assert!(editor.undo());
    assert_eq!(editor.text(), "");
    editor.insert_typed('z');
    assert_eq!(editor.text(), "z");
    assert!(!editor.redo(), "a new edit after undo discards redo");
}

#[test]
fn clear_send_and_reset_history_semantics() {
    let mut editor = Editor::new();
    editor.insert_str("draft");
    editor.insert_image(image("a.png"));
    editor.clear();
    assert!(editor.is_empty());
    assert!(editor.undo(), "Ctrl+C-style clear is undoable");
    assert_eq!(editor.text(), "draft");
    assert_eq!(editor.images().len(), 1);

    let mut editor = Editor::new();
    editor.insert_str("task");
    let message = editor.take();
    assert_eq!(message.text, "task");
    assert!(editor.is_empty());
    assert!(!editor.undo(), "send resets undo history");

    let mut editor = Editor::new();
    editor.insert_str("keep");
    editor.reset_history();
    assert_eq!(editor.text(), "keep", "session switch keeps the draft");
    assert!(!editor.undo(), "session switch resets undo history");

    let mut editor = Editor::new();
    editor.insert_str("command");
    editor.reset();
    assert!(editor.is_empty());
    assert!(!editor.undo(), "executed command clears and resets history");
}

#[test]
fn ctrl_c_clear_is_undoable_and_keeps_input_history() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("draft");
    handle_key(&mut app, &mut editor, ctrl(KeyCode::Char('c')));
    assert!(editor.is_empty());
    assert!(
        !app.history.is_empty(),
        "cleared draft enters input history"
    );
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('u')));
    assert_eq!(editor.text(), "draft", "the clear can be undone");
}

#[test]
fn send_resets_undo_through_handle_key() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("hello");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_some());
    assert_eq!(editor.text(), "hello", "draft remains until Accepted");
}

#[test]
fn history_recall_is_an_undoable_draft_replacement() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.history.push("recalled".to_owned());
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "recalled");
    assert_eq!(editor.cursor(), (0, 8));
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('u')));
    assert_eq!(editor.text(), "", "recall replacement is undoable");
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('r')));
    assert_eq!(editor.text(), "recalled");
}

#[test]
fn help_overlay_precedence_blocks_undo() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("keep");
    app.help_open = true;
    handle_key(&mut app, &mut editor, alt(KeyCode::Char('u')));
    assert_eq!(editor.text(), "keep", "help handles input before undo");
    assert!(app.help_open);
}

#[test]
fn history_is_hard_bounded_and_oversize_is_documented() {
    // Entry cap: five separate edits keep only the newest three pre-states.
    let mut editor = Editor::new();
    editor.set_history_limits(1024, 3);
    for _ in 0..5 {
        editor.insert_str("a");
    }
    let mut undos = 0;
    while editor.undo() {
        undos += 1;
        assert!(undos <= 3, "entry cap bounds undo depth");
    }
    assert_eq!(undos, 3, "three bounded entries remain");

    // Byte cap: retained text never exceeds the small budget.
    let mut editor = Editor::new();
    editor.set_history_limits(64, 100);
    for _ in 0..10 {
        editor.insert_str("0123456789");
    }
    assert!(
        editor.retained_bytes() <= 64,
        "combined undo+redo retained text obeys the byte cap"
    );

    // A live draft larger than the budget refuses undo, leaving it unchanged.
    let mut editor = Editor::new();
    editor.set_history_limits(16, 10);
    editor.insert_str("0123456789012345678901234567890123456789");
    let before = editor.text();
    assert!(!editor.undo(), "oversized live draft is unavailable");
    assert_eq!(editor.text(), before, "refusal leaves the draft unchanged");

    // An oversized pre-edit snapshot resets and skips history.
    editor.insert_typed('x');
    assert_eq!(editor.text().len(), 41, "the edit still applies");
    assert!(!editor.undo(), "oversized pre-state reset skipped history");
}

#[test]
fn empty_lines_count_against_the_text_bound() {
    // Newline delimiters count: a newline-only draft still consumes the budget.
    let mut editor = Editor::new();
    editor.set_history_limits(16, 100);
    for _ in 0..20 {
        editor.insert_newline();
    }
    assert!(
        editor.retained_bytes() <= 16,
        "newline bytes are counted in the retained total"
    );
    let before = editor.text();
    assert!(
        !editor.undo(),
        "a live newline-only draft larger than the budget cannot be undone"
    );
    assert_eq!(editor.text(), before);
}

#[test]
fn combined_byte_transfer_stays_bounded_across_undo_redo_and_branch() {
    let mut editor = Editor::new();
    editor.set_history_limits(24, 100);
    for chunk in ["aaaa", "bbbb", "cccc", "dddd", "eeee"] {
        editor.insert_str(chunk);
        assert!(editor.retained_bytes() <= 24, "edits stay within the cap");
    }
    // Undo/redo move the live draft between the stacks; the combined retained
    // text must stay within the cap at every transition.
    let mut undos = 0;
    while editor.undo() {
        undos += 1;
        assert!(editor.retained_bytes() <= 24, "undo transfer stays bounded");
        assert!(undos <= 5, "the byte cap bounds undo depth");
    }
    assert!(undos >= 1, "at least one bounded step remains");
    let mut redos = 0;
    while editor.redo() {
        redos += 1;
        assert!(editor.retained_bytes() <= 24, "redo transfer stays bounded");
    }
    assert_eq!(redos, undos, "redo restores every undone step");
    // Branch: undo then a new edit discards redo and stays bounded.
    assert!(editor.undo());
    editor.insert_str("z");
    assert!(editor.retained_bytes() <= 24, "branched edit stays bounded");
    assert!(!editor.redo(), "a branch discards redo");

    // Newline-only drafts count against the byte bound during transfers too.
    let mut editor = Editor::new();
    editor.set_history_limits(8, 100);
    for _ in 0..6 {
        editor.insert_newline();
        assert!(
            editor.retained_bytes() <= 8,
            "newline transfers stay bounded"
        );
    }
}

fn assert_undo_restores_preferred_column(width: usize, text: &str, ups: usize) {
    let mut control = Editor::new();
    let mut roundtrip = Editor::new();
    for editor in [&mut control, &mut roundtrip] {
        editor.insert_str(text);
        for _ in 0..ups {
            editor.move_visual_up(width);
        }
    }
    assert_eq!(control.cursor(), roundtrip.cursor());
    // Editing resets the preferred column; undo must restore the prior value.
    roundtrip.insert_str("Z");
    assert!(roundtrip.undo());
    assert_eq!(roundtrip.text(), control.text());
    assert_eq!(roundtrip.cursor(), control.cursor());
    for step in 0..2 {
        control.move_visual_up(width);
        roundtrip.move_visual_up(width);
        assert_eq!(
            roundtrip.cursor(),
            control.cursor(),
            "width {width}, step {step}: undo restores the preferred display column"
        );
    }
}

#[test]
fn undo_restores_preferred_display_column() {
    assert_undo_restores_preferred_column(40, "abcdefgh\nx\nabcdefgh", 1);
    // Narrow width exercises the soft-wrap fragment path too.
    assert_undo_restores_preferred_column(4, "abcdefgh\nx\nabcdefgh", 1);
}
