use super::*;
use crate::client::history::{History, HistoryEntry, TaskStatus};
use crate::client::types::{InputItem, InputMessageContent, MessageRole};
use crate::frontend::tui::commands::Commands;
use crate::frontend::tui::sessions::Sessions;
use crate::frontend::tui::state::ContextInfo;
use crate::frontend::tui::theme::{self, Theme};
use crate::session::SessionEvent;
use unicode_width::UnicodeWidthStr;

fn screen(app: &mut App, editor: &Editor, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, app, editor))
        .unwrap();
    buffer_text(terminal.backend().buffer())
}

fn replay(history: Vec<HistoryEntry>, status: TaskStatus) -> History {
    History { history, status }
}

fn user(text: &str) -> InputItem {
    InputItem::Message {
        role: MessageRole::User,
        content: InputMessageContent::Text(text.to_owned()),
    }
}

fn assistant_item(text: &str) -> InputItem {
    InputItem::Message {
        role: MessageRole::Assistant,
        content: InputMessageContent::Text(text.to_owned()),
    }
}

fn reasoning_item(text: &str) -> InputItem {
    InputItem::Reasoning {
        content: vec![crate::client::types::ReasoningText::new(text)],
        provider_data: None,
    }
}

fn start_process(app: &mut App) {
    app.submit("Do the task");
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Inspect first".into(),
    });
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "read".into(),
        arguments: json!({"path": "src/main.rs"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".into(),
        name: "read".into(),
        output: json!({"output": "fn main() {}", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "All done.".into(),
    });
}

#[test]
fn streamed_answer_reflows_to_include_the_activity_separator_after_completion() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("Do work");
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "bash".into(),
        arguments: json!({"command": "echo hi"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".into(),
        name: "bash".into(),
        output: json!({"output": "hi", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "Answer".into(),
    });
    // Cache the streaming answer layout before the task boundary is known.
    let streaming = screen(&mut app, &editor, 48, 12);
    assert!(streaming.contains("Answer"), "{streaming}");
    assert!(
        !streaming.contains("▸ Activity"),
        "the running activity is expanded: {streaming}"
    );

    app.apply_event(AgentEvent::TurnCompleted);
    let completed = screen(&mut app, &editor, 48, 12);
    let rows: Vec<&str> = completed.lines().collect();
    let activity = rows
        .iter()
        .position(|row| row.contains("Activity"))
        .unwrap();
    assert!(rows[activity].contains("▸ Activity"), "{completed}");
    assert!(
        rows[activity + 1].trim().is_empty(),
        "one blank separates the collapsed Activity from the answer: {completed}"
    );
    assert!(rows[activity + 2].contains("Answer"), "{completed}");
}

#[test]
fn consecutive_tasks_create_separate_activity_groups() {
    let mut app = App::new();
    app.submit("first task");
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "bash".into(),
        arguments: json!({"command": "echo one"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".into(),
        name: "bash".into(),
        output: json!({"output": "one", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m1".into(),
        text: "first done".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);
    let first = 1;
    assert!(matches!(
        &app.records[first],
        Record::Activity {
            end: Some(3),
            expanded: false,
            ..
        }
    ));

    app.submit("second task");
    let second = app
        .records
        .iter()
        .rposition(|record| matches!(record, Record::Activity { .. }))
        .unwrap();
    assert!(second > first);
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c2".into(),
        name: "bash".into(),
        arguments: json!({"command": "echo two"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c2".into(),
        name: "bash".into(),
        output: json!({"output": "two", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m2".into(),
        text: "second done".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);
    assert!(matches!(
        &app.records[first],
        Record::Activity { end: Some(3), .. }
    ));
    assert!(matches!(
        &app.records[second],
        Record::Activity {
            end: Some(end),
            expanded: false,
            ..
        } if *end == second + 2
    ));
    let keys = app.visible_record_keys();
    assert_eq!(
        keys.iter()
            .filter(|key| matches!(app.records[**key], Record::Activity { .. }))
            .count(),
        2
    );
}

#[test]
fn empty_activity_marker_is_hidden_and_never_selected() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("hello");
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "hi".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);

    let keys = app.visible_record_keys();
    assert!(
        !keys
            .iter()
            .any(|key| matches!(app.records[*key], Record::Activity { .. })),
        "empty activity must not become a display item"
    );
    let text = screen(&mut app, &editor, 48, 12);
    assert!(!text.contains("Activity"), "{text}");

    app.focus = Focus::Transcript;
    app.selected = 1;
    view::select(&mut app, 0);
    assert!(matches!(
        app.records[app.selected],
        Record::User { .. } | Record::Assistant { .. }
    ));
}

#[test]
fn completed_task_collapses_to_one_activity_and_keeps_the_answer_clear() {
    let mut app = App::new();
    let mut editor = Editor::new();
    start_process(&mut app);
    app.apply_event(AgentEvent::TurnCompleted);
    assert!(app.follow);
    assert!(matches!(
        &app.records[1],
        Record::Activity {
            end: Some(4),
            expanded: false,
            ..
        }
    ));

    let text = screen(&mut app, &editor, 60, 20);
    assert!(text.contains("▸ Activity"), "{text}");
    assert!(text.contains("All done."), "{text}");
    assert!(!text.contains("Inspect first"), "{text}");
    assert!(!text.contains("Read src/main.rs"), "{text}");
    let rows: Vec<&str> = text.lines().collect();
    let activity = rows
        .iter()
        .position(|row| row.contains("▸ Activity"))
        .unwrap();
    assert!(
        rows[activity + 1].trim().is_empty(),
        "one blank between the collapsed Activity and the answer: {text}"
    );
    assert!(
        rows[activity + 2].contains("All done."),
        "the answer starts after exactly one blank: {text}"
    );

    // Keyboard expansion keeps the answer readable below the process.
    app.focus = Focus::Transcript;
    app.selected = 4;
    view::select(&mut app, -1);
    assert_eq!(app.selected, 1, "collapsed Activity is one selectable item");
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    let text = screen(&mut app, &editor, 60, 20);
    assert!(text.contains("Inspect first"), "{text}");
    assert!(text.contains("Read src/main.rs"), "{text}");
    assert!(text.contains("All done."), "{text}");
}

#[test]
fn narrow_reading_footer_reserves_context_before_optional_actions() {
    let mut app = App::new();
    let editor = Editor::new();
    app.context = Some(ContextInfo {
        model: "provider/deepseek-v4-flash".to_owned(),
        tokens: Some(0),
        window: Some(1_000_000),
        input_limit: None,
    });
    app.home();
    let text = screen(&mut app, &editor, 18, 4);
    let footer = text.lines().last().unwrap();
    assert!(footer.contains("Reading"), "{footer}");
    assert!(
        footer.replace(' ', "").contains("Reading·0%/1M"),
        "the context label survives the narrow reading footer: {footer}"
    );
    assert!(!footer.contains("Latest"), "{footer}");
    assert!(!footer.contains("Help"), "{footer}");
}

#[test]
fn selected_activity_header_shows_the_common_focus_marker() {
    let mut app = App::new();
    let editor = Editor::new();
    start_process(&mut app);
    app.apply_event(AgentEvent::TurnCompleted);
    app.focus = Focus::Transcript;
    app.selected = 4;
    view::select(&mut app, -1);
    assert_eq!(app.selected, 1, "collapsed Activity is selectable");

    let text = screen(&mut app, &editor, 60, 20);
    let rows: Vec<&str> = text.lines().collect();
    let activity = rows
        .iter()
        .position(|row| row.contains("Activity"))
        .unwrap();
    assert!(
        rows[activity].contains('▌'),
        "selected Activity header shows the common focus marker: {text}"
    );

    view::select(&mut app, 1);
    app.end();
    let text = screen(&mut app, &editor, 60, 20);
    let rows: Vec<&str> = text.lines().collect();
    let activity = rows
        .iter()
        .position(|row| row.contains("Activity"))
        .unwrap();
    assert!(
        !rows[activity].contains('▌'),
        "moving the selection clears the Activity marker: {text}"
    );
}

#[test]
fn mouse_toggles_activity_and_tool_details() {
    let mut app = App::new();
    let editor = Editor::new();
    start_process(&mut app);
    app.apply_event(AgentEvent::TurnCompleted);
    screen(&mut app, &editor, 60, 20);

    let (button, index) = app
        .disclosure_buttons
        .iter()
        .find(|(_, index)| matches!(app.records[*index], Record::Activity { .. }))
        .copied()
        .expect("activity disclosure button");
    assert!(matches!(
        &app.records[index],
        Record::Activity {
            expanded: false,
            ..
        }
    ));
    handle_mouse(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
    );
    handle_mouse(
        &mut app,
        mouse(MouseEventKind::Up(MouseButton::Left), button.x, button.y),
    );
    assert!(matches!(
        &app.records[index],
        Record::Activity { expanded: true, .. }
    ));
    // Redraw so the expanded members register their own disclosure buttons.
    screen(&mut app, &editor, 60, 20);

    let (tool_button, tool_index) = app
        .disclosure_buttons
        .iter()
        .find(|(_, index)| matches!(app.records[*index], Record::Tool { .. }))
        .copied()
        .expect("tool disclosure button");
    handle_mouse(
        &mut app,
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            tool_button.x,
            tool_button.y,
        ),
    );
    handle_mouse(
        &mut app,
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            tool_button.x,
            tool_button.y,
        ),
    );
    assert!(matches!(
        &app.records[tool_index],
        Record::Tool { expanded: true, .. }
    ));
    let text = screen(&mut app, &editor, 60, 20);
    assert!(text.contains("fn main()"), "{text}");
}

#[test]
fn manual_activity_expansion_survives_completion_and_follow() {
    for manual_expansion in [true, false] {
        let mut app = App::new();
        start_process(&mut app);
        // First toggle records the explicit choice; a second toggle selects the
        // expanded outcome when the test wants manual expansion.
        app.toggle_record(1);
        if manual_expansion {
            app.toggle_record(1);
        }
        app.apply_event(AgentEvent::TurnCompleted);
        assert!(
            matches!(&app.records[1], Record::Activity { expanded: true, .. }) == manual_expansion,
            "manual choice must win over auto-collapse"
        );
        app.end();
        assert!(
            matches!(&app.records[1], Record::Activity { expanded: true, .. }) == manual_expansion
        );
    }
}

#[test]
fn manually_collapsed_activity_stays_collapsed_on_failure() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("Long task");
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Inspect first".into(),
    });
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "bash".into(),
        arguments: json!({"command": "sleep 1"}).to_string(),
    });
    // The reader explicitly collapses the running process.
    app.toggle_record(1);
    app.home();
    let anchor = app.anchor;
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "Partial answer".into(),
    });
    app.apply_event(AgentEvent::TurnFailed {
        message: "link lost".into(),
    });
    assert!(matches!(
        &app.records[1],
        Record::Activity {
            expanded: false,
            failed: true,
            ..
        }
    ));
    assert_eq!(
        app.anchor, anchor,
        "failure must not steal the reading position"
    );
    let text = screen(&mut app, &editor, 60, 20);
    assert!(text.contains("failed"), "{text}");
    assert!(text.contains("link lost"), "{text}");
    assert!(text.contains("Partial answer"), "{text}");
    assert!(!text.contains("Inspect first"), "{text}");
}

#[test]
fn reader_position_prevents_auto_collapse() {
    let mut app = App::new();
    start_process(&mut app);
    app.home();
    let anchor = app.anchor;
    app.apply_event(AgentEvent::TurnCompleted);
    assert_eq!(app.anchor, anchor);
    assert!(matches!(
        &app.records[1],
        Record::Activity { expanded: true, .. }
    ));
}

#[test]
fn failed_task_keeps_process_visible_and_partial_text_unlabeled() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("Break the task");
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "bash".into(),
        arguments: json!({"command": "false"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".into(),
        name: "bash".into(),
        output: json!({"output": "", "exit_code": 1}),
    });
    app.apply_event(AgentEvent::ResponseTextDelta {
        item_id: "m".into(),
        delta: "Partial answer".into(),
    });
    app.apply_event(AgentEvent::TurnFailed {
        message: "network disconnected".into(),
    });
    let text = screen(&mut app, &editor, 60, 20);
    assert!(text.contains("failed"), "{text}");
    assert!(text.contains("Partial answer"), "{text}");
    assert!(text.contains("network disconnected"), "{text}");
    let header = view::record_layout(&mut app, 1, 60);
    assert!(
        header
            .iter()
            .flat_map(|line| &line.line.spans)
            .any(|span| span.style.fg == Some(app.theme.error)),
        "failed activity keeps its error style"
    );
}

#[test]
fn tool_failure_count_refreshes_in_the_activity_summary() {
    let mut app = App::new();
    let editor = Editor::new();
    app.submit("Check status");
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "bash".into(),
        arguments: json!({"command": "exit 2"}).to_string(),
    });
    let running = screen(&mut app, &editor, 60, 12);
    assert!(running.contains("1 tool"), "{running}");
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".into(),
        name: "bash".into(),
        output: json!({"output": "", "exit_code": 2}),
    });
    let failed = screen(&mut app, &editor, 60, 12);
    assert!(failed.contains("1 failed"), "{failed}");
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "Recovered".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);
    let collapsed = screen(&mut app, &editor, 60, 12);
    assert!(collapsed.contains("1 failed"), "{collapsed}");
    let layout = view::record_layout(&mut app, 1, 60);
    assert!(
        layout
            .iter()
            .flat_map(|line| &line.line.spans)
            .any(|span| span.style.fg == Some(app.theme.error))
    );
}

#[test]
fn restore_collapses_completed_tasks_and_leaves_legacy_history_untouched() {
    let completed = replay(
        vec![
            HistoryEntry::TaskStarted {
                task_id: "t1".into(),
                message_id: None,
                configuration: None,
            },
            HistoryEntry::Messages {
                items: vec![
                    user("old question"),
                    reasoning_item("old thought"),
                    assistant_item("old answer"),
                ],
            },
            HistoryEntry::TaskFinished {
                task_id: "t1".into(),
                status: TaskStatus::Completed,
                error: None,
            },
        ],
        TaskStatus::Completed,
    );
    let app = App::restore(&completed);
    assert!(matches!(
        &app.records[1],
        Record::Activity {
            end: Some(3),
            expanded: false,
            failed: false,
            interrupted: false,
            ..
        }
    ));
    assert_eq!(app.visible_record_keys(), vec![0, 1, 3]);

    let unknown = replay(
        vec![HistoryEntry::Messages {
            items: vec![user("legacy"), assistant_item("legacy answer")],
        }],
        TaskStatus::Unknown,
    );
    let app = App::restore(&unknown);
    assert!(
        !app.records
            .iter()
            .any(|record| matches!(record, Record::Activity { .. })),
        "legacy history without task boundaries is not proof of completion"
    );
    assert_eq!(app.visible_record_keys(), vec![0, 1]);
}

#[test]
fn restore_interrupted_group_does_not_absorb_the_next_task() {
    let interrupted = replay(
        vec![
            HistoryEntry::TaskStarted {
                task_id: "t1".into(),
                message_id: None,
                configuration: None,
            },
            HistoryEntry::Messages {
                items: vec![
                    user("interrupted question"),
                    reasoning_item("partial thinking"),
                    assistant_item("partial answer"),
                ],
            },
        ],
        TaskStatus::Interrupted,
    );
    let mut app = App::restore(&interrupted);
    let old_end = match &app.records[1] {
        Record::Activity {
            end: Some(end),
            interrupted: true,
            expanded: true,
            ..
        } => *end,
        other => panic!("expected a closed interrupted activity, got {other:?}"),
    };
    assert_eq!(old_end, 4, "restored range closes at the replay boundary");

    app.submit("next question");
    let new_marker = app
        .records
        .iter()
        .rposition(|record| matches!(record, Record::Activity { .. }))
        .unwrap();
    assert!(new_marker > old_end, "new marker starts a separate group");
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c2".into(),
        name: "read".into(),
        arguments: json!({"path": "src/lib.rs"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c2".into(),
        name: "read".into(),
        output: json!({"output": "data", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "next answer".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);

    assert!(matches!(
        &app.records[1],
        Record::Activity { end: Some(end), .. } if *end == old_end
    ));
    assert!(matches!(
        &app.records[new_marker],
        Record::Activity {
            end: Some(_),
            expanded: false,
            ..
        }
    ));
}

#[test]
fn hidden_activity_marker_does_not_trap_paging() {
    let mut app = App::new();
    app.submit("simple chat");
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "simple answer".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);
    app.last_width = 30;
    app.last_height = 3;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 2,
        offset: 0,
    });
    view::page_up(&mut app, 1);
    assert_eq!(app.anchor.unwrap().record, 0);
    view::page_up(&mut app, 1);
    assert_eq!(app.anchor.unwrap().record, 0);
    assert_eq!(app.anchor.unwrap().offset, 0);
}

#[test]
fn theme_roles_reach_rendered_user_block_and_body_text() {
    for theme in [theme::Theme::dark(), theme::Theme::light()] {
        let mut app = App::new();
        let editor = Editor::new();
        app.theme = theme;
        app.push(Record::User {
            text: "theme check".to_owned(),
            images: Vec::new(),
        });
        app.push(assistant("body text"));
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &mut app, &editor))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = buffer
            .content()
            .chunks(40)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let user_row = rows
            .iter()
            .position(|row| row.contains("theme check"))
            .unwrap();
        for x in 0..40u16 {
            let cell = buffer.cell((x, user_row as u16)).unwrap();
            assert_eq!(cell.fg, theme.user_fg, "user foreground");
            assert_eq!(cell.bg, theme.user_bg, "user background");
        }
        let body_row = rows
            .iter()
            .position(|row| row.contains("body text"))
            .unwrap();
        let x = rows[body_row].find("body text").unwrap() as u16;
        assert_eq!(
            buffer.cell((x, body_row as u16)).unwrap().fg,
            theme.text,
            "answer body uses the theme text role"
        );
    }
}

fn layout_text(app: &mut App, index: usize, width: usize) -> Vec<String> {
    view::record_layout(app, index, width)
        .iter()
        .map(|visual| visual.line.to_string())
        .collect()
}

#[test]
fn table_columns_render_with_explicit_alignment() {
    let text = "| Name | Count |\n| :--- | ---: |\n| alpha | 12 |\n| beta | 3 |";
    let mut app = App::new();
    app.push(assistant(text));
    let lines = layout_text(&mut app, 0, 60);
    let header = lines
        .iter()
        .position(|line| line.contains("Name"))
        .expect("header row");
    assert!(
        lines[header].contains("Name") && lines[header].contains("│ Count"),
        "{:?}",
        lines[header]
    );
    let header_layout = view::record_layout(&mut app, 0, 60);
    assert!(
        header_layout[header]
            .line
            .spans
            .iter()
            .any(|span| span.content.contains("Name")
                && span
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)),
        "header cells are bold"
    );
    let alpha = lines
        .iter()
        .find(|line| line.contains("alpha"))
        .expect("alpha row");
    let index = alpha.find("12").expect("count cell");
    assert!(
        alpha[..index].ends_with("   "),
        "right-aligned numeric cell must be padded before the value: {alpha:?}"
    );
    let beta = lines
        .iter()
        .find(|line| line.contains("beta"))
        .expect("beta row");
    assert!(beta.contains("│     3"), "{beta:?}");
}

#[test]
fn narrow_table_falls_back_to_key_value_records_keeping_every_cell() {
    let text = "| Name | Description |\n| --- | --- |\n| alpha | a long description value |";
    let mut app = App::new();
    app.push(assistant(text));
    let lines = layout_text(&mut app, 0, 20);
    let joined = lines.join("\n");
    let dense: String = joined.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(dense.contains("Name:alpha"), "{joined}");
    assert!(dense.contains("Description:a"), "{joined}");
    assert!(dense.contains("long"), "{joined}");
    assert!(dense.contains("value"), "{joined}");
    assert!(
        lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 20),
        "fallback lines stay inside the viewport: {lines:?}"
    );
}

#[test]
fn tiny_table_fallback_wraps_long_labels_instead_of_clipping_fields() {
    let text = "| Extremely long header | Value |\n| --- | --- |\n| first value | second value |";
    let mut app = App::new();
    app.push(assistant(text));
    let lines = layout_text(&mut app, 0, 12);
    let joined = lines.join("\n");
    let dense: String = joined.chars().filter(|c| !c.is_whitespace()).collect();
    for word in ["Extremely", "long", "header", "first", "value", "second"] {
        assert!(dense.contains(word), "missing {word:?} in {joined:?}");
    }
    assert!(
        lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 12),
        "tiny fallback lines stay inside the viewport: {lines:?}"
    );
}

#[test]
fn table_paragraph_gets_one_blank_and_copy_stays_raw() {
    let text = "| Name | Count |\n| --- | ---: |\n| alpha | 12 |\n\nAfter the table.";
    let mut app = App::new();
    app.push(assistant(text));
    let lines = layout_text(&mut app, 0, 40);
    let table_row = lines
        .iter()
        .position(|line| line.contains("alpha"))
        .expect("table body row");
    assert!(
        lines[table_row + 1].trim().is_empty(),
        "one blank after the table: {lines:?}"
    );
    assert!(
        lines[table_row + 2].contains("After the table."),
        "paragraph starts after exactly one blank: {lines:?}"
    );
    app.selected = 0;
    assert_eq!(view::copy_source(&app).as_deref(), Some(text));
}

#[test]
fn code_language_label_uses_secondary_text() {
    let mut app = App::new();
    app.push(assistant("```rust\nlet x = 1;\n```"));
    let layout = view::record_layout(&mut app, 0, 40);
    let label = layout
        .iter()
        .flat_map(|visual| &visual.line.spans)
        .find(|span| span.content.contains("rust"))
        .expect("language label");
    assert_eq!(label.style.fg, Some(app.theme.secondary));
}

#[test]
fn table_copy_keeps_raw_source() {
    let text = "| Name | Count |\n| :--- | ---: |\n| alpha | 12 |";
    let mut app = App::new();
    app.push(assistant(text));
    app.selected = 0;
    assert_eq!(view::copy_source(&app).as_deref(), Some(text));
}

#[test]
fn table_paging_reaches_every_line_including_the_header() {
    let text = "Before\n\n| Name | Count |\n| :--- | ---: |\n| alpha | 12 |\n| beta | 3 |\n\nAfter";
    let mut app = App::new();
    app.push(assistant(text));
    app.last_width = 30;
    app.last_height = 3;
    let layout = view::record_layout(&mut app, 0, 30);
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: layout.last().unwrap().source_start,
    });
    let mut visited = vec![line_index(&layout, app.anchor.unwrap().offset)];
    while !app.follow {
        view::page_up(&mut app, 1);
        if app.follow {
            break;
        }
        let line = line_index(&layout, app.anchor.unwrap().offset);
        if line == *visited.last().unwrap() && app.anchor.unwrap().offset == 0 {
            break;
        }
        visited.push(line);
        if visited.len() > layout.len() + 2 {
            panic!("paging stalled: {visited:?}");
        }
    }
    visited.dedup();
    for line in 0..layout.len() {
        assert!(
            visited.contains(&line),
            "line {line} unreachable; visited {visited:?}"
        );
    }
}

#[test]
fn prose_is_capped_while_tables_use_the_viewport() {
    let long = "word ".repeat(60);
    let mut app = App::new();
    app.push(assistant(&long));
    let lines = layout_text(&mut app, 0, 200);
    assert!(lines.len() > 1, "long prose wraps");
    assert!(
        lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 96),
        "prose stays at the readable cap: {:?}",
        lines
            .iter()
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .max()
    );

    let long = "notes ".repeat(24);
    let table = format!("| Notes |\n| --- |\n| {long} |");
    let mut app = App::new();
    app.push(assistant(&table));
    let lines = layout_text(&mut app, 0, 200);
    let widest = lines
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0);
    assert!(
        widest > 96,
        "table may use more than the prose cap: {widest}"
    );
}

#[test]
fn list_continuations_hang_and_remain_reachable_across_widths() {
    let text = "1. This is a deliberately long numbered item with many words so that it wraps several times at narrow widths without losing its number or indentation";
    for width in [16usize, 24, 40] {
        let mut app = App::new();
        app.push(assistant(text));
        app.last_width = width;
        app.last_height = 3;
        let layout = view::record_layout(&mut app, 0, width);
        assert!(layout.len() >= 3, "width {width}: expected wrapping");
        for (index, visual) in layout.iter().enumerate() {
            let line = visual.line.to_string();
            if index == 0 {
                assert!(line.starts_with("   1. "), "width {width}: {line:?}");
            } else if !line.trim().is_empty() {
                assert!(
                    line.starts_with("      "),
                    "width {width}: continuation keeps the hanging indent: {line:?}"
                );
            }
        }
        app.follow = false;
        app.anchor = Some(Anchor {
            record: 0,
            offset: layout.last().unwrap().source_start,
        });
        let mut expected = line_index(&layout, app.anchor.unwrap().offset);
        let mut steps = 0;
        while expected > 0 {
            view::page_up(&mut app, 1);
            expected -= 1;
            assert_eq!(
                line_index(&layout, app.anchor.unwrap().offset),
                expected,
                "width {width}: every continuation must be reachable one line at a time"
            );
            steps += 1;
            assert!(steps < layout.len() + 2, "width {width}: paging stalled");
        }
    }
}

#[test]
fn nested_and_task_list_continuations_keep_their_indent() {
    let long = "a deliberately long nested item that wraps several times over";
    let text = format!("- outer\n  - {long}\n- [ ] {long}");
    for width in [24usize, 32] {
        let mut app = App::new();
        app.push(assistant(&text));
        let layout = view::record_layout(&mut app, 0, width);
        let lines: Vec<String> = layout.iter().map(|v| v.line.to_string()).collect();
        let nested = lines
            .iter()
            .position(|line| line.contains("• a deliberately"))
            .expect("nested item first line");
        assert!(lines[nested].starts_with("     • "), "{:?}", lines[nested]);
        let task = lines
            .iter()
            .position(|line| line.contains("☐ a deliberately"))
            .expect("task item line");
        for line in lines[nested + 1..task]
            .iter()
            .filter(|line| !line.trim().is_empty())
        {
            assert!(
                line.starts_with("       "),
                "width {width}: nested continuation keeps 7-cell indent: {line:?}"
            );
        }
        for line in lines[task + 1..]
            .iter()
            .filter(|line| !line.trim().is_empty())
        {
            assert!(
                line.starts_with("     "),
                "width {width}: task continuation keeps 5-cell indent: {line:?}"
            );
        }
    }
}

#[test]
fn every_panel_renders_from_the_app_palette() {
    let mut app = App::new();
    let mut editor = Editor::new();
    app.theme = Theme::light();

    app.help_open = true;
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(90)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let help_row = rows.iter().position(|row| row.contains("Help ·")).unwrap();
    let help_x = rows[help_row].find("Help ·").unwrap() as u16;
    assert_eq!(
        buffer.cell((help_x, help_row as u16)).unwrap().fg,
        Theme::light().accent
    );
    app.help_open = false;

    editor.set_draft("/re");
    let mut commands = Commands::default();
    commands.sync(&app, &editor);
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal
        .draw(|frame| {
            view::draw(frame, &mut app, &editor);
            commands.draw(frame, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(90)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let command_row = rows
        .iter()
        .position(|row| row.contains("/resume"))
        .expect("command completion is drawn");
    let command_x = rows[command_row].find("/resume").unwrap() as u16;
    assert_eq!(
        buffer.cell((command_x, command_row as u16)).unwrap().fg,
        Theme::light().accent,
        "command completion follows the app palette"
    );

    editor.set_draft("");
    let mut sessions = Sessions::default();
    sessions.apply_event(&mut app, &mut editor, SessionEvent::Listed(Vec::new()));
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal
        .draw(|frame| {
            view::draw(frame, &mut app, &editor);
            sessions.draw(frame, app.theme);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = buffer
        .content()
        .chunks(90)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let picker_row = rows
        .iter()
        .position(|row| row.contains("Sessions"))
        .expect("session picker border");
    let picker_x = rows[picker_row].find("Sessions").unwrap() as u16;
    assert_eq!(
        buffer.cell((picker_x, picker_row as u16)).unwrap().fg,
        Theme::light().accent,
        "session picker border follows the app palette"
    );
}

#[test]
#[ignore = "manual visual fixture: run with --ignored --nocapture"]
fn visual_fixture_dumps_a_full_task_for_independent_review() {
    let mut app = App::new();
    let editor = Editor::new();
    app.working_directory = Some("/Workspace/Rust/Saki".into());
    app.submit("Fix the failing parser test");
    app.apply_event(AgentEvent::ReasoningDelta {
        item_id: "r".into(),
        delta: "Read the failing test and trace the table path.".into(),
    });
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        name: "read".into(),
        arguments: json!({"path": "src/frontend/tui/markdown.rs"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c1".into(),
        name: "read".into(),
        output: json!({"output": "table path", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ToolStarted {
        call_id: "c2".into(),
        name: "bash".into(),
        arguments: json!({"command": "cargo test tui::markdown"}).to_string(),
    });
    app.apply_event(AgentEvent::ToolFinished {
        call_id: "c2".into(),
        name: "bash".into(),
        output: json!({"output": "test result: ok", "exit_code": 0}),
    });
    app.apply_event(AgentEvent::ResponseText {
        item_id: "m".into(),
        text: "Fixed the table parser.\n\n| Check | Result |\n| --- | --- |\n| formatting | pass |\n| tests | pass |\n\nRun `cargo test` again after the next change.".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);

    let mut dump = String::new();
    for (name, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        app.theme = theme;
        for width in [48u16, 100] {
            let text = {
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal
                    .draw(|frame| view::draw(frame, &mut app, &editor))
                    .unwrap();
                buffer_text(terminal.backend().buffer())
            };
            assert!(text.contains("Fixed the table parser."));
            assert!(text.contains("Activity"));
            let rows: Vec<&str> = text.lines().collect();
            let activity_row = rows
                .iter()
                .position(|row| row.contains("Activity"))
                .unwrap();
            assert!(
                rows[activity_row + 1].trim().is_empty(),
                "blank line between Activity and answer: {text}"
            );
            assert!(
                rows[activity_row + 2].contains("Fixed the table parser."),
                "{text}"
            );
            let table_row = rows
                .iter()
                .position(|row| row.contains("tests") && row.contains("pass"))
                .unwrap();
            assert!(
                rows[table_row + 1].trim().is_empty(),
                "blank line between the table and the next paragraph: {text}"
            );
            assert!(rows[table_row + 2].contains("Run cargo test"), "{text}");
            dump.push_str(&format!(
                "===== {name} theme · {width} columns =====\n{text}\n\n"
            ));
        }
    }
    let path = std::path::PathBuf::from("/tmp/saki-tui-style-render-dump.txt");
    std::fs::write(&path, dump).unwrap();
}
