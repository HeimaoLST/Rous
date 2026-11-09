use super::*;
use crate::input::ImageAttachment;
use std::time::{Duration, Instant};

fn render(app: &mut App, editor: &Editor, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, app, editor))
        .unwrap();
    buffer_text(terminal.backend().buffer()).replace(' ', "")
}

#[test]
fn streaming_updates_one_record_reflows_cached_text_and_preserves_reading_position() {
    let mut app = App::new();
    let editor = Editor::new();
    app.push(assistant(&"earlier\n".repeat(40)));
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "First".into(),
    });
    let screen = render(&mut app, &editor, 48, 20);
    assert!(screen.contains("First"));
    assert!(app.layouts[1].is_some());
    let record_count = app.records.len();
    let anchor = app.anchor;
    let follow = app.follow;
    let running = app.running;
    for event in [
        AgentEvent::TaskStarted {
            task_id: "task".into(),
            message_id: None,
        },
        AgentEvent::ModelRequestStarted {
            purpose: crate::client::ModelRequestPurpose::Generation,
        },
        AgentEvent::ModelRequestStarted {
            purpose: crate::client::ModelRequestPurpose::Compaction,
        },
    ] {
        app.apply_event(event);
    }
    assert_eq!(app.records.len(), record_count);
    assert_eq!(app.anchor, anchor);
    assert_eq!(app.follow, follow);
    assert_eq!(app.running, running);
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: " second".into(),
    });
    assert!(app.layouts[1].is_none());
    let screen = render(&mut app, &editor, 48, 20);
    assert!(screen.contains("Firstsecond"), "{screen}");
    app.home();
    let anchor = app.anchor;
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: " third".into(),
    });
    assert_eq!(app.anchor, anchor);
    assert!(!app.follow);
    assert!(app.unread_output);
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "First second third".into(),
    });
    assert_eq!(app.records.len(), 2);
    app.end();
    assert!(render(&mut app, &editor, 24, 20).contains("third"));
    app.apply_event(AgentEvent::Usage {
        input_tokens: None,
        output_tokens: None,
    });
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "Partial next turn".into(),
    });
    app.apply_event(AgentEvent::TurnFailed {
        message: "Stream disconnected".into(),
    });
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "New response".into(),
    });
    assert_eq!(app.records.len(), 5);
    assert!(matches!(&app.records[2], Record::Assistant { text } if text == "Partial next turn"));
    assert!(matches!(&app.records[4], Record::Assistant { text } if text == "New response"));
}

#[test]
fn reasoning_streams_separately_from_the_answer_and_remains_copyable() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("Solve this");
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Check the".into(),
    });
    let screen = render(&mut app, &editor, 48, 20);
    assert!(screen.contains("Thought"), "{screen}");
    assert!(screen.contains("Checkthe"), "{screen}");
    assert!(app.layouts[2].is_some());
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: " assumptions 🧠".into(),
    });
    assert!(app.layouts[2].is_none());
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "The answer".into(),
    });
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: "Check the assumptions 🧠 carefully.".into(),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "The answer is 42.".into(),
    });
    assert_eq!(app.records.len(), 4);
    app.selected = 2;
    assert_eq!(
        view::copy_source(&app).as_deref(),
        Some("Check the assumptions 🧠 carefully.")
    );
    let screen = render(&mut app, &editor, 48, 20);
    assert_eq!(screen.matches("Thought").count(), 1, "{screen}");
    assert!(screen.contains("Theansweris42."), "{screen}");
    let layout = view::record_layout(&mut app, 2, 48);
    assert!(
        layout
            .iter()
            .flat_map(|line| &line.line.spans)
            .any(
                |span| (span.content.contains('▸') || span.content.contains('▾'))
                    && span.style.fg == Some(app.theme.accent)
            ),
        "Thought disclosure marker uses accent"
    );
    assert!(
        layout
            .iter()
            .flat_map(|line| &line.line.spans)
            .filter(|span| !span.content.trim().is_empty())
            .filter(|span| !span.content.contains('▸') && !span.content.contains('▾'))
            .all(|span| span.style.fg == Some(app.theme.secondary)),
        "reasoning content stays secondary"
    );
    assert!(
        layout
            .windows(2)
            .all(|pair| pair[0].source_start <= pair[1].source_start)
    );
    for width in [1, 3, 12] {
        let layout = view::record_layout(&mut app, 2, width);
        // A wide glyph may occupy two cells even in a one-cell viewport.
        assert!(layout.iter().all(|line| line.line.width() <= width.max(2)));
    }
}

#[test]
fn reasoning_without_final_plaintext_keeps_streamed_text_and_empty_items_stay_hidden() {
    let mut app = App::new();
    app.apply_event(AgentEvent::Reasoning {
        item_id: "opaque".into(),
        text: String::new(),
    });
    assert!(app.records.is_empty());
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Visible reasoning".into(),
    });
    app.home();
    let anchor = app.anchor;
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: " continues".into(),
    });
    assert_eq!(app.anchor, anchor);
    assert!(!app.follow);
    assert!(app.unread_output);
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: String::new(),
    });
    assert!(
        matches!(&app.records[0], Record::Reasoning { text, .. } if text == "Visible reasoning continues")
    );
    app.apply_event(AgentEvent::TurnFailed {
        message: "Disconnected".into(),
    });
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: "Next response".into(),
    });
    assert_eq!(app.records.len(), 3);
    assert!(matches!(&app.records[2], Record::Reasoning { text, .. } if text == "Next response"));
}

#[test]
fn reasoning_collapses_before_the_answer_and_enter_reopens_it_without_losing_text() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.submit("Question");
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Inspect the assumptions".into(),
    });
    assert!(render(&mut app, &editor, 60, 20).contains("Inspecttheassumptions"));
    app.apply_event(AgentEvent::ReasoningCompleted {
        item_id: "r".into(),
    });
    assert!(app.running, "the response is still generating");
    let screen = render(&mut app, &editor, 60, 20);
    assert!(screen.contains("▸Thought"), "{screen}");
    assert!(!screen.contains("Inspecttheassumptions"), "{screen}");
    assert_eq!(view::record_layout(&mut app, 2, 60).len(), 1);
    app.focus = Focus::Transcript;
    app.selected = 2;
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(render(&mut app, &editor, 60, 20).contains("Inspecttheassumptions"));
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: "Inspect the assumptions carefully".into(),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "Answer".into(),
    });
    assert_eq!(
        app.records.len(),
        4,
        "final reasoning reconciles the existing block"
    );
    assert!(matches!(
        &app.records[2],
        Record::Reasoning { expanded: true, .. }
    ));
    assert_eq!(
        view::copy_source(&app).as_deref(),
        Some("Inspect the assumptions carefully")
    );
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(view::record_layout(&mut app, 2, 60).len(), 1);
    assert_eq!(
        view::copy_source(&app).as_deref(),
        Some("Inspect the assumptions carefully")
    );
}

#[test]
fn clicking_thought_opens_from_the_beginning_and_keeps_the_draft_and_focus() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("pending draft");
    app.push(assistant("Earlier answer"));
    let thought = format!("BEGIN\n{}END", "details\n".repeat(100));
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: thought.clone(),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "Final answer".into(),
    });
    let screen = render(&mut app, &editor, 48, 20);
    assert!(screen.contains("▸Thought"), "{screen}");
    assert!(!screen.contains("Reasoning") && !screen.contains("collapsed"));
    let (button, index) = app.disclosure_buttons[0];
    assert_eq!(index, 1);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert!(handle_mouse(&mut app, mouse(kind, button.x - 1, button.y)).is_none());
    }
    assert!(matches!(
        &app.records[1],
        Record::Reasoning {
            expanded: false,
            ..
        }
    ));
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert!(handle_mouse(&mut app, mouse(kind, button.x, button.y)).is_none());
    }
    let screen = render(&mut app, &editor, 48, 20);
    assert!(
        screen.contains("▾Thought") && screen.contains("BEGIN"),
        "{screen}"
    );
    assert!(!screen.contains("END"), "opening must not jump to the tail");
    assert!(!app.follow);
    assert_eq!(app.focus, Focus::Input);
    assert_eq!(editor.text(), "pending draft");
    let (button, _) = app.disclosure_buttons[0];
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert!(handle_mouse(&mut app, mouse(kind, button.right() - 1, button.y)).is_none());
    }
    assert!(matches!(
        &app.records[1],
        Record::Reasoning {
            expanded: false,
            ..
        }
    ));
    app.selected = 1;
    assert_eq!(view::copy_source(&app), Some(thought));
}

#[test]
fn dragging_the_thought_label_copies_text_and_does_not_toggle_it() {
    let mut app = App::new();
    let editor = Editor::new();
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: "Hidden text".into(),
    });
    render(&mut app, &editor, 48, 20);
    let (button, _) = app.disclosure_buttons[0];
    handle_mouse(
        &mut app,
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.x + 2,
            button.y,
        ),
    );
    handle_mouse(
        &mut app,
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            button.right(),
            button.y,
        ),
    );
    assert_eq!(
        handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                button.right(),
                button.y
            )
        ),
        Some("Thought".into())
    );
    assert!(matches!(
        &app.records[0],
        Record::Reasoning {
            expanded: false,
            ..
        }
    ));
    app.help_open = true;
    render(&mut app, &editor, 24, 20);
    let (button, _) = app.disclosure_buttons[0];
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert!(handle_mouse(&mut app, mouse(kind, button.x, button.y)).is_none());
    }
    assert!(matches!(
        &app.records[0],
        Record::Reasoning {
            expanded: false,
            ..
        }
    ));
}

#[test]
fn automatic_reasoning_collapse_preserves_reading_and_manual_expansion() {
    for reading in [true, false] {
        let mut app = App::new();
        let mut editor = Editor::new();
        app.apply_event(AgentEvent::ReasoningDelta {
            item_id: "r".into(),
            delta: "Keep visible".into(),
        });
        if reading {
            app.home();
        } else {
            app.focus = Focus::Transcript;
            // Manually close and reopen while reasoning is still running.
            handle_key(&mut app, &mut editor, key(KeyCode::Enter));
            handle_key(&mut app, &mut editor, key(KeyCode::Enter));
            app.focus = Focus::Input;
        }
        let anchor = app.anchor;
        app.apply_event(AgentEvent::ReasoningCompleted {
            item_id: "r".into(),
        });
        app.apply_event(AgentEvent::Reasoning {
            item_id: "r".into(),
            text: "Keep visible".into(),
        });
        assert!(matches!(
            &app.records[0],
            Record::Reasoning { expanded: true, .. }
        ));
        assert_eq!(app.anchor, anchor);
    }
}

#[test]
fn reasoning_without_an_item_end_event_collapses_at_final_response_and_failure_keeps_partial_text()
{
    let mut app = App::new();
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Streamed text".into(),
    });
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: String::new(),
    });
    assert!(
        matches!(&app.records[0], Record::Reasoning { text, expanded: false, .. } if text == "Streamed text")
    );
    app.apply_event(AgentEvent::Usage {
        input_tokens: None,
        output_tokens: None,
    });
    app.apply_event(AgentEvent::Reasoning {
        item_id: "r".into(),
        text: "Non-streaming text".into(),
    });
    assert_eq!(app.records.len(), 2);
    assert!(matches!(
        &app.records[1],
        Record::Reasoning {
            expanded: false,
            ..
        }
    ));
    app.apply_event(AgentEvent::Usage {
        input_tokens: None,
        output_tokens: None,
    });
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Interrupted".into(),
    });
    app.apply_event(AgentEvent::TurnFailed {
        message: "Disconnected".into(),
    });
    assert!(
        matches!(&app.records[2], Record::Reasoning { text, expanded: true, .. } if text == "Interrupted")
    );
}

#[test]
fn shortcuts_and_usage_are_available_on_demand_instead_of_in_the_footer() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.usage = Some((Some(1178), None));
    for width in [120, 48, 20, 12] {
        let screen = render(&mut app, &editor, width, 24);
        assert!(screen.contains("Ready"), "{screen}");
        assert!(screen.contains("Help"), "{screen}");
        assert!(!screen.contains("Enter"), "{screen}");
        assert!(!screen.contains("1178"), "{screen}");
    }
    handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
    let screen = render(&mut app, &editor, 120, 24);
    assert!(
        screen.contains("Lastresponse:input1178/outputunknowntokens"),
        "{screen}"
    );
    let mut screen = screen;
    for _ in 0..10 {
        if screen.contains("Ctrl+V") {
            break;
        }
        handle_key(&mut app, &mut editor, key(KeyCode::PageDown));
        screen = render(&mut app, &editor, 120, 24);
    }
    assert!(screen.contains("Ctrl+V"), "{screen}");
    app.focus = Focus::Transcript;
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    let screen = render(&mut app, &editor, 120, 24);
    assert!(!screen.contains("Ctrl+V"), "{screen}");
}

#[test]
fn help_dismissal_preserves_the_entire_draft_and_reading_context() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.push(assistant(&"history\n".repeat(50)));
    app.home();
    app.focus = Focus::Transcript;
    editor.insert_str("草稿🙂");
    editor.move_left();
    editor.insert_image(ImageAttachment::new(
        "snapshot.png",
        "data:image/png;base64,QUJD",
    ));
    let cursor = editor.cursor();
    let anchor = app.anchor;
    for close in [
        key(KeyCode::Esc),
        key(KeyCode::F(1)),
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
        render(&mut app, &editor, 48, 20);
        let enter = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
        assert!(enter.send.is_none());
        let outcome = handle_key(&mut app, &mut editor, close);
        assert!(!outcome.quit);
        assert!(!app.help_open);
        assert_eq!(app.focus, Focus::Transcript);
        assert_eq!(app.anchor, anchor);
        assert!(!app.follow);
        assert_eq!(editor.text(), "草稿🙂");
        assert_eq!(editor.cursor(), cursor);
        assert_eq!(editor.images()[0].data_url, "data:image/png;base64,QUJD");
        assert!(app.history.is_empty());
    }
}

#[test]
fn question_mark_is_help_only_for_a_completely_empty_input() {
    let mut app = App::new();
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Char('?')));
    assert!(app.help_open);
    assert!(editor.is_empty());
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    editor.insert_str("why");
    handle_key(&mut app, &mut editor, key(KeyCode::Char('?')));
    assert_eq!(editor.text(), "why?");
    editor.clear();
    editor.insert_image(ImageAttachment::new(
        "image.png",
        "data:image/png;base64,QUJD",
    ));
    handle_key(&mut app, &mut editor, key(KeyCode::Char('?')));
    assert_eq!(editor.text(), "?");
    assert!(!app.help_open);
}

#[test]
fn help_can_be_scrolled_without_moving_history_even_in_a_tiny_viewport() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.push(assistant(&"history\n".repeat(50)));
    app.home();
    let anchor = app.anchor;
    for (width, height) in [(80, 24), (24, 8), (12, 4)] {
        handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
        render(&mut app, &editor, width, height);
        assert!(app.help_area.height > 0);
        handle_key(&mut app, &mut editor, key(KeyCode::PageDown));
        assert!(app.help_scroll > 0);
        handle_key(&mut app, &mut editor, key(KeyCode::End));
        let end = app.help_scroll;
        handle_key(&mut app, &mut editor, key(KeyCode::Down));
        assert_eq!(app.help_scroll, end);
        handle_key(&mut app, &mut editor, key(KeyCode::Home));
        assert_eq!(app.help_scroll, 0);
        assert_eq!(app.anchor, anchor);
        handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    }
    for (width, height) in [(1, 1), (2, 2), (4, 3)] {
        app.help_open = true;
        render(&mut app, &editor, width, height);
        handle_key(&mut app, &mut editor, key(KeyCode::Esc));
        assert!(!app.help_open);
    }
}

#[test]
fn mouse_help_and_latest_actions_preserve_input_and_use_the_drawn_bounds() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.push(assistant(&"history\n".repeat(50)));
    editor.insert_str("pending");
    render(&mut app, &editor, 80, 24);
    let button = app.help_button_area;
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
    );
    assert!(app.help_open);
    render(&mut app, &editor, 48, 20);
    let area = app.help_area;
    let _ = handle_mouse(&mut app, mouse(MouseEventKind::ScrollDown, area.x, area.y));
    assert_eq!(app.help_scroll, 3);
    let button = app.help_button_area;
    let _ = handle_mouse(
        &mut app,
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.right() - 1,
            button.y,
        ),
    );
    assert!(!app.help_open);
    app.home();
    app.apply_event(AgentEvent::ResponseText {
        item_id: "message".to_owned(),
        text: "NEW_RESULT".to_owned(),
    });
    let screen = render(&mut app, &editor, 80, 24);
    assert!(screen.contains("Newoutput"), "{screen}");
    let button = app.latest_button_area;
    let _ = handle_mouse(
        &mut app,
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.x - 1,
            button.y,
        ),
    );
    assert!(!app.follow, "the space before the action is not clickable");
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
    );
    assert!(app.follow);
    assert!(!app.unread_output);
    assert_eq!(editor.text(), "pending");
    assert_eq!(app.focus, Focus::Input);
}

#[test]
fn new_tool_results_are_reported_while_reading_and_cleared_when_following() {
    let mut app = App::new();
    app.push(tool("call"));
    app.home();
    let anchor = app.anchor;
    app.apply_event(AgentEvent::Usage {
        input_tokens: Some(10),
        output_tokens: None,
    });
    assert!(!app.unread_output);
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "call".to_owned(),
        name: "bash".to_owned(),
        output: json!({"exit_code": 0}),
    });
    assert!(app.unread_output);
    assert_eq!(app.anchor, anchor);
    assert_eq!(app.records.len(), 1);
    view::page_down(&mut app, 100);
    assert!(app.follow);
    assert!(!app.unread_output);
}

#[test]
fn copy_feedback_expires_without_masking_task_activity_or_exit_state() {
    let mut app = App::new();
    let editor = Editor::new();
    app.running = true;
    app.push(tool("call"));
    app.notify("Copied 3 characters".to_owned());
    let now = Instant::now();
    let screen = render(&mut app, &editor, 80, 24);
    assert!(screen.contains("Working·Runechohi"), "{screen}");
    assert!(screen.contains("Copied3characters"), "{screen}");
    app.expire_feedback(now);
    assert!(app.feedback.is_some());
    app.expire_feedback(now + Duration::from_secs(4));
    assert!(app.feedback.is_none());
    app.quit_after_turn = true;
    let screen = render(&mut app, &editor, 80, 24);
    assert!(screen.contains("Waitingtoexit"), "{screen}");
    assert!(app.running);
}

#[test]
fn log_updates_are_reported_and_returning_to_latest_clears_them() {
    let path = std::env::temp_dir().join(format!("saki-ux-log-{}.txt", std::process::id()));
    std::fs::write(&path, "log body\n").unwrap();
    let mut app = App::new();
    app.push(Record::Tool {
        call_id: "log".to_owned(),
        name: "bash".to_owned(),
        arguments: "{}".to_owned(),
        output: Some(json!({"output_path": path})),
        expanded: true,
    });
    assert!(log::open(&mut app));
    app.apply_event(AgentEvent::ResponseText {
        item_id: "message".to_owned(),
        text: "result".to_owned(),
    });
    assert!(app.unread_output);
    log::close(&mut app);
    assert!(app.follow);
    assert!(!app.unread_output);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tool_summary_stays_on_one_line_while_details_and_copy_keep_the_full_command() {
    let mut app = App::new();
    let command = format!("echo 中文🙂{}\necho LAST_COMMAND", "长命令".repeat(20));
    app.push(Record::Tool {
        call_id: "long".to_owned(),
        name: "bash".to_owned(),
        arguments: json!({"command": command}).to_string(),
        output: None,
        expanded: false,
    });
    let summary = view::record_layout(&mut app, 0, 24);
    assert_eq!(summary.len(), 1);
    assert!(summary[0].line.to_string().ends_with('…'));
    assert!(summary[0].line.width() <= 24);
    assert_eq!(view::copy_source(&app).as_deref(), Some(command.as_str()));
    app.toggle_selected();
    let details = view::record_layout(&mut app, 0, 24);
    assert!(details.len() > 1);
    let body: String = details
        .iter()
        .skip(1)
        .map(|line| line.line.to_string().trim_start().to_owned())
        .collect();
    assert!(body.contains("LAST_COMMAND"));
}

#[test]
fn statusline_shows_model_and_calibrated_context_while_preserving_actions_on_resize() {
    let mut app = App::new();
    app.working_directory = Some("/Workspace/Rust/Saki".into());
    let mut editor = Editor::new();
    app.apply_event(AgentEvent::ContextUpdated {
        model: "provider/deepseek-v4-flash".to_owned(),
        context_tokens: Some(120_000),
        context_window: Some(1_000_000),
        input_limit: Some(991_808),
    });
    assert!(app.records.is_empty());
    assert!(!app.unread_output);
    for width in [120, 80, 48, 32, 20, 12] {
        let screen = render(&mut app, &editor, width, 24);
        let footer = screen.lines().last().unwrap();
        assert!(
            screen.lines().nth(20).unwrap().contains("Ready"),
            "{screen}"
        );
        assert!(!footer.contains("Ready"), "{width}: {footer}");
        assert!(!footer.contains("SAKI"));
        assert!(
            !footer.contains("120k") && !footer.contains("~"),
            "{footer}"
        );
        // The context label is reserved before the optional Help action, so
        // Help appears only once both fit together.
        assert_eq!(footer.contains("Help"), width >= 14, "{width}: {footer}");
        assert!(footer.starts_with("12%/1M"), "{width}: {footer}");
        if width >= 48 {
            assert!(footer.contains("deepseek-v4-flash"), "{width}: {footer}");
            assert!(!footer.contains("Saki"), "{footer}");
        }
    }
    app.running = true;
    app.push(tool("call"));
    let screen = render(&mut app, &editor, 120, 24);
    assert!(
        screen
            .lines()
            .nth(20)
            .unwrap()
            .contains("Working·Runechohi"),
        "{screen}"
    );
    assert!(!screen.lines().last().unwrap().contains("Working"));
    let tiny = render(&mut app, &editor, 48, 4);
    assert!(tiny.lines().last().unwrap().contains("Working"), "{tiny}");
    assert!(tiny.lines().last().unwrap().contains("Help"), "{tiny}");
    app.home();
    app.apply_event(AgentEvent::ContextUpdated {
        model: "provider/deepseek-v4-flash".to_owned(),
        context_tokens: Some(5000),
        context_window: Some(1_000_000),
        input_limit: Some(991_808),
    });
    assert!(
        !app.unread_output,
        "context metadata is not new transcript output"
    );
    handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
    let screen = render(&mut app, &editor, 120, 24);
    assert!(
        screen.contains("Directory:/Workspace/Rust/Saki"),
        "{screen}"
    );
    assert!(
        screen.contains("Model:provider/deepseek-v4-flash"),
        "{screen}"
    );
    assert!(screen.contains("Inputlimit:991808tokens"), "{screen}");
    app.help_open = false;
    app.apply_event(AgentEvent::ContextUpdated {
        model: "provider/deepseek-v4-flash".to_owned(),
        context_tokens: Some(120_000),
        context_window: Some(2_000_000),
        input_limit: Some(1_991_808),
    });
    let screen = render(&mut app, &editor, 80, 24);
    assert!(
        screen.lines().last().unwrap().starts_with("6%/2M"),
        "{screen}"
    );
}

#[test]
fn missing_context_measurements_stay_unknown_instead_of_becoming_zero() {
    let mut app = App::new();
    let editor = Editor::new();
    app.apply_event(AgentEvent::ContextUpdated {
        model: "test".to_owned(),
        context_tokens: None,
        context_window: None,
        input_limit: None,
    });
    let screen = render(&mut app, &editor, 80, 24);
    assert!(
        screen.lines().last().unwrap().starts_with("?%/?"),
        "{screen}"
    );
    assert!(!screen.contains("0%"), "{screen}");
}

#[test]
fn esc_running_requests_cancel_keeps_draft_and_is_idempotent() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.insert_str("keep me");
    app.running = true;
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(outcome.cancel, "Esc while running asks to cancel");
    assert!(!outcome.quit && outcome.send.is_none());
    assert_eq!(editor.text(), "keep me");
    app.cancelling = true;
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel, "repeat Esc while cancelling is harmless");
    assert!(!outcome.quit);
    assert_eq!(editor.text(), "keep me");
}

#[test]
fn esc_clears_selection_before_cancel_and_idle_esc_never_quits() {
    let mut app = App::new();
    app.theme = crate::frontend::tui::theme::Theme::dark();
    let mut editor = Editor::new();
    app.running = true;
    app.push(assistant("alpha beta"));
    let mut terminal = Terminal::new(TestBackend::new(48, 12)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
    );
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Drag(MouseButton::Left), 4, 0),
    );
    assert!(app.text_selection.active());
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel, "an active selection is cleared first");
    assert!(app.text_selection.text().is_none());
    assert!(!outcome.quit);
    app.running = false;
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel && !outcome.quit);
}

#[test]
fn turn_interrupted_marks_the_real_activity_group_and_closes_the_running_tool() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("run a tool");
    app.cancelling = true;
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c".into(),
        name: "bash".into(),
        output: json!({"error":"tool interrupted","interrupted":true}),
    });
    app.apply_event(AgentEvent::TurnInterrupted);
    assert!(!app.running && !app.cancelling && app.status.is_none());
    let activity = app
        .records
        .iter()
        .find_map(|record| match record {
            Record::Activity {
                interrupted,
                failed,
                ..
            } => Some((*interrupted, *failed)),
            _ => None,
        })
        .expect("the submitted activity group exists");
    assert_eq!(
        activity,
        (true, false),
        "activity is interrupted, not failed"
    );
    let output = app
        .records
        .iter()
        .find_map(|record| match record {
            Record::Tool {
                output: Some(output),
                ..
            } => Some(output),
            _ => None,
        })
        .expect("the running tool output was closed");
    let (glyph, failed) = view::tool_status(Some(output));
    assert!(failed, "an interrupted tool must not show success: {glyph}");
    let screen = render(&mut app, &editor, 48, 20);
    assert!(screen.contains("Interrupted"), "{screen}");
    assert!(
        !screen.contains('✓'),
        "an interrupted tool must not render a green check: {screen}"
    );
}

#[test]
fn plain_click_then_esc_while_running_cancels() {
    let mut app = App::new();
    app.theme = crate::frontend::tui::theme::Theme::dark();
    let mut editor = Editor::new();
    app.running = true;
    app.push(assistant("alpha beta"));
    let mut terminal = Terminal::new(TestBackend::new(48, 12)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let _ = handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 2, 0),
    );
    assert!(handle_mouse(&mut app, mouse(MouseEventKind::Up(MouseButton::Left), 2, 0)).is_none());
    assert!(
        !app.text_selection.active(),
        "a plain click must not leave an invisible selection"
    );
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(outcome.cancel, "Esc after a plain click cancels the turn");
}

#[test]
fn esc_closes_help_before_interrupting_a_running_task() {
    let mut app = App::new();
    app.running = true;
    let mut editor = Editor::new();
    editor.insert_str("keep draft");
    handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
    assert!(app.help_open);
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel, "the first Esc only closes help");
    assert!(!app.help_open);
    assert_eq!(editor.text(), "keep draft");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(outcome.cancel, "a later Esc interrupts the task");
    assert_eq!(editor.text(), "keep draft");
}
