use super::*;
use crate::client::AgentEvent;
use crate::frontend::tui::editor::Editor;
use crate::frontend::tui::log;
use crate::frontend::tui::state::{Anchor, App, Focus, Record};
use crate::frontend::tui::view;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};
use serde_json::json;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn assistant(text: &str) -> Record {
    Record::Assistant {
        text: text.to_owned(),
    }
}

fn tool(call_id: &str) -> Record {
    Record::Tool {
        call_id: call_id.to_owned(),
        name: "bash".to_owned(),
        arguments: json!({"command": "echo hi"}).to_string(),
        output: None,
        expanded: false,
    }
}

fn buffer_text(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn separately_typed_combining_mark_is_one_grapheme() {
    let mut editor = Editor::new();
    editor.insert_str("e");
    editor.insert_str("\u{301}");
    assert_eq!(editor.text(), "e\u{301}");
    assert_eq!(editor.cursor(), (0, 1));
    editor.backspace();
    assert!(editor.is_empty());
}

#[test]
fn separately_typed_zwj_sequence_is_one_grapheme() {
    let mut editor = Editor::new();
    editor.insert_str("👨");
    editor.insert_str("\u{200d}");
    editor.insert_str("👩");
    assert_eq!(editor.cursor(), (0, 1));
    editor.backspace();
    assert!(editor.is_empty());
}

#[test]
fn multiline_paste_keeps_lines_and_does_not_submit() {
    let mut editor = Editor::new();
    editor.insert_str("第一行\n第二行\n第三行");
    assert_eq!(editor.text().split('\n').count(), 3);
    assert_eq!(editor.text(), "第一行\n第二行\n第三行");
}

#[test]
fn backspace_at_line_start_merges_the_previous_line() {
    let mut editor = Editor::new();
    editor.insert_str("ab\ncd");
    editor.home();
    editor.backspace();
    assert_eq!(editor.text(), "abcd");
    assert_eq!(editor.cursor(), (0, 2));
}

#[test]
fn delete_at_line_end_merges_the_next_line() {
    let mut editor = Editor::new();
    editor.insert_str("ab\ncd");
    editor.move_visual_up(80);
    editor.end();
    editor.delete();
    assert_eq!(editor.text(), "abcd");
    assert_eq!(editor.cursor(), (0, 2));
}

#[test]
fn tool_finished_attaches_to_the_matching_call_id_only() {
    let mut app = App::new();
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".to_owned(),
        name: "bash".to_owned(),
        arguments: "{}".to_owned(),
    });
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c2".to_owned(),
        name: "bash".to_owned(),
        arguments: "{}".to_owned(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c2".to_owned(),
        name: "bash".to_owned(),
        output: json!({"exit_code": 0}),
    });
    match &app.records[0] {
        Record::Tool { output, .. } => assert!(output.is_none()),
        other => panic!("expected tool, got {other:?}"),
    }
    match &app.records[1] {
        Record::Tool { output, .. } => assert_eq!(output, &Some(json!({"exit_code": 0}))),
        other => panic!("expected tool, got {other:?}"),
    }
}

#[test]
fn reading_position_survives_new_output_and_earlier_expansion() {
    let mut app = App::new();
    app.push(assistant("first"));
    app.push(tool("c1"));
    app.push(assistant("anchor text"));
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 2,
        offset: 0,
    });
    app.last_width = 40;
    app.last_height = 4;

    app.apply_event(AgentEvent::ResponseText {
        item_id: "message".to_owned(),
        text: "late".to_owned(),
    });
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 2,
            offset: 0
        })
    );
    assert!(!app.follow);

    if let Record::Tool { expanded, .. } = &mut app.records[1] {
        *expanded = true;
    }
    let lines = view::visible_lines(&mut app, 40, 4);
    let text = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("anchor text"));
    assert!(!text.contains("first"));
}

#[test]
fn anchor_tracks_source_offset_across_widths() {
    let text = "alpha beta gamma delta\n\nepsilon zeta eta theta\n\niota kappa lambda";
    let mut app = App::new();
    app.push(assistant(text));
    let byte = text.find("epsilon").unwrap();
    let offset = byte;
    app.follow = false;
    app.anchor = Some(Anchor { record: 0, offset });
    app.last_height = 3;
    for width in [16usize, 34] {
        let lines = view::visible_lines(&mut app, width, 3);
        let joined = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("epsilon"), "width {width}: {joined}");
    }
}

#[test]
fn submit_while_reading_does_not_resume_follow() {
    let mut app = App::new();
    app.push(assistant("one"));
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    app.submit("next".to_owned());
    assert!(!app.follow);
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        })
    );
    assert!(app.running);
}

#[test]
fn end_resumes_follow() {
    let mut app = App::new();
    app.push(assistant("one"));
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    app.end();
    assert!(app.follow);
    assert!(app.anchor.is_none());
}

#[test]
fn history_larger_than_u16_does_not_overflow() {
    let mut app = App::new();
    for index in 0..70_000 {
        app.push(assistant(&format!("line {index}")));
    }
    app.last_width = 40;
    app.last_height = 1;
    view::page_up(&mut app, 1);
    let anchor = app.anchor.expect("page up sets an anchor");
    // The last record now has a content line plus a trailing blank; the first
    // PageUp lands on its content, and the second crosses to the previous
    // record. Record 69_999 is already beyond u16::MAX.
    assert_eq!(anchor.record, 69_999);
    assert_eq!(anchor.offset, 0);
    view::page_up(&mut app, 1);
    let anchor = app.anchor.expect("page up sets an anchor");
    assert_eq!(anchor.record, 69_998);
    assert!(!app.follow);
}

#[test]
fn page_down_to_the_bottom_resumes_follow() {
    let mut app = App::new();
    app.push(assistant("a"));
    app.push(assistant("b"));
    app.last_width = 40;
    app.last_height = 2;
    view::page_up(&mut app, 1);
    assert!(!app.follow);
    view::page_down(&mut app, 10);
    assert!(app.follow);
    assert!(app.anchor.is_none());
}

#[test]
fn transcript_selection_scrolls_the_selected_record_into_view() {
    let mut app = App::new();
    for index in 0..5 {
        app.push(assistant(&format!("record {index}")));
    }
    app.last_width = 40;
    app.last_height = 2;
    view::focus_transcript(&mut app);
    assert_eq!(app.selected, 4);
    view::select(&mut app, -1);
    assert_eq!(app.selected, 3);
    assert!(!app.follow);
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 3,
            offset: 0
        })
    );
    let lines = view::visible_lines(&mut app, 40, 2);
    assert!(lines[0].to_string().contains("record 3"));
}

#[test]
fn tool_status_classifies_failures() {
    assert_eq!(view::tool_status(None), ("…", false));
    assert_eq!(
        view::tool_status(Some(&json!({"exit_code": 0}))),
        ("✓", false)
    );
    assert!(view::tool_status(Some(&json!({"exit_code": 7}))).1);
    assert!(view::tool_status(Some(&json!({"timed_out": true}))).1);
    assert!(view::tool_status(Some(&json!({"error": "not found"}))).1);
}

#[test]
fn tool_headline_prefers_command_then_path() {
    assert_eq!(
        view::tool_headline("bash", r#"{"command":"ls -la"}"#, None),
        "Run ls -la"
    );
    assert_eq!(
        view::tool_headline("read", r#"{"path":"src/main.rs"}"#, None),
        "Read src/main.rs"
    );
}

#[test]
fn editor_wraps_cjk_at_cell_width() {
    let mut editor = Editor::new();
    editor.insert_str("你好世");
    let layout = editor.layout(4);
    let lines: Vec<String> = layout.lines.iter().map(|line| line.text.clone()).collect();
    assert_eq!(lines, vec!["你好".to_owned(), "世".to_owned()]);
}

#[test]
fn enter_sends_the_draft_when_idle() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("hello");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(
        outcome.send.map(|message| message.text),
        Some("hello".to_owned())
    );
    assert_eq!(editor.text(), "hello", "draft remains until Accepted");
}

#[test]
fn enter_while_running_keeps_the_draft() {
    let mut app = App::new();
    app.running = true;
    let mut editor = Editor::new();
    editor.insert_str("draft");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none());
    assert_eq!(editor.text(), "draft");
    assert!(app.status.is_some());
}

#[test]
fn alt_enter_inserts_a_newline() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("a");
    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Enter, KeyModifiers::ALT),
    );
    assert!(outcome.send.is_none());
    assert_eq!(editor.text().split('\n').count(), 2);
}

#[test]
fn ctrl_c_quits_when_idle_and_waits_while_running() {
    let mut app = App::new();
    let mut editor = Editor::new();
    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(outcome.quit);

    app.running = true;
    let outcome = handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(!outcome.quit);
    assert!(app.quit_after_turn);
}

#[test]
fn tab_toggles_focus() {
    let mut app = App::new();
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Transcript);
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Input);
}

#[test]
fn cursor_wraps_to_a_new_line_when_input_fills_the_width() {
    let mut editor = Editor::new();
    editor.insert_str(&"a".repeat(10));
    let layout = editor.layout(10);
    let lines: Vec<String> = layout.lines.iter().map(|line| line.text.clone()).collect();
    assert_eq!(lines, vec!["aaaaaaaaaa".to_owned(), String::new()]);
    assert_eq!((layout.cursor_line, layout.cursor_cell), (1, 0));
}

#[test]
fn selected_assistant_record_shows_a_marker() {
    let backend = TestBackend::new(30, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new();
    app.push(assistant("hello"));
    app.focus = Focus::Transcript;
    app.selected = 0;
    let editor = Editor::new();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("▌  hello"), "{text}");
}

#[test]
fn test_backend_draws_records_and_editor() {
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new();
    app.push(Record::User {
        text: "你好".to_owned(),
        images: Vec::new(),
    });
    app.push(assistant("world"));
    let mut editor = Editor::new();
    editor.insert_str("draft");
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.replace(' ', "").contains("你好"), "{text}");
    assert!(text.contains("world"));
    assert!(text.contains("draft"));
}

#[test]
fn expanded_tool_layout_offsets_are_monotonic() {
    let mut app = App::new();
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".to_owned(),
        name: "bash".to_owned(),
        arguments: json!({"command": "echo hi"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".to_owned(),
        name: "bash".to_owned(),
        output: json!({"output": "hi\n", "exit_code": 0}),
    });
    if let Record::Tool { expanded, .. } = &mut app.records[0] {
        *expanded = true;
    }
    let lines = view::record_layout(&mut app, 0, 40);
    assert!(lines.len() > 2, "expected headline, args and result");
    assert!(
        lines
            .windows(2)
            .all(|pair| pair[0].source_start <= pair[1].source_start)
    );
}

#[test]
fn spilled_log_keeps_result_metadata_visible() {
    let path = std::env::temp_dir().join(format!("saki-log-meta-{}.txt", std::process::id()));
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
            "exit_code": 7,
            "timed_out": false,
            "truncated": true,
            "output_path": path.to_string_lossy(),
        }),
    });
    if let Record::Tool { expanded, .. } = &mut app.records[0] {
        *expanded = true;
    }
    let lines = view::record_layout(&mut app, 0, 60);
    let text = lines
        .iter()
        .map(|visual| visual.line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("exit=7"), "{text}");
    assert!(text.contains("truncated=true"), "{text}");
    assert!(text.contains("line one"), "{text}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn log_page_pages_by_byte_range_to_eof() {
    let path = std::env::temp_dir().join(format!("saki-log-page-{}.txt", std::process::id()));
    let mut content = String::new();
    for index in 0..5000 {
        content.push_str(&format!("line {index}\n"));
    }
    std::fs::write(&path, &content).unwrap();
    let path = path.to_string_lossy().to_string();
    let mut start = 0u64;
    let mut pages = 0;
    loop {
        let page = log::page(&path, start);
        assert!(page.next > start || page.eof, "page stalled at {start}");
        pages += 1;
        if page.eof {
            break;
        }
        start = page.next;
        assert!(pages < 100, "too many pages");
    }
    assert!(pages > 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn long_line_page_advances_and_is_bounded() {
    let path = std::env::temp_dir().join(format!("saki-log-long-{}.txt", std::process::id()));
    std::fs::write(&path, "y".repeat(100_000)).unwrap();
    let path = path.to_string_lossy().to_string();
    let first = log::page(&path, 0);
    assert!(first.next > 0);
    assert!(first.text.len() < 10_000);
    let second = log::page(&path, first.next);
    assert!(second.next > first.next);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn copy_source_and_code_source() {
    let mut app = App::new();
    app.push(Record::Assistant {
        text: "text\n\n```rust\nlet x = 1;\n```\n".to_owned(),
    });
    app.selected = 0;
    assert_eq!(
        view::copy_source(&app).as_deref(),
        Some("text\n\n```rust\nlet x = 1;\n```\n")
    );
    assert_eq!(
        view::copy_code_source(&app).as_deref(),
        Some("let x = 1;\n")
    );
    app.push(tool("c1"));
    app.selected = 1;
    assert_eq!(view::copy_source(&app).as_deref(), Some("echo hi"));
}

#[test]
fn y_key_copies_the_selected_source() {
    let mut app = App::new();
    app.push(assistant("hello"));
    app.focus = Focus::Transcript;
    app.selected = 0;
    let mut editor = Editor::new();
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Char('y')));
    assert_eq!(outcome.copy.as_deref(), Some("hello"));
}

#[test]
fn exit_code_null_is_a_failure() {
    assert!(view::tool_status(Some(&json!({"exit_code": null}))).1);
}

#[test]
fn selected_marker_overlays_without_stale_cache() {
    let mut app = App::new();
    app.push(assistant("one"));
    app.push(assistant("two"));
    app.focus = Focus::Transcript;
    app.selected = 0;
    let first = view::record_layout(&mut app, 0, 40);
    assert!(first[0].line.to_string().starts_with('▌'));
    app.selected = 1;
    let second = view::record_layout(&mut app, 1, 40);
    assert!(second[0].line.to_string().starts_with('▌'));
    let first_again = view::record_layout(&mut app, 0, 40);
    assert!(!first_again[0].line.to_string().starts_with('▌'));
}

#[test]
fn log_page_does_not_claim_eof_before_the_file_ends() {
    let path = std::env::temp_dir().join(format!("saki-log-lines-{}.txt", std::process::id()));
    let mut content = String::new();
    for index in 0..1000 {
        content.push_str(&format!("line {index}\n"));
    }
    content.push_str("tail\n");
    std::fs::write(&path, content.as_bytes()).unwrap();
    let path = path.to_string_lossy().to_string();
    let mut start = 0u64;
    let mut seen_tail = false;
    loop {
        let page = log::page(&path, start);
        if page.text.contains("tail") {
            seen_tail = true;
        }
        if page.eof {
            assert_eq!(page.next, content.len() as u64);
            break;
        }
        assert!(page.next > start, "page must advance");
        start = page.next;
    }
    assert!(seen_tail, "content after the first page must be reachable");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn log_page_offsets_use_raw_bytes_with_invalid_utf8() {
    let bytes: &[u8] = b"ok\n\xFF\xFE bad \xF0\x9F\x98\x80\nend\n";
    let path = std::env::temp_dir().join(format!("saki-log-bytes-{}.txt", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let path = path.to_string_lossy().to_string();
    let mut start = 0u64;
    loop {
        let page = log::page(&path, start);
        if page.eof {
            assert_eq!(page.next, bytes.len() as u64);
            break;
        }
        assert!(page.next > start, "page must advance");
        start = page.next;
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn synthetic_markdown_decorations_keep_offsets_monotonic_and_paging_progresses() {
    let text =
        "intro paragraph\n\n---\n\n- first item\n  - nested one\n  - nested two\n\ntail target";
    // With the 3-cell body margin, width 7 and 6 force the 5-cell rule and
    // the nested list to wrap; width 12 keeps the rule on one line. The old
    // rendered-length offset accounting overflowed the raw range and let later
    // offsets move backwards at the narrower widths.
    for width in [6usize, 7, 12] {
        let mut app = App::new();
        app.push(assistant(text));
        app.last_width = width;
        app.last_height = 3;
        let layout = view::record_layout(&mut app, 0, width);
        assert!(!layout.is_empty());
        assert!(
            layout
                .windows(2)
                .all(|pair| pair[0].source_start <= pair[1].source_start),
            "width {width}: source offsets must not go backwards: {:?}",
            layout
                .iter()
                .map(|visual| visual.source_start)
                .collect::<Vec<_>>()
        );
        assert!(
            layout
                .iter()
                .all(|visual| visual.source_start <= text.len()),
            "width {width}: source offsets must stay inside the raw text"
        );

        app.follow = false;
        app.anchor = Some(Anchor {
            record: 0,
            offset: layout.last().unwrap().source_start,
        });
        let top_key = crate::frontend::tui::selection::RowKey { record: 0, line: 0 };
        let mut steps = 0;
        loop {
            let before_offset = app.anchor.unwrap().offset;
            let before_row = app.visual_anchor.map(|visual| visual.key);
            view::page_up(&mut app, 1);
            let after = app.visual_anchor.map(|visual| visual.key);
            let after_offset = app.anchor.unwrap().offset;
            assert!(
                after_offset <= before_offset,
                "width {width}: offsets must not increase: {before_offset} -> {after_offset}"
            );
            if let (Some(before_row), Some(after)) = (before_row, after) {
                assert!(
                    after < before_row,
                    "width {width}: page up stalled at {before_row:?}"
                );
            }
            assert!(
                after.is_some(),
                "width {width}: page up must set an exact row"
            );
            if after == Some(top_key) {
                break;
            }
            steps += 1;
            assert!(steps < 500, "width {width}: page up did not reach the top");
        }
        assert_eq!(app.anchor.unwrap().offset, 0, "width {width}: top reached");
        let top = view::visible_lines(&mut app, width, 3);
        assert!(!top.is_empty(), "width {width}: top view has content");

        for _ in 0..500 {
            if app.follow {
                break;
            }
            view::page_down(&mut app, 1);
        }
        assert!(
            app.follow,
            "width {width}: page down did not return to follow mode"
        );
    }
}

fn visual_blank(visual: &crate::frontend::tui::state::VisualLine) -> bool {
    visual.line.to_string().trim().is_empty()
}

#[test]
fn messages_have_one_blank_line_and_tools_stay_compact() {
    let mut app = App::new();
    app.push(Record::User {
        text: "ask".to_owned(),
        images: Vec::new(),
    });
    app.push(assistant("answer"));
    for call_id in ["c1", "c2"] {
        app.apply_event(AgentEvent::ToolStarted {
            call_id: call_id.to_owned(),
            name: "bash".to_owned(),
            arguments: "{}".to_owned(),
        });
        app.apply_event(AgentEvent::ToolFinished {
            call_id: call_id.to_owned(),
            name: "bash".to_owned(),
            output: json!({"exit_code": 0}),
        });
    }
    app.push(assistant("done"));

    let user = view::record_layout(&mut app, 0, 40);
    assert!(user.len() >= 3, "user block has vertical padding");
    assert!(visual_blank(&user[0]));
    assert!(visual_blank(user.last().unwrap()));

    let answer = view::record_layout(&mut app, 1, 40);
    assert!(visual_blank(answer.last().unwrap()));
    assert!(!visual_blank(&answer[answer.len() - 2]));

    for index in [2, 3] {
        let tool = view::record_layout(&mut app, index, 40);
        assert!(!visual_blank(tool.last().unwrap()), "tools stay compact");
    }
}

#[test]
fn user_block_fills_rows_with_background_and_right_padding() {
    let width = 40u16;
    let backend = TestBackend::new(width, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new();
    app.theme = crate::frontend::tui::theme::Theme::light();
    app.push(Record::User {
        text: "hello there".to_owned(),
        images: Vec::new(),
    });
    let editor = Editor::new();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let row = rows
        .iter()
        .position(|line| line.contains("hello there"))
        .expect("user text visible");
    for x in 0..width {
        assert_eq!(
            buffer.cell((x, row as u16)).unwrap().bg,
            app.theme.user_bg,
            "cell {x} of the user row must share the block background"
        );
    }
    // One cell of right inner padding keeps the text off the right edge.
    assert_eq!(buffer.cell((width - 1, row as u16)).unwrap().symbol(), " ");
}

#[test]
fn user_block_marker_lands_on_the_content_line() {
    let backend = TestBackend::new(30, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new();
    app.push(assistant("first"));
    app.push(Record::User {
        text: "picked".to_owned(),
        images: Vec::new(),
    });
    app.focus = Focus::Transcript;
    app.selected = 1;
    let editor = Editor::new();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("▌› picked"), "{text}");
}

#[test]
fn user_block_page_up_crosses_to_the_previous_record() {
    let mut app = App::new();
    app.push(assistant("before"));
    app.push(Record::User {
        text: "middle".to_owned(),
        images: Vec::new(),
    });
    app.push(assistant("after"));
    app.last_width = 30;
    app.last_height = 3;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 1,
        offset: 0,
    });
    // Exact visual stepping visits the leading padding row first; the next
    // step crosses to the previous record without skipping any row.
    view::page_up(&mut app, 1);
    assert_eq!(app.anchor.unwrap().record, 1);
    assert_eq!(
        app.anchor.unwrap().offset,
        view::record_layout(&mut app, 1, 30)[0].source_start
    );
    view::page_up(&mut app, 1);
    assert_eq!(
        app.anchor.unwrap().record,
        0,
        "a second page up crosses to the previous record"
    );
}

#[test]
fn short_assistant_marker_lands_on_the_content_line() {
    let mut app = App::new();
    app.push(assistant("hello"));
    app.focus = Focus::Transcript;
    app.selected = 0;
    let lines = view::record_layout(&mut app, 0, 30);
    let marked = lines
        .iter()
        .find(|visual| visual.line.to_string().starts_with('▌'))
        .expect("selected marker");
    assert!(marked.line.to_string().contains("hello"), "{}", marked.line);
}

#[test]
fn markdown_blocks_get_one_blank_line_and_items_stay_compact() {
    let text = "intro\n\n## Head\n\n- one\n- two\n\n```\ncode line\n```\n\nclosing";
    let rendered: String = markdown::segments(text)
        .into_iter()
        .map(|segment| segment.text)
        .collect();
    assert!(rendered.contains("intro\n\n"), "{rendered:?}");
    assert!(rendered.contains("Head\n\n"), "{rendered:?}");
    assert!(rendered.contains("• one\n• two"), "{rendered:?}");
    assert!(rendered.contains("two\n\n"), "{rendered:?}");
    assert!(rendered.contains("code line\n\n"), "{rendered:?}");
    assert!(
        !rendered.contains("code line\n\n\n"),
        "code block must be followed by exactly one blank line: {rendered:?}"
    );
    assert!(!rendered.starts_with('\n'), "{rendered:?}");
}

#[test]
fn list_item_starting_with_inline_code_keeps_its_bullet() {
    let rendered: String = markdown::segments("- `read` the file")
        .into_iter()
        .map(|segment| segment.text)
        .collect();
    assert!(rendered.starts_with("• read"), "{rendered:?}");
}

#[test]
fn list_only_message_has_no_leading_blank() {
    let rendered: String = markdown::segments("- alpha\n- beta")
        .into_iter()
        .map(|segment| segment.text)
        .collect();
    assert!(rendered.starts_with("• alpha"), "{rendered:?}");
}

#[test]
fn anchor_zero_shows_the_first_paragraph_not_a_blank() {
    let text = "first paragraph\n\nsecond paragraph with epsilon";
    let mut app = App::new();
    app.push(assistant(text));
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    let lines = view::visible_lines(&mut app, 40, 3);
    let joined = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("first paragraph"), "{joined}");
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn line_index(layout: &[crate::frontend::tui::state::VisualLine], offset: usize) -> usize {
    let mut chosen = 0;
    for (index, visual) in layout.iter().enumerate() {
        if visual.source_start <= offset {
            chosen = index;
        } else {
            break;
        }
    }
    chosen
}

fn code_fixture(lines: usize) -> String {
    let mut text = String::from("```\n");
    for index in 0..lines {
        text.push_str(&format!("{index:02} line\n"));
    }
    text.push_str("```");
    text
}

#[test]
fn paragraph_spacing_does_not_shift_wrapped_text_source_offsets() {
    let text = "abcdefghij\n\nsecond paragraph";
    let mut app = App::new();
    app.push(assistant(text));
    let layout = view::record_layout(&mut app, 0, 9);
    let continuation = layout
        .iter()
        .find(|line| line.line.to_string().contains("ghij"))
        .expect("first paragraph wraps after six body cells");
    assert_eq!(continuation.source_start, text.find("ghij").unwrap());
}

#[test]
fn transcript_up_scrolls_one_line_and_shift_up_selects() {
    let mut app = App::new();
    app.push(assistant(&code_fixture(30)));
    app.push(assistant("second record"));
    app.push(assistant("third record"));
    app.last_width = 40;
    app.last_height = 3;
    app.focus = Focus::Transcript;
    app.follow = false;
    let layout = view::record_layout(&mut app, 0, 40);
    let start = 15;
    app.anchor = Some(Anchor {
        record: 0,
        offset: layout[start].source_start,
    });
    app.selected = 2;
    let mut editor = Editor::new();

    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(
        line_index(&layout, app.anchor.unwrap().offset),
        start - 1,
        "plain Up scrolls exactly one line"
    );
    assert_eq!(app.selected, 2, "plain Up keeps the record selection");

    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Up, KeyModifiers::SHIFT),
    );
    assert_eq!(app.selected, 1, "Shift+Up moves the record selection");
}

#[test]
fn transcript_down_crosses_records_and_end_resumes_follow() {
    let mut app = App::new();
    app.push(assistant(&code_fixture(20)));
    app.push(assistant(&code_fixture(20)));
    app.last_width = 40;
    app.last_height = 3;
    app.focus = Focus::Transcript;
    app.follow = false;
    let layout = view::record_layout(&mut app, 0, 40);
    let start = layout.len() - 1;
    app.anchor = Some(Anchor {
        record: 0,
        offset: layout[start].source_start,
    });
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Down));
    let anchor = app.anchor.expect("Down crosses into the next record");
    assert_eq!(anchor.record, 1);
    handle_key(&mut app, &mut editor, key(KeyCode::End));
    assert!(app.follow, "End resumes following");
    assert!(app.anchor.is_none());
}

#[test]
fn pgup_uses_the_viewport_height_not_a_fixed_step() {
    let mut app = App::new();
    app.push(assistant(&code_fixture(30)));
    app.last_width = 40;
    app.last_height = 4;
    app.focus = Focus::Transcript;
    app.follow = false;
    let layout = view::record_layout(&mut app, 0, 40);
    let start = 20;
    app.anchor = Some(Anchor {
        record: 0,
        offset: layout[start].source_start,
    });
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::PageUp));
    assert_eq!(
        line_index(&layout, app.anchor.unwrap().offset),
        start - 3,
        "PgUp steps by last_height - 1"
    );
}

#[test]
fn alt_up_down_scrolls_without_moving_the_input_cursor() {
    let mut app = App::new();
    app.push(assistant(&code_fixture(30)));
    app.last_width = 40;
    app.last_height = 3;
    app.focus = Focus::Input;
    app.follow = false;
    let layout = view::record_layout(&mut app, 0, 40);
    let start = 10;
    app.anchor = Some(Anchor {
        record: 0,
        offset: layout[start].source_start,
    });
    let mut editor = Editor::new();
    editor.insert_str("hello\nworld");
    let cursor = editor.cursor();

    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Up, KeyModifiers::ALT),
    );
    assert_eq!(line_index(&layout, app.anchor.unwrap().offset), start - 1);
    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Down, KeyModifiers::ALT),
    );
    assert_eq!(line_index(&layout, app.anchor.unwrap().offset), start);
    assert_eq!(
        editor.cursor(),
        cursor,
        "Alt+Up/Down never touch the input cursor"
    );
}

#[test]
fn input_up_down_still_edits_and_repeat_is_accepted_release_ignored() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("one\ntwo");
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_eq!(editor.cursor().0, 0, "Input Up still moves the cursor");
    assert_eq!(editor.text(), "one\ntwo");

    app.push(assistant(&code_fixture(10)));
    app.focus = Focus::Transcript;
    app.last_width = 40;
    app.last_height = 3;
    let before = app.anchor;

    let mut release = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    handle_key(&mut app, &mut editor, release);
    assert_eq!(app.anchor, before, "Release events are ignored");

    let mut repeat = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    repeat.kind = KeyEventKind::Repeat;
    handle_key(&mut app, &mut editor, repeat);
    assert!(!app.follow, "Repeat events scroll");
}

#[test]
fn ctrl_home_and_end_jump_conversation_from_any_focus() {
    let mut app = App::new();
    app.push(assistant("one"));
    app.push(assistant("two"));
    app.last_width = 40;
    app.last_height = 3;
    app.focus = Focus::Input;
    let mut editor = Editor::new();

    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::End, KeyModifiers::CONTROL),
    );
    assert!(app.follow, "Ctrl+End follows the latest output");
    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Home, KeyModifiers::CONTROL),
    );
    assert!(!app.follow, "Ctrl+Home jumps to the conversation start");
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        })
    );
}

#[test]
fn mouse_wheel_scrolls_exactly_three_lines_and_respects_pane_bounds() {
    let mut app = App::new();
    app.push(assistant(&code_fixture(30)));
    app.last_width = 40;
    app.last_height = 4;
    app.transcript_area = Rect::new(0, 0, 40, 4);
    app.focus = Focus::Input;
    app.follow = false;
    let layout = view::record_layout(&mut app, 0, 40);
    let start = 15;
    app.anchor = Some(Anchor {
        record: 0,
        offset: layout[start].source_start,
    });
    let mut editor = Editor::new();
    editor.insert_str("draft");

    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp, 10, 2));
    assert_eq!(
        line_index(&layout, app.anchor.unwrap().offset),
        start - 3,
        "wheel scrolls exactly 3 lines"
    );
    assert_eq!(app.focus, Focus::Input, "mouse must not steal focus");
    assert_eq!(editor.text(), "draft", "mouse must not touch the draft");

    // column == width, outside the pane, and non-wheel events are ignored.
    let anchor = app.anchor;
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp, 40, 2));
    assert_eq!(app.anchor, anchor, "column == width is outside");
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp, 10, 4));
    assert_eq!(app.anchor, anchor, "row == pane bottom is outside");
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp, 10, 20));
    assert_eq!(app.anchor, anchor, "footer is ignored");
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::Moved, 10, 2));
    assert_eq!(app.anchor, anchor, "non-wheel events are ignored");

    // The rightmost in-bounds column is accepted and wheel down returns.
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollDown, 39, 2));
    assert_eq!(
        line_index(&layout, app.anchor.unwrap().offset),
        start,
        "wheel down 3 returns"
    );
    assert!(!app.follow);
}

#[test]
fn mouse_wheel_in_log_clamps_to_the_last_line_and_keeps_page_range() {
    let path = std::env::temp_dir().join(format!("saki-scroll-log-{}.txt", std::process::id()));
    let mut content = String::new();
    for index in 0..80 {
        content.push_str(&format!("line {index}\n"));
    }
    std::fs::write(&path, &content).unwrap();
    let file_len = std::fs::metadata(&path).unwrap().len();
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
            "output": "x",
            "exit_code": 0,
            "output_path": path.to_string_lossy(),
        }),
    });
    app.selected = 0;
    assert!(log::open(&mut app));
    app.last_width = 40;
    app.last_height = 10;
    app.transcript_area = Rect::new(0, 0, 40, 10);

    {
        let view = app.log_view.as_ref().unwrap();
        assert_eq!(view.offset, 0);
        assert_eq!(view.next, file_len, "the whole file is one page");
        assert!(view.eof);
    }

    for _ in 0..100 {
        let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollDown, 5, 3));
    }
    {
        let view = app.log_view.as_ref().unwrap();
        assert_eq!(view.scroll, 79, "clamps to the last display line");
        assert_eq!(view.offset, 0, "in-page wheel must not move the file page");
        assert_eq!(view.next, file_len);
        assert!(view.eof);
    }

    for _ in 0..100 {
        let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp, 5, 3));
    }
    assert_eq!(app.log_view.as_ref().unwrap().scroll, 0);
    assert_eq!(app.focus, Focus::Input);
    let _ = std::fs::remove_file(&path);
}

fn body_column(text: &str) -> usize {
    match text.strip_prefix('▌') {
        Some(rest) => 1 + rest.chars().take_while(|c| *c == ' ').count(),
        None => text.chars().take_while(|c| *c == ' ').count(),
    }
}

#[test]
fn assistant_lines_share_a_three_cell_left_margin() {
    let mut app = App::new();
    app.push(assistant("alpha beta gamma delta epsilon zeta eta theta"));
    app.last_width = 20;
    let layout = view::record_layout(&mut app, 0, 20);
    let content: Vec<&crate::frontend::tui::state::VisualLine> = layout
        .iter()
        .filter(|visual| !visual_blank(visual))
        .collect();
    assert!(content.len() >= 2, "text should wrap at width 20");
    for visual in &content {
        let text = visual.line.to_string();
        assert_eq!(
            body_column(&text),
            3,
            "every assistant line body starts at column 3: {text:?}"
        );
    }
}

#[test]
fn assistant_selection_marker_keeps_the_body_column() {
    let mut app = App::new();
    app.push(assistant("alpha beta gamma delta epsilon zeta eta theta"));
    app.focus = Focus::Transcript;
    app.selected = 0;
    app.last_width = 20;
    let layout = view::record_layout(&mut app, 0, 20);
    let content: Vec<&crate::frontend::tui::state::VisualLine> = layout
        .iter()
        .filter(|visual| !visual_blank(visual))
        .collect();
    assert!(content[0].line.to_string().starts_with('▌'));
    for visual in &content {
        assert_eq!(body_column(&visual.line.to_string()), 3);
    }
}

#[test]
fn assistant_tiny_width_drops_the_left_margin() {
    let mut app = App::new();
    app.push(assistant("hello"));
    let layout = view::record_layout(&mut app, 0, 3);
    let first = layout[0].line.to_string();
    assert!(
        !first.starts_with(' '),
        "tiny width drops the padding: {first:?}"
    );
}

#[test]
fn assistant_body_aligns_with_the_user_body_column() {
    let width = 40u16;
    let backend = TestBackend::new(width, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new();
    app.push(Record::User {
        text: "hello".to_owned(),
        images: Vec::new(),
    });
    app.push(assistant("world"));
    let editor = Editor::new();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let user_row = rows.iter().position(|line| line.contains("hello")).unwrap();
    let assistant_row = rows.iter().position(|line| line.contains("world")).unwrap();
    let column =
        |text: &str, needle: &str| text.find(needle).map(|byte| text[..byte].chars().count());
    assert_eq!(column(&rows[user_row], "hello"), Some(3));
    assert_eq!(column(&rows[assistant_row], "world"), Some(3));
}

mod images;
mod input;

mod native_input;
mod pty;
mod semantic_colors;
mod style;

mod theme_auto;

mod ux;

mod selection;

mod sessions;

mod commands;

fn user_record(text: &str) -> Record {
    Record::User {
        text: text.to_owned(),
        images: Vec::new(),
    }
}

fn ctrl_shift(code: KeyCode) -> KeyEvent {
    key_with(code, KeyModifiers::CONTROL | KeyModifiers::SHIFT)
}

fn feedback(app: &App) -> &str {
    app.feedback
        .as_ref()
        .map(|(message, _)| message.as_str())
        .unwrap_or("")
}

fn long_user_text(prefix: &str) -> String {
    (0..20)
        .map(|index| format!("{prefix} line {index} alpha beta gamma delta"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn message_navigation_reports_empty_first_and_latest() {
    let mut app = App::new();
    app.last_width = 40;
    app.last_height = 4;
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Up));
    assert_eq!(feedback(&app), "No messages yet.");
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    assert_eq!(feedback(&app), "No messages yet.");

    app.push(user_record("only"));
    app.push(assistant("answer"));
    app.last_width = 40;
    app.last_height = 4;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Up));
    assert_eq!(app.anchor.unwrap().record, 0);
    assert_eq!(feedback(&app), "Already at the first message.");
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    assert_eq!(app.anchor.unwrap().record, 0);
    assert_eq!(feedback(&app), "Already at the latest message.");
}

#[test]
fn ctrl_shift_jumps_only_between_real_user_messages() {
    let mut app = App::new();
    app.push(user_record("first question"));
    app.push(assistant("first answer"));
    app.push(tool("c1"));
    app.push(assistant("tool follow-up"));
    app.push(user_record("second question"));
    app.push(tool("c2"));
    app.last_width = 60;
    app.last_height = 6;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    let mut editor = Editor::new();

    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 4,
            offset: 0
        }),
        "next skips assistant and tool records"
    );
    assert!(!app.follow, "an actual jump stops follow");

    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Up));
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        }),
        "previous returns to the prior real user boundary"
    );
}

#[test]
fn previous_inside_a_long_user_returns_its_start_then_the_prior_user() {
    let mut app = App::new();
    app.push(user_record("prior question"));
    app.push(user_record(&long_user_text("current")));
    app.last_width = 24;
    app.last_height = 5;
    app.follow = false;
    let layout = view::record_layout(&mut app, 1, 24);
    let start = layout
        .iter()
        .position(|visual| !visual.line.to_string().trim().is_empty())
        .expect("content row");
    let inside = start + 4;
    app.anchor = Some(Anchor {
        record: 1,
        offset: layout[inside].source_start,
    });
    let mut editor = Editor::new();

    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Up));
    assert_eq!(app.anchor.unwrap().record, 1, "stays in the same message");
    assert_eq!(
        app.anchor.unwrap().offset,
        0,
        "returns to the message start"
    );
    assert_eq!(
        app.visual_anchor.map(|visual| visual.key),
        Some(crate::frontend::tui::selection::RowKey {
            record: 1,
            line: start,
        }),
        "the first content row is the boundary, not the padding row"
    );

    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Up));
    assert_eq!(
        app.anchor.unwrap().record,
        0,
        "from the first content row, previous selects the prior user"
    );
}

#[test]
fn next_from_inside_a_long_user_selects_the_following_user() {
    let mut app = App::new();
    app.push(user_record("prior question"));
    app.push(user_record(&long_user_text("current")));
    app.push(assistant("answer"));
    app.push(user_record("following question"));
    app.last_width = 24;
    app.last_height = 5;
    app.follow = false;
    let layout = view::record_layout(&mut app, 1, 24);
    app.anchor = Some(Anchor {
        record: 1,
        offset: layout[5].source_start,
    });
    let mut editor = Editor::new();

    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    assert_eq!(app.anchor.unwrap().record, 3);
    assert_eq!(app.anchor.unwrap().offset, 0);
}

#[test]
fn message_jumps_keep_draft_cursor_images_and_focus() {
    let mut app = App::new();
    app.push(user_record("one"));
    app.push(assistant("answer"));
    app.push(user_record("two"));
    app.last_width = 40;
    app.last_height = 4;
    let mut editor = Editor::new();
    editor.insert_str("draft\nlines");
    editor.move_visual_up(40);
    editor.insert_image(crate::input::ImageAttachment::new(
        "a.png",
        "data:image/png;base64,AA",
    ));
    let text = editor.text();
    let cursor = editor.cursor();
    let images = editor.images().len();
    app.focus = Focus::Input;

    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Up));
    assert_eq!(editor.text(), text);
    assert_eq!(editor.cursor(), cursor);
    assert_eq!(editor.images().len(), images);
    assert_eq!(app.focus, Focus::Input);
}

#[test]
fn user_jump_anchor_survives_reflow_and_new_output() {
    let mut app = App::new();
    app.push(user_record("first marker alpha beta gamma delta epsilon"));
    app.push(assistant("answer one"));
    app.push(user_record("second marker zeta eta theta iota kappa"));
    app.push(assistant("answer two"));
    app.last_width = 30;
    app.last_height = 4;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    assert_eq!(app.anchor.unwrap().record, 2);

    for width in [14usize, 40] {
        let lines = view::visible_lines(&mut app, width, 4);
        let joined = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let flat = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("second"), "width {width}: {joined}");
    }

    app.apply_event(AgentEvent::ResponseText {
        item_id: "late".to_owned(),
        text: "late output".to_owned(),
    });
    assert!(!app.follow);
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 2,
            offset: 0
        })
    );
    let lines = view::visible_lines(&mut app, 30, 4);
    let joined = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let flat = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("second marker"), "{joined}");
}

#[test]
fn user_jump_anchor_survives_activity_collapse() {
    let mut app = App::new();
    app.push(user_record("first question"));
    app.push(Record::Activity {
        end: Some(4),
        expanded: true,
        failed: false,
        interrupted: false,
        manual: true,
    });
    app.push(tool("c1"));
    app.push(tool("c2"));
    app.push(user_record("second question"));
    app.last_width = 40;
    app.last_height = 4;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, ctrl_shift(KeyCode::Down));
    assert_eq!(app.anchor.unwrap().record, 4);

    assert!(app.toggle_record(1));
    assert!(!app.activity_expanded(1));
    let keys = app.visible_record_keys();
    assert!(keys.contains(&4));
    assert!(!keys.contains(&2));
    let lines = view::visible_lines(&mut app, 40, 4);
    let joined = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("second question"), "{joined}");
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 4,
            offset: 0
        })
    );
}

#[test]
fn message_navigation_requires_ctrl_and_shift() {
    let mut app = App::new();
    app.push(user_record("one"));
    app.push(assistant("answer"));
    app.push(user_record("two"));
    app.last_width = 40;
    app.last_height = 4;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    let mut editor = Editor::new();
    editor.insert_str("line1\nline2");

    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Down, KeyModifiers::CONTROL),
    );
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        }),
        "Ctrl without Shift must not navigate"
    );
    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Down, KeyModifiers::SHIFT),
    );
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        }),
        "Shift without Ctrl must not navigate"
    );

    let before = editor.cursor();
    handle_key(&mut app, &mut editor, key(KeyCode::Up));
    assert_ne!(
        editor.cursor(),
        before,
        "plain Up still moves the input cursor"
    );
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        })
    );

    app.anchor = Some(Anchor {
        record: 1,
        offset: 0,
    });
    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Up, KeyModifiers::ALT),
    );
    assert_eq!(
        app.anchor.map(|anchor| anchor.record),
        Some(0),
        "Alt+Up still scrolls one line instead of jumping to a message"
    );
}

mod transcript_search_tests;

mod path_completion_tests;

mod external_editor_tests;

mod code_highlight_tests;
