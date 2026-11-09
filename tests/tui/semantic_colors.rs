use super::*;
use crate::frontend::tui::theme::{NativeColors, Theme, native_theme};
use ratatui::style::{Color, Modifier};

fn first_span<'a>(
    layout: &'a [crate::frontend::tui::state::VisualLine],
    needle: &str,
) -> &'a ratatui::text::Span<'static> {
    layout
        .iter()
        .flat_map(|visual| &visual.line.spans)
        .find(|span| span.content.contains(needle))
        .unwrap_or_else(|| panic!("no span containing {needle:?}"))
}

#[test]
fn headings_strong_and_inline_code_keep_semantic_hues() {
    let mut app = App::new();
    app.theme = Theme::light();
    app.push(Record::Assistant {
        text: "# Head **strong** `code`\n\nplain **bold** `span`".to_owned(),
    });
    let layout = view::record_layout(&mut app, 0, 60);
    assert_eq!(
        first_span(&layout, "Head").style.fg,
        Some(Theme::light().accent),
        "headings use the accent landmark"
    );
    assert_eq!(
        first_span(&layout, "strong").style.fg,
        Some(Theme::light().accent),
        "explicit strong markup uses the accent landmark"
    );
    assert!(
        first_span(&layout, "strong")
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(
        first_span(&layout, "code").style.fg,
        Some(Theme::light().code),
        "inline code keeps its own hue inside a heading"
    );
    assert_eq!(
        first_span(&layout, "plain").style.fg,
        Some(Theme::light().text),
        "ordinary prose stays foreground"
    );
    assert_eq!(
        first_span(&layout, "bold").style.fg,
        Some(Theme::light().accent)
    );
    assert_eq!(
        first_span(&layout, "span").style.fg,
        Some(Theme::light().code)
    );
}

#[test]
fn table_header_uses_accent_while_inline_code_keeps_code_hue() {
    let mut app = App::new();
    app.theme = Theme::light();
    app.push(Record::Assistant {
        text: "| Head `code` | Plain |\n| --- | --- |\n| value | other |".to_owned(),
    });
    let layout = view::record_layout(&mut app, 0, 60);
    let header = layout
        .iter()
        .find(|visual| visual.line.to_string().contains("Head"))
        .expect("header row");
    let header_span = |needle: &str| {
        header
            .line
            .spans
            .iter()
            .find(|span| span.content.contains(needle))
            .unwrap_or_else(|| panic!("no header span containing {needle:?}"))
    };
    assert_eq!(
        header_span("Head").style.fg,
        Some(Theme::light().accent),
        "plain header text is tinted accent"
    );
    assert!(
        header_span("Head")
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(
        header_span("code").style.fg,
        Some(Theme::light().code),
        "inline code in a header keeps the code hue"
    );
    assert_eq!(header_span("Plain").style.fg, Some(Theme::light().accent));
}

#[test]
fn fallback_table_labels_use_accent_and_values_stay_plain() {
    let mut app = App::new();
    app.theme = Theme::light();
    app.push(Record::Assistant {
        text: "| Name | Value |\n| --- | --- |\n| ab | cd |".to_owned(),
    });
    let layout = view::record_layout(&mut app, 0, 11);
    let value_line = layout
        .iter()
        .find(|visual| visual.line.to_string().contains(" ab"))
        .expect("short fallback value line");
    assert!(
        value_line
            .line
            .spans
            .iter()
            .any(|span| span.content.contains("Name:")
                && span.style.fg == Some(Theme::light().accent)),
        "fallback field label is accent: {:?}",
        value_line.line
    );
    assert!(
        value_line
            .line
            .spans
            .iter()
            .any(|span| span.content.contains("ab") && span.style.fg == Some(Theme::light().text)),
        "fallback value stays plain: {:?}",
        value_line.line
    );

    // A label too long for the prefix wraps onto its own accent line, and the
    // value keeps the plain foreground.
    let mut app = App::new();
    app.theme = Theme::light();
    app.push(Record::Assistant {
        text: "| Extremely long header | Value |\n| --- | --- |\n| alpha | beta |".to_owned(),
    });
    let layout = view::record_layout(&mut app, 0, 14);
    let label_line = layout
        .iter()
        .find(|visual| visual.line.to_string().contains("header"))
        .expect("wrapped fallback label line");
    assert!(
        label_line
            .line
            .spans
            .iter()
            .any(|span| span.style.fg == Some(Theme::light().accent)),
        "wrapped fallback label is accent: {:?}",
        label_line.line
    );
    let value_line = layout
        .iter()
        .find(|visual| visual.line.to_string().contains("alpha"))
        .expect("long fallback value line");
    assert!(
        value_line.line.spans.iter().any(
            |span| span.content.contains("alpha") && span.style.fg == Some(Theme::light().text)
        ),
        "long fallback value stays plain: {:?}",
        value_line.line
    );
}

#[test]
fn tool_marks_tint_by_state_while_command_stays_secondary() {
    let mut app = App::new();
    app.theme = Theme::light();
    let tool = |call_id: &str, output: Option<serde_json::Value>| Record::Tool {
        call_id: call_id.to_owned(),
        name: "bash".to_owned(),
        arguments: json!({"command": "echo hi"}).to_string(),
        output,
        expanded: false,
    };
    app.push(Record::User {
        text: "run".to_owned(),
        images: Vec::new(),
    });
    app.push(tool("running", None));
    app.push(tool("success", Some(json!({"exit_code": 0}))));
    app.push(tool("failed", Some(json!({"exit_code": 3}))));

    let running = view::record_layout(&mut app, 1, 60);
    assert_eq!(
        first_span(&running, "…").style.fg,
        Some(Theme::light().warning),
        "running mark is amber"
    );
    assert_eq!(
        first_span(&running, "Run echo hi").style.fg,
        Some(Theme::light().secondary),
        "running command stays secondary"
    );

    let success = view::record_layout(&mut app, 2, 60);
    assert_eq!(
        first_span(&success, "✓").style.fg,
        Some(Theme::light().accent),
        "success mark is green accent"
    );
    assert_eq!(
        first_span(&success, "Run echo hi").style.fg,
        Some(Theme::light().secondary),
        "successful rows do not color the command text"
    );

    let failed = view::record_layout(&mut app, 3, 60);
    assert_eq!(
        first_span(&failed, "✗").style.fg,
        Some(Theme::light().error),
        "failure mark is red"
    );
    assert_eq!(
        first_span(&failed, "Run echo hi").style.fg,
        Some(Theme::light().error),
        "failure detail stays readable error red"
    );
}

#[test]
fn activity_and_thought_disclosures_use_accent_labels() {
    let mut app = App::new();
    app.theme = Theme::light();
    app.submit("task");
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
        text: "Done.".into(),
    });
    app.apply_event(AgentEvent::TurnCompleted);

    let activity = view::record_layout(&mut app, 1, 60);
    assert_eq!(
        first_span(&activity, "Activity").style.fg,
        Some(Theme::light().accent),
        "Activity disclosure label is accent"
    );
    assert_eq!(
        first_span(&activity, "1 tool").style.fg,
        Some(Theme::light().secondary),
        "Activity counts stay secondary"
    );

    let mut app = App::new();
    app.theme = Theme::light();
    app.push(Record::Reasoning {
        text: "hidden".into(),
        expanded: true,
        auto_collapse: false,
    });
    let reasoning = view::record_layout(&mut app, 0, 60);
    assert_eq!(
        first_span(&reasoning, "▾").style.fg,
        Some(Theme::light().accent),
        "Thought disclosure marker is accent"
    );
    assert_eq!(
        first_span(&reasoning, "Thought").style.fg,
        Some(Theme::light().secondary),
        "Thought label stays secondary"
    );
}

#[test]
#[ignore = "manual rendered-color capture: cargo test --bin rous semantic_render_capture -- --ignored --nocapture"]
fn semantic_render_capture_writes_actual_cells() {
    let mut html = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>SAKI semantic colors</title>\
         <style>body{font-family:ui-monospace,Menlo,monospace;margin:16px}\
         h2{margin:18px 0 6px}pre{display:inline-block;padding:12px;line-height:1.25}\
         .dark{background:#191d1c}.light{background:#f6efe2}</style>",
    );
    let mut text_dump = String::new();
    // Actual auto-derived palettes: the reported dark-purple-on-cream terminal
    // plus a dark terminal. Defaults for `Color::Reset` come from the sampled
    // pair, not from a theme guess.
    let tiles: [(&str, Theme, &str, &str); 2] = [
        (
            "auto purple/cream",
            native_theme(NativeColors {
                foreground: (0x57, 0x52, 0x79),
                background: (0xf6, 0xef, 0xe2),
            }),
            "#575279",
            "#f6efe2",
        ),
        (
            "auto dark",
            native_theme(NativeColors {
                foreground: (0xdc, 0xe4, 0xde),
                background: (0x19, 0x1d, 0x1c),
            }),
            "#dce4de",
            "#191d1c",
        ),
    ];
    for (name, theme, default_fg, default_bg) in tiles {
        let mut app = App::new();
        app.theme = theme;
        app.working_directory = Some("/Workspace/Rust/Saki".into());
        app.submit("Fix the failing parser test");
        app.apply_event(AgentEvent::ReasoningDelta {
            item_id: "r".into(),
            delta: "Read the failing test, derive the surface, then summarize.".into(),
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
            output: json!({"output": "failed", "exit_code": 3}),
        });
        app.apply_event(AgentEvent::ToolStarted {
            call_id: "c3".into(),
            name: "bash".into(),
            arguments: json!({"command": "sleep 1"}).to_string(),
        });
        app.apply_event(AgentEvent::ResponseText {
            item_id: "m".into(),
            text: "## Result\n\n**Fixed** the table parser.\n\n| Check | Result |\n| --- | --- |\n| formatting | pass |\n| tests | pass |\n\nRun `cargo test` again after the next change.".into(),
        });
        app.apply_event(AgentEvent::TurnCompleted);
        app.selected = 1;
        app.toggle_record(1);

        let width = 100u16;
        let height = 34u16;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &mut app, &Editor::new()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        text_dump.push_str(&format!("===== {name} =====\n"));
        text_dump.push_str(&buffer_text(buffer));
        text_dump.push('\n');
        html.push_str(&format!(
            "<h2>{name}</h2><pre style=\"background:{default_bg}\">"
        ));
        html.push_str(&cells_to_html(buffer, default_fg, default_bg));
        html.push_str("</pre>");
    }
    std::fs::write("/tmp/saki-tui-semantic-colors.html", html).expect("write HTML capture");
    std::fs::write("/tmp/saki-tui-semantic-colors.txt", text_dump).expect("write text capture");
}

fn cells_to_html(buffer: &Buffer, default_fg: &str, default_bg: &str) -> String {
    let fallback_bg = default_bg;
    let fallback_fg = default_fg;
    let color_css = |color: Color, fallback: &str| -> String {
        match color {
            Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
            _ => fallback.to_owned(),
        }
    };
    let mut html = String::new();
    for row in buffer.content().chunks(buffer.area.width as usize) {
        let mut current: Option<String> = None;
        for cell in row {
            let fg = color_css(cell.fg, fallback_fg);
            let bg = color_css(cell.bg, fallback_bg);
            let mut style = format!("color:{fg}");
            if bg != fallback_bg {
                style.push_str(&format!(";background:{bg}"));
            }
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str(";font-weight:bold");
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                style.push_str(";font-style:italic");
            }
            if cell.modifier.contains(Modifier::UNDERLINED) {
                style.push_str(";text-decoration:underline");
            }
            if current.as_deref() != Some(style.as_str()) {
                if current.is_some() {
                    html.push_str("</span>");
                }
                html.push_str(&format!("<span style=\"{style}\">"));
                current = Some(style);
            }
            html.push_str(&escape_html(cell.symbol()));
        }
        if current.is_some() {
            html.push_str("</span>");
        }
        html.push('\n');
    }
    html
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
