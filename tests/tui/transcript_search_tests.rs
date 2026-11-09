use super::*;
use crate::frontend::tui::transcript_search as search;
use unicode_width::UnicodeWidthStr;

fn hits(app: &App) -> Vec<search::SearchMatch> {
    app.transcript_search
        .as_ref()
        .map(|state| state.matches.clone())
        .unwrap_or_default()
}

fn cell_start(text: &str, needle: &str) -> usize {
    let index = text.find(needle).expect("needle in rendered row");
    UnicodeWidthStr::width(&text[..index])
}

fn open_on(app: &mut App, width: usize) {
    app.last_width = width;
    app.last_height = 8;
    search::open(app);
}

#[test]
fn empty_query_case_insensitive_and_no_match() {
    let mut app = App::new();
    app.push(assistant("Alpha BETA gamma"));
    open_on(&mut app, 40);
    assert!(search::active(&app));
    assert!(hits(&app).is_empty(), "an empty query indexes nothing");

    search::push_str(&mut app, "beta");
    assert_eq!(hits(&app).len(), 1, "lowercase query matches mixed case");
    assert!(!app.follow, "a hit jump stops follow");

    search::push_str(&mut app, "zzz");
    assert!(hits(&app).is_empty(), "appended query has no match");

    search::cancel(&mut app);
    assert!(!search::active(&app));
    assert!(app.follow);
    assert!(app.anchor.is_none());
}

#[test]
fn unicode_case_expansion_maps_back_to_original_cells() {
    let mut app = App::new();
    app.push(assistant("İstanbul café"));
    open_on(&mut app, 40);
    // U+0130 lowercases to "i" + combining dot above.
    search::push_str(&mut app, "i\u{307}stanbul");
    let found = hits(&app);
    assert_eq!(found.len(), 1, "expanding lowercase still matches");
    // Assistant body padding is three cells; the original "İstanbul" is eight.
    assert_eq!(found[0].start, 3);
    assert_eq!(found[0].end, 3 + 8);
}

#[test]
fn highlight_ranges_track_rendered_markdown_code_and_tables() {
    let markdown =
        "intro alpha\n\n```\nlet alpha = 1;\n```\n\n| V | W |\n| --- | --- |\n| ZEBRA | ok |\n";
    let mut app = App::new();
    app.push(assistant(markdown));

    open_on(&mut app, 80);
    search::push_str(&mut app, "zebra");
    let table_hit = hits(&app)
        .into_iter()
        .find(|item| item.key.record == 0)
        .expect("table cell match");
    let layout = view::record_layout(&mut app, 0, 80);
    let row = layout[table_hit.key.line].line.to_string();
    assert!(row.contains("ZEBRA"), "table row: {row}");
    assert_eq!(table_hit.start, cell_start(&row, "ZEBRA"));
    assert_eq!(table_hit.end, table_hit.start + 5);

    // Narrow widths fall back to a key/value rendering; the cell text is still
    // searchable and ranges are recomputed for the new rows.
    app.last_width = 11;
    search::refresh(&mut app, 11);
    let narrow = hits(&app)
        .into_iter()
        .find(|item| item.key.record == 0)
        .expect("narrow table fallback stays searchable");
    let fallback_row = view::record_layout(&mut app, 0, 11)[narrow.key.line]
        .line
        .to_string();
    assert!(
        fallback_row.contains("ZEBRA"),
        "fallback row: {fallback_row}"
    );
    assert_eq!(narrow.start, cell_start(&fallback_row, "ZEBRA"));

    search::cancel(&mut app);
    open_on(&mut app, 40);
    search::push_str(&mut app, "alpha");
    let code_hit = hits(&app)
        .into_iter()
        .find(|item| {
            view::record_layout(&mut app, 0, 40)[item.key.line]
                .line
                .to_string()
                .contains("let alpha")
        })
        .expect("code line match");
    let code_row = view::record_layout(&mut app, 0, 40)[code_hit.key.line]
        .line
        .to_string();
    assert_eq!(code_hit.start, cell_start(&code_row, "alpha"));
}

#[test]
fn narrow_wrapped_words_report_ranges_on_the_wrapped_row() {
    let mut app = App::new();
    app.push(assistant("0123456789 ZEBRA tail"));
    open_on(&mut app, 13);
    search::push_str(&mut app, "zebra");
    let found = hits(&app);
    assert_eq!(found.len(), 1);
    let row = view::record_layout(&mut app, 0, 13)[found[0].key.line]
        .line
        .to_string();
    assert!(row.contains("ZEBRA"), "wrapped row: {row}");
    assert_eq!(found[0].start, cell_start(&row, "ZEBRA"));
    assert_eq!(found[0].end, found[0].start + 5);
}

#[test]
fn navigation_wraps_and_esc_restores_follow_or_anchored_view() {
    let mut app = App::new();
    app.push(assistant("alpha one"));
    app.push(assistant("alpha two"));
    open_on(&mut app, 40);
    search::push_str(&mut app, "alpha");
    assert_eq!(hits(&app).len(), 2);
    assert_eq!(app.transcript_search.as_ref().unwrap().selected, 0);
    assert_eq!(app.anchor.unwrap().record, 0);

    search::step(&mut app, true);
    assert_eq!(app.transcript_search.as_ref().unwrap().selected, 1);
    assert_eq!(app.anchor.unwrap().record, 1);
    search::step(&mut app, true);
    assert_eq!(app.transcript_search.as_ref().unwrap().selected, 0, "wraps");
    search::step(&mut app, false);
    assert_eq!(
        app.transcript_search.as_ref().unwrap().selected,
        1,
        "wraps back"
    );

    search::cancel(&mut app);
    assert!(app.follow, "follow mode restored");
    assert!(app.anchor.is_none());

    // Anchored mode restores the exact saved source anchor.
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 1,
        offset: 5,
    });
    app.visual_anchor = None;
    open_on(&mut app, 40);
    search::push_str(&mut app, "alpha");
    search::step(&mut app, true);
    assert_eq!(app.anchor.map(|anchor| anchor.record), Some(1));
    assert_ne!(
        app.anchor,
        Some(Anchor {
            record: 1,
            offset: 5
        })
    );
    search::cancel(&mut app);
    assert!(!app.follow);
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 1,
            offset: 5
        })
    );
}

#[test]
fn search_keeps_draft_cursor_images_and_undo() {
    let mut app = App::new();
    app.push(assistant("find this phrase"));
    app.last_width = 40;
    app.last_height = 8;
    let mut editor = Editor::new();
    editor.insert_str("keep");
    editor.insert_str("\nmore");
    editor.insert_image(crate::input::ImageAttachment::new(
        "a.png",
        "data:image/png;base64,AA",
    ));
    let text = editor.text();
    let cursor = editor.cursor();
    let images = editor.images().len();

    handle_key(&mut app, &mut editor, key(KeyCode::F(3)));
    assert!(search::active(&app));
    for character in "find".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!search::active(&app));

    assert_eq!(editor.text(), text);
    assert_eq!(editor.cursor(), cursor);
    assert_eq!(editor.images().len(), images);
    assert!(editor.undo(), "prior draft edit is still undoable");
    assert!(editor.undo(), "the earlier typing step is still undoable");
    assert_eq!(editor.text(), "keep");
}

#[test]
fn busy_esc_closes_search_before_cancelling_the_task() {
    let mut app = App::new();
    app.push(assistant("alpha"));
    app.running = true;
    let mut editor = Editor::new();
    open_on(&mut app, 40);

    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel, "Esc closes the search first");
    assert!(!search::active(&app));
    assert!(app.running);

    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(outcome.cancel, "the next Esc cancels the running task");
}

#[test]
fn collapsed_activity_body_is_not_indexed_until_expanded() {
    let mut app = App::new();
    app.push(assistant("visible text"));
    app.push(Record::Activity {
        end: Some(3),
        expanded: false,
        failed: false,
        interrupted: false,
        manual: true,
    });
    app.push(Record::Reasoning {
        text: "HIDDENWORD body".to_owned(),
        expanded: true,
        auto_collapse: false,
    });
    app.push(assistant("tail"));
    open_on(&mut app, 40);
    search::push_str(&mut app, "hiddenword");
    assert!(hits(&app).is_empty(), "collapsed member body is hidden");

    app.toggle_record(1);
    // Reproduce a draw rebuilding the changed record's layout BEFORE search
    // refreshes: cache validity must not be treated as proof of freshness.
    let _ = view::record_layout(&mut app, 2, 40);
    search::refresh(&mut app, 40);
    assert_eq!(hits(&app).len(), 1, "expanded body becomes searchable");

    app.toggle_record(1);
    let _ = view::record_layout(&mut app, 2, 40);
    search::refresh(&mut app, 40);
    assert!(hits(&app).is_empty(), "collapsing removes hidden hits");
}

#[test]
fn streamed_and_final_text_refresh_even_after_layout_rebuild() {
    let mut app = App::new();
    app.push(assistant("early"));
    open_on(&mut app, 40);
    search::push_str(&mut app, "late");
    assert!(hits(&app).is_empty());

    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".to_owned(),
        delta: " late".to_owned(),
    });
    let _ = view::record_layout(&mut app, 1, 40);
    search::refresh(&mut app, 40);
    assert_eq!(hits(&app).len(), 1, "streamed text is found");

    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".to_owned(),
        text: "final late answer".to_owned(),
    });
    let _ = view::record_layout(&mut app, 1, 40);
    search::refresh(&mut app, 40);
    assert_eq!(hits(&app).len(), 1, "final text keeps the hit fresh");

    search::refresh(&mut app, 18);
    let found = hits(&app);
    assert_eq!(found.len(), 1, "resize rebuilds ranges");
    let row = view::record_layout(&mut app, 1, 18)[found[0].key.line]
        .line
        .to_string();
    assert!(row.contains("late"), "resized row: {row}");
}

#[test]
fn paste_charslash_and_f1_route_as_a_modal() {
    let mut app = App::new();
    app.push(assistant("alpha"));
    app.last_width = 40;
    app.last_height = 8;
    let mut editor = Editor::new();
    editor.insert_str("secret draft");
    open_on(&mut app, 40);

    handle_paste_text(&mut app, &mut editor, "pasted\ntext");
    assert_eq!(
        app.transcript_search.as_ref().unwrap().query,
        "pasted text",
        "paste folds newlines and never reaches the draft"
    );
    assert_eq!(editor.text(), "secret draft");

    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('v'), KeyModifiers::CONTROL),
    );
    assert!(outcome.paste_clipboard);
    assert!(search::active(&app));
    assert_eq!(editor.text(), "secret draft");

    handle_key(&mut app, &mut editor, key(KeyCode::Char('/')));
    assert!(app.transcript_search.as_ref().unwrap().query.ends_with('/'));
    assert_eq!(editor.text(), "secret draft");

    handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
    assert!(!search::active(&app));
    assert!(app.help_open);
}

#[test]
fn ctrl_f_keeps_character_motion_and_f3_requires_no_ctrl() {
    let mut app = App::new();
    app.push(assistant("alpha"));
    app.last_width = 40;
    app.last_height = 8;
    let mut editor = Editor::new();
    editor.insert_str("ab");
    editor.home();

    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('f'), KeyModifiers::CONTROL),
    );
    assert!(!outcome.paste_clipboard);
    assert!(!search::active(&app), "Ctrl+F is not search");
    assert_eq!(editor.cursor(), (0, 1), "Ctrl+F still moves right");

    handle_key(&mut app, &mut editor, key(KeyCode::F(3)));
    assert!(search::active(&app));
}

#[test]
fn whole_string_lowercase_matches_greek_final_sigma() {
    // "ΟΣ".to_lowercase() is "ος" (final sigma), unlike per-grapheme "οσ".
    for query in ["ΟΣ", "ος"] {
        let mut app = App::new();
        app.push(assistant("ΟΣ identical uppercase"));
        open_on(&mut app, 40);
        search::push_str(&mut app, query);
        let found = hits(&app);
        assert_eq!(found.len(), 1, "query {query} must match");
        assert_eq!((found[0].start, found[0].end), (3, 5));
    }
}

#[test]
fn literal_marker_glyph_in_record_text_is_searchable() {
    let mut app = App::new();
    app.push(assistant("▌literal content"));
    open_on(&mut app, 40);
    search::push_str(&mut app, "▌literal");
    let found = hits(&app);
    assert_eq!(found.len(), 1, "a real ▌ in content is not stripped");
    let row = view::record_layout(&mut app, 0, 40)[found[0].key.line]
        .line
        .to_string();
    assert_eq!(found[0].start, cell_start(&row, "▌literal"));
    assert_eq!(found[0].end, found[0].start + 8);
}

#[test]
fn commands_popup_is_suspended_while_search_owns_input() {
    use crate::frontend::tui::commands::Commands;

    let mut app = App::new();
    app.push(assistant("alpha"));
    app.last_width = 40;
    app.last_height = 8;
    let mut editor = Editor::new();
    editor.set_draft("/");
    let mut commands = Commands::default();
    commands.sync(&app, &editor);
    assert!(commands.key(&mut editor, key(KeyCode::Down)));

    // F3 opens search with the slash draft still in the hidden composer.
    handle_key(&mut app, &mut editor, key(KeyCode::F(3)));
    assert!(search::active(&app));
    commands.sync(&app, &editor);
    assert!(
        !commands.key(&mut editor, key(KeyCode::Tab)),
        "Tab must not complete the hidden draft"
    );
    assert!(
        !commands.key(&mut editor, key(KeyCode::Up)),
        "Up must not move a stale command popup"
    );
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.text(), "/", "routing preserved the exact draft");

    for character in "alpha".chars() {
        handle_key(&mut app, &mut editor, key(KeyCode::Char(character)));
    }
    handle_key(&mut app, &mut editor, key(KeyCode::F(3)));
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!search::active(&app));
    assert_eq!(
        editor.text(),
        "/",
        "Esc after navigation preserved the draft"
    );

    commands.sync(&app, &editor);
    assert!(
        commands.key(&mut editor, key(KeyCode::Tab)),
        "completion works again once search is closed"
    );
    assert_ne!(editor.text(), "/");
}

#[test]
fn composer_cursor_is_hidden_and_narrow_search_keeps_its_cursor() {
    let mut app = App::new();
    app.push(assistant("alpha"));
    app.last_width = 40;
    app.last_height = 8;
    let mut editor = Editor::new();
    editor.insert_str("draft");
    app.focus = Focus::Input;

    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let composer = terminal.get_cursor_position().unwrap();
    assert!(composer.y > 0, "composer owns the cursor normally");

    open_on(&mut app, 40);
    search::push_str(&mut app, "alpha");
    // A fresh backend shows whether the composer claimed the hardware cursor:
    // with search active nothing may call set_cursor_position.
    let mut search_terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    search_terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let search_cursor = search_terminal.get_cursor_position().unwrap();
    assert_eq!(
        (search_cursor.x, search_cursor.y),
        (0, 0),
        "the composer cursor is suppressed while search owns input"
    );

    // Very narrow strip: only the label, the query tail and the cursor glyph fit.
    let mut narrow = Terminal::new(TestBackend::new(6, 1)).unwrap();
    narrow
        .draw(|frame| {
            crate::frontend::tui::transcript_search::draw(frame, &app, frame.area());
        })
        .unwrap();
    let text = buffer_text(narrow.backend().buffer());
    assert!(text.contains("▌"), "query cursor stays visible: {text:?}");
    assert!(text.contains("S:") || text.contains("Search:"), "{text:?}");
}
