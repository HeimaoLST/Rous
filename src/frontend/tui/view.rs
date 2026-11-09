use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::frontend::tui::editor::Editor;
use crate::frontend::tui::help;
use crate::frontend::tui::log;
use crate::frontend::tui::markdown;
use crate::frontend::tui::markdown_layout;
use crate::frontend::tui::selection::{Edge, RowKey, ViewportRow, VisualAnchor};
use crate::frontend::tui::state::{
    Anchor, App, CachedLayout, ContextInfo, Focus, Record, VisualLine,
};
use crate::frontend::tui::textwrap::wrap_bytes;
use crate::frontend::tui::theme;
use crate::frontend::tui::transcript_search;
use crate::input::ImageAttachment;

/// Draws the whole screen. Only the records needed to fill the transcript
/// viewport are laid out, so long history does not build one big buffer.
pub fn draw(frame: &mut Frame, app: &mut App, editor: &Editor) {
    let theme = app.theme;
    let area = frame.area();
    let input_height =
        (editor.layout(area.width as usize).lines.len() + editor.images().len()).clamp(1, 5) as u16;
    let feedback_height = u16::from(app.status.is_some() || app.feedback.is_some());
    let activity_height = u16::from(area.height > 4);
    let search_height = u16::from(app.transcript_search.is_some());
    let help_height = if app.help_open {
        let available = area
            .height
            .saturating_sub(input_height + 2 + feedback_height + activity_height + search_height);
        available.saturating_sub(u16::from(available > 1)).min(14)
    } else {
        0
    };
    let chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(search_height),
        Constraint::Length(help_height),
        Constraint::Length(activity_height),
        Constraint::Length(input_height + 1),
        Constraint::Length(feedback_height),
        Constraint::Length(1),
    ])
    .split(area);
    app.disclosure_buttons.clear();
    if app.log_view.is_some() {
        draw_log(frame, app, chunks[0]);
    } else {
        draw_transcript(frame, app, chunks[0]);
    }
    if search_height > 0 {
        transcript_search::draw(frame, app, chunks[1]);
    }
    app.help_area = chunks[2];
    if app.help_open {
        help::draw(frame, app, chunks[2]);
    }
    if activity_height > 0 {
        frame.render_widget(
            Paragraph::new(activity_line(app, editor, area.width as usize)),
            chunks[3],
        );
    }
    app.input_area = chunks[4];
    draw_input(frame, app, editor, chunks[4]);
    if let Some(message) = app
        .feedback
        .as_ref()
        .map(|(text, _)| text.as_str())
        .or(app.status.as_deref())
    {
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(theme.accent)),
            chunks[5],
        );
    }
    draw_status(frame, app, editor, chunks[6], activity_height > 0);
    let viewport: Vec<(RowKey, u16)> = app
        .transcript_rows
        .iter()
        .map(|row| (row.key, row.row))
        .collect();
    let area = app.transcript_area;
    app.text_selection
        .render(frame.buffer_mut(), theme.selection(), &viewport, area);
}

fn draw_transcript(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.transcript_area != area || app.last_width != area.width as usize {
        // Logical row identity does not survive width or region reflow.
        app.text_selection.clear();
    }
    app.last_width = area.width as usize;
    app.last_height = area.height as usize;
    app.transcript_area = area;
    let lines = visible_lines(app, app.last_width, app.last_height);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    app.last_width = area.width as usize;
    app.last_height = area.height as usize;
    app.transcript_area = area;
    let Some(view) = &app.log_view else {
        return;
    };
    let filename = std::path::Path::new(&view.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&view.path);
    let range = format!(
        "bytes {}..{}{}",
        view.offset,
        view.next,
        if view.eof { "  EOF" } else { "" }
    );
    let body = log_body_lines(view, app.last_width);
    let available = area.height.saturating_sub(2) as usize;
    let start = view.scroll.min(body.len().saturating_sub(1));
    let mut lines = vec![
        Line::from(Span::styled(
            format!("log {filename}"),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(range, Style::default().fg(theme.accent))),
    ];
    lines.extend(body.into_iter().skip(start).take(available));
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn log_body_lines(view: &crate::frontend::tui::state::LogView, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for raw in view.text.lines() {
        if raw.is_empty() {
            lines.push(Line::from(String::new()));
            continue;
        }
        for (part, _) in wrap_bytes(raw, width) {
            lines.push(Line::from(part));
        }
    }
    lines
}

fn log_total_lines(view: &crate::frontend::tui::state::LogView, width: usize) -> usize {
    log_body_lines(view, width).len()
}

/// Scrolls inside the current log page.
pub fn log_scroll(app: &mut App, delta: isize) {
    let width = app.last_width.max(1);
    let Some(view) = &mut app.log_view else {
        return;
    };
    let total = log_total_lines(view, width);
    let max = total.saturating_sub(1) as isize;
    view.scroll = (view.scroll as isize + delta).clamp(0, max) as usize;
}

/// PageUp/PageDown inside the log: scroll the page first, then move to the
/// previous/next file page.
pub fn log_page_nav(app: &mut App, forward: bool) {
    let width = app.last_width.max(1);
    let step = app.last_height.saturating_sub(2).max(1);
    let action = match &app.log_view {
        Some(view) => {
            let total = log_total_lines(view, width);
            if forward {
                if view.scroll + step < total {
                    Some(NavAction::Scroll(step))
                } else if !view.eof {
                    Some(NavAction::NextFile)
                } else {
                    None
                }
            } else if view.scroll == 0 {
                Some(NavAction::PrevFile)
            } else {
                Some(NavAction::ScrollBack(step))
            }
        }
        None => None,
    };
    match action {
        Some(NavAction::Scroll(step)) => {
            if let Some(view) = &mut app.log_view {
                view.scroll += step;
            }
        }
        Some(NavAction::ScrollBack(step)) => {
            if let Some(view) = &mut app.log_view {
                view.scroll = view.scroll.saturating_sub(step);
            }
        }
        Some(NavAction::NextFile) => log::next_page(app),
        Some(NavAction::PrevFile) => log::prev_page(app),
        None => {}
    }
}

enum NavAction {
    Scroll(usize),
    ScrollBack(usize),
    NextFile,
    PrevFile,
}

fn draw_input(frame: &mut Frame, app: &mut App, editor: &Editor, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(app.theme.accent));
    let inner = block.inner(area);
    app.input_width = inner.width as usize;
    let layout = editor.layout(app.input_width);
    let mut display: Vec<String> = editor
        .images()
        .iter()
        .map(|image| format!("[image] {}", image.name))
        .collect();
    let image_lines = display.len();
    display.extend(layout.lines.iter().map(|line| line.text.clone()));
    let cursor_display = image_lines + layout.cursor_line;
    let height = inner.height as usize;
    let start = cursor_display.saturating_sub(height.saturating_sub(1));
    let visible = display[start.min(display.len())..(start + height).min(display.len())].join("\n");
    frame.render_widget(
        Paragraph::new(visible)
            .block(block)
            .style(Style::default().fg(app.theme.text)),
        area,
    );
    // The input cursor is only shown when the input actually owns the focus and
    // no log sub-view is open.
    if height > 0
        && app.focus == Focus::Input
        && app.log_view.is_none()
        && !app.help_open
        && app.transcript_search.is_none()
    {
        let x = inner.x + (layout.cursor_cell as u16).min(inner.width.saturating_sub(1));
        let y = inner.y + (cursor_display - start) as u16;
        frame.set_cursor_position((x, y));
    }
}

fn activity_line(app: &App, editor: &Editor, width: usize) -> Line<'static> {
    let theme = app.theme;
    let state = if app.quit_after_turn {
        "Waiting to exit"
    } else if app.cancelling {
        "Cancelling"
    } else if app.running {
        "Working"
    } else if app.log_view.is_some() {
        "Log"
    } else if !app.follow {
        "Reading"
    } else if app.focus == Focus::Transcript {
        "Records"
    } else if !editor.is_empty() {
        "Draft"
    } else {
        "Ready"
    };
    let state_style = if app.running && !app.quit_after_turn {
        theme.accent
    } else {
        theme.secondary
    };
    let mut spans = vec![Span::styled(state, Style::default().fg(state_style))];
    if app.running && app.log_view.is_some() {
        spans.push(Span::styled(" · Log", Style::default().fg(theme.secondary)));
    } else if app.running && !app.follow {
        spans.push(Span::styled(
            " · Reading",
            Style::default().fg(theme.secondary),
        ));
    }
    if app.unread_output {
        spans.push(Span::styled(
            " · New output",
            Style::default().fg(theme.accent),
        ));
    } else if app.running
        && app.follow
        && !app.quit_after_turn
        && let Some(Record::Tool {
            name,
            arguments,
            output: None,
            ..
        }) = app.records.last()
    {
        let headline = tool_headline(name, arguments, app.working_directory.as_deref());
        let detail = headline.lines().next().unwrap_or("");
        let available = width.saturating_sub(spans.iter().map(Span::width).sum());
        if available >= name.width() + 3 {
            let activity = if detail.width() + 3 <= available {
                detail
            } else {
                name
            };
            spans.push(Span::styled(
                format!(" · {activity}"),
                Style::default().fg(theme.secondary),
            ));
        }
    }
    Line::from(spans)
}

fn draw_status(
    frame: &mut Frame,
    app: &mut App,
    editor: &Editor,
    area: Rect,
    activity_visible: bool,
) {
    let theme = app.theme;
    app.help_button_area = Rect::default();
    app.session_button_area = Rect::default();
    app.latest_button_area = Rect::default();
    let fallback = if activity_visible {
        Line::default()
    } else {
        activity_line(app, editor, area.width as usize)
    };
    let state_width = fallback.width() as u16;
    let help_label = if app.help_open { "Close help" } else { "Help" };
    let help_width = help_label.width() as u16;
    let latest_width = "Latest".width() as u16;
    let separator = usize::from(state_width > 0) * 3;
    let info = app.context.as_ref().map(context_label).filter(|label| {
        label.width() <= (area.width as usize).saturating_sub(state_width as usize + separator)
    });
    // Context is the priority information: reserve its cells before deciding
    // whether the optional Latest/Help actions still fit. Their keyboard paths
    // stay available when narrow windows drop the buttons.
    let reserved = state_width as usize
        + info
            .as_ref()
            .map(|label| separator + label.width())
            .unwrap_or(0);
    let show_help = area.width as usize >= reserved + help_width as usize + 2;
    let show_latest = show_help
        && (!app.follow || app.log_view.is_some())
        && !app.help_open
        && area.width as usize >= reserved + help_width as usize + latest_width as usize + 6;
    let actions_width =
        if show_help { help_width + 2 } else { 0 } + if show_latest { latest_width + 2 } else { 0 };
    let left = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(actions_width),
        area.height,
    );
    let mut spans = fallback.spans;
    if let Some(info) = info {
        if state_width > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(theme.secondary)));
        }
        spans.push(Span::styled(info, Style::default().fg(theme.secondary)));
    }
    if let Some(metadata) = &app.session {
        let current_width = Line::from(spans.clone()).width();
        let model_width = app.context.as_ref().map_or(0, |context| {
            context
                .model
                .rsplit('/')
                .next()
                .unwrap_or(&context.model)
                .width()
                + 3
        });
        let remaining = (left.width as usize)
            .saturating_sub(current_width + 3 + model_width)
            .min(28);
        let title = wrap_bytes(&metadata.title, remaining.max(1))
            .first()
            .map(|(part, _)| part.clone())
            .unwrap_or_default();
        if title.width() <= remaining {
            if current_width > 0 {
                spans.push(Span::styled(" · ", Style::default().fg(theme.secondary)));
            }
            let x = left.x + Line::from(spans.clone()).width() as u16;
            app.session_button_area = Rect::new(x, left.y, title.width() as u16, left.height);
            spans.push(Span::styled(title, Style::default().fg(theme.accent)));
        }
    }
    let line = Line::from(spans);
    let model = app.context.as_ref().and_then(|context| {
        let available = (left.width as usize).saturating_sub(line.width() + 3);
        [
            context.model.as_str(),
            context.model.rsplit('/').next().unwrap_or(&context.model),
        ]
        .into_iter()
        .find(|model| model.width() <= available)
    });
    frame.render_widget(Paragraph::new(line), left);
    if let Some(model) = model {
        let width = model.width() as u16;
        frame.render_widget(
            Paragraph::new(model).style(Style::default().fg(theme.secondary)),
            Rect::new(left.right() - width, left.y, width, left.height),
        );
    }
    if show_latest {
        app.latest_button_area = Rect::new(
            area.right() - help_width - 2 - latest_width,
            area.y,
            latest_width,
            area.height,
        );
        frame.render_widget(
            Paragraph::new("Latest").style(Style::default().fg(theme.accent)),
            app.latest_button_area,
        );
    }
    if show_help {
        app.help_button_area =
            Rect::new(area.right() - help_width, area.y, help_width, area.height);
        frame.render_widget(
            Paragraph::new(help_label).style(Style::default().fg(theme.accent)),
            app.help_button_area,
        );
    }
}

fn format_tokens(tokens: usize) -> String {
    let (value, unit) = if tokens >= 1_000_000 {
        (tokens as f64 / 1_000_000.0, "M")
    } else if tokens >= 1_000 {
        (tokens as f64 / 1_000.0, "k")
    } else {
        return tokens.to_string();
    };
    format!("{}{unit}", format!("{value:.1}").trim_end_matches(".0"))
}

fn context_label(context: &ContextInfo) -> String {
    let window = context.window.map_or_else(|| "?".to_owned(), format_tokens);
    let percent = context
        .tokens
        .zip(context.window.filter(|window| *window > 0))
        .map_or_else(
            || "?%".to_owned(),
            |(used, window)| format!("{:.0}%", used as f64 / window as f64 * 100.0),
        );
    format!("{percent} / {window}")
}

/// Lays out one display key. Activity headers are rebuilt on every call so a
/// tool that finishes can never leave a stale summary behind.
pub fn record_layout(app: &mut App, index: usize, width: usize) -> Vec<VisualLine> {
    if matches!(app.records.get(index), Some(Record::Activity { .. })) {
        // Activity headers are rebuilt fresh, but they still share the common
        // selection marker path so keyboard selection stays visible.
        let mut lines = build_activity(app, index, width);
        if app.focus == Focus::Transcript && index == app.selected {
            overlay_marker(&mut lines);
        }
        return lines;
    }
    let (expanded, has_output) = match app.records.get(index) {
        Some(Record::Tool {
            expanded, output, ..
        }) => (*expanded, output.is_some()),
        Some(Record::Reasoning { expanded, .. }) => (*expanded, false),
        _ => (false, false),
    };
    let mut lines = match app.layouts.get(index) {
        Some(Some(cached))
            if cached.width == width
                && cached.expanded == expanded
                && cached.has_output == has_output =>
        {
            cached.lines.clone()
        }
        _ => {
            let lines = build_record(app, index, width);
            if index < app.layouts.len() {
                app.layouts[index] = Some(CachedLayout {
                    width,
                    expanded,
                    has_output,
                    lines: lines.clone(),
                });
            }
            lines
        }
    };
    if app.focus == Focus::Transcript && index == app.selected {
        overlay_marker(&mut lines);
    }
    lines
}

fn overlay_marker(lines: &mut [VisualLine]) {
    // User blocks start with a background padding row; the marker must land on
    // the first line that actually carries content.
    let Some(first) = lines.iter_mut().find(|visual| !is_blank_line(visual)) else {
        return;
    };
    let Some(span) = first.line.spans.first_mut() else {
        return;
    };
    let content = span.content.to_mut();
    if let Some(rest) = content.strip_prefix(' ') {
        *content = format!("▌{rest}");
    }
}

fn build_record(app: &App, index: usize, width: usize) -> Vec<VisualLine> {
    let width = width.max(1);
    let mut out = Vec::new();
    let Some(record) = app.records.get(index) else {
        return out;
    };
    // A one-cell placeholder; the selection marker replaces it when needed.
    let marker = " ";
    let theme = app.theme;
    match record {
        Record::User { text, images } => {
            push_user_block(&mut out, text, images, width, marker, theme);
        }
        Record::Assistant { text } => {
            // First line and every wrapped line share the same 3-cell body margin.
            let padding = if width >= 4 { "   " } else { "" };
            // One blank separates the final answer from a collapsed or expanded
            // Activity process; the anchor stays at the answer's first source.
            if answer_after_process(app, index) {
                out.push(VisualLine {
                    line: Line::from(String::new()),
                    source_start: 0,
                });
            }
            markdown_layout::push_markdown(&mut out, text, width, padding, theme);
            push_message_spacing(&mut out, text.len());
        }
        Record::Reasoning { text, expanded, .. } => {
            let padding = if width >= 4 { "   " } else { "" };
            let budget = width.saturating_sub(padding.len());
            let arrow = if *expanded { "▾" } else { "▸" };
            let mut spans = vec![Span::styled(padding.to_owned(), Style::default())];
            if budget >= 1 {
                spans.push(Span::styled(
                    arrow.to_owned(),
                    Style::default().fg(theme.accent),
                ));
                let mut label = String::new();
                let mut cells = 1;
                for grapheme in " Thought".graphemes(true) {
                    let glyph = grapheme.width().max(1);
                    if cells + glyph > budget {
                        break;
                    }
                    label.push_str(grapheme);
                    cells += glyph;
                }
                if !label.is_empty() {
                    spans.push(Span::styled(label, Style::default().fg(theme.secondary)));
                }
            }
            out.push(VisualLine {
                line: Line::from(spans),
                source_start: 0,
            });
            if *expanded {
                // Reserve offset zero for the disclosure so expanding retains its anchor.
                push_visual(
                    &mut out,
                    text,
                    1,
                    width,
                    padding,
                    padding,
                    Style::default().fg(theme.secondary),
                );
                push_message_spacing(&mut out, text.len() + 1);
            }
        }
        Record::Tool {
            name,
            arguments,
            output,
            expanded,
            ..
        } => {
            let (mark, failed) = tool_status(output.as_ref());
            let (mark_style, text_style) = if failed {
                (
                    Style::default().fg(theme.error),
                    Style::default().fg(theme.error),
                )
            } else if output.is_none() {
                (
                    Style::default().fg(theme.warning),
                    Style::default().fg(theme.secondary),
                )
            } else {
                (
                    Style::default().fg(theme.accent),
                    Style::default().fg(theme.secondary),
                )
            };
            let headline = tool_headline(name, arguments, app.working_directory.as_deref());
            let first_line = headline.lines().next().unwrap_or("");
            let prefix = format!("{marker}│ ");
            let mark_text = format!("{mark} ");
            let available = width.saturating_sub(
                UnicodeWidthStr::width(prefix.as_str())
                    + UnicodeWidthStr::width(mark_text.as_str()),
            );
            let headline = if first_line.width() > available || headline.contains('\n') {
                let mut summary = String::new();
                let mut cells = 0;
                for grapheme in first_line.graphemes(true) {
                    if cells + grapheme.width() > available.saturating_sub(1) {
                        break;
                    }
                    summary.push_str(grapheme);
                    cells += grapheme.width();
                }
                summary.push('…');
                summary
            } else {
                headline
            };
            out.push(VisualLine {
                line: Line::from(vec![
                    Span::styled(prefix, Style::default()),
                    Span::styled(mark_text.clone(), mark_style),
                    Span::styled(headline.clone(), text_style),
                ]),
                source_start: 0,
            });
            let mut base = mark_text.len() + headline.len() + 1;
            if *expanded {
                base = push_visual(
                    &mut out,
                    &format!("args {arguments}"),
                    base,
                    width,
                    "    ",
                    "    ",
                    Style::default().fg(theme.secondary),
                );
                if let Some(output) = output {
                    if let Some(meta) = tool_meta(output) {
                        base = push_visual(
                            &mut out,
                            &meta,
                            base,
                            width,
                            "    ",
                            "    ",
                            Style::default().fg(theme.secondary),
                        );
                    }
                    match output.get("output_path").and_then(Value::as_str) {
                        Some(path) => {
                            base = push_visual(
                                &mut out,
                                &format!("log {path}"),
                                base,
                                width,
                                "    ",
                                "    ",
                                Style::default().fg(theme.secondary),
                            );
                            push_visual(
                                &mut out,
                                &log::first_page(path),
                                base,
                                width,
                                "    ",
                                "    ",
                                Style::default().fg(theme.secondary),
                            );
                        }
                        None => {
                            push_visual(
                                &mut out,
                                &bounded_json(output),
                                base,
                                width,
                                "    ",
                                "    ",
                                Style::default().fg(theme.secondary),
                            );
                        }
                    }
                }
            }
        }
        Record::Activity { .. } => {
            // Handled by record_layout; build_record must not cache it.
            out = build_activity(app, index, width);
        }
        Record::Compaction {
            tokens_before,
            tokens_after,
        } => {
            push_visual(
                &mut out,
                &format!("compacted ~{tokens_before} → ~{tokens_after} tokens"),
                0,
                width,
                &format!("{marker} "),
                "  ",
                Style::default().fg(theme.secondary),
            );
        }
        Record::Error { message } => {
            push_visual(
                &mut out,
                message,
                0,
                width,
                &format!("{marker}! "),
                "   ",
                Style::default().fg(theme.error),
            );
        }
    }
    out
}

/// Activity disclosure: one header line with real process counts and failure
/// state. The members themselves render as their own records while expanded.
fn build_activity(app: &App, index: usize, width: usize) -> Vec<VisualLine> {
    let width = width.max(1);
    let theme = app.theme;
    let Some(Record::Activity {
        end,
        expanded,
        failed,
        interrupted,
        ..
    }) = app.records.get(index)
    else {
        return Vec::new();
    };
    let mut tools = 0usize;
    let mut thoughts = 0usize;
    let mut failures = 0usize;
    for member in app.activity_members(index) {
        match &app.records[member] {
            Record::Tool { output, .. } => {
                tools += 1;
                if tool_failed(output.as_ref()) {
                    failures += 1;
                }
            }
            Record::Reasoning { .. } => thoughts += 1,
            _ => {}
        }
    }
    let running = end.is_none();
    let mut parts = Vec::new();
    if tools > 0 {
        parts.push(format!(
            "{tools} {}",
            if tools == 1 { "tool" } else { "tools" }
        ));
    }
    if thoughts > 0 {
        parts.push(format!(
            "{thoughts} {}",
            if thoughts == 1 { "thought" } else { "thoughts" }
        ));
    }
    let status = if *interrupted {
        Some("interrupted")
    } else if *failed {
        Some("failed")
    } else {
        None
    };
    let mut spans = Vec::new();
    let padding = if width >= 4 { "   " } else { "" };
    if !padding.is_empty() {
        spans.push(Span::styled(padding.to_owned(), Style::default()));
    }
    // The failure/interrupted state lives directly on the label and comes
    // before optional counts, so clipping can never hide the error.
    spans.push(Span::styled(
        format!("{}Activity", if *expanded { "▾ " } else { "▸ " }),
        Style::default().fg(if status.is_some() {
            theme.error
        } else {
            theme.accent
        }),
    ));
    if let Some(word) = status {
        spans.push(Span::styled(
            format!(" · {word}"),
            Style::default().fg(theme.error),
        ));
    } else if failures > 0 {
        spans.push(Span::styled(
            format!(" · {failures} failed"),
            Style::default().fg(theme.error),
        ));
    } else if running {
        spans.push(Span::styled(
            " · running",
            Style::default().fg(theme.secondary),
        ));
    }
    if !parts.is_empty() {
        spans.push(Span::styled(
            format!(" · {}", parts.join(" · ")),
            Style::default().fg(theme.secondary),
        ));
    }
    let line = Line::from(clip_spans(spans, width));
    vec![VisualLine {
        line,
        source_start: 0,
    }]
}

/// Clips styled spans to an exact cell budget while keeping each span's style.
/// An ellipsis marks truncation; the clipped tail keeps the original style so
/// an error header stays visually identifiable.
fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let budget = width.saturating_sub(1);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    'outer: for span in spans {
        let mut text = String::new();
        for grapheme in span.content.graphemes(true) {
            let glyph = UnicodeWidthStr::width(grapheme).max(1);
            if used + glyph > budget {
                truncated = true;
                if !text.is_empty() {
                    out.push(Span::styled(text, span.style));
                }
                break 'outer;
            }
            text.push_str(grapheme);
            used += glyph;
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span.style));
        }
    }
    if truncated {
        let style = out.last().map(|span| span.style).unwrap_or_default();
        out.push(Span::styled("…", style));
    }
    out
}

/// Appends one blank line after a message so turns breathe, unless the message
/// already ends blank. The blank uses the raw EOF offset, so it never repeats
/// `source_start = 0` and paging across the record boundary stays monotonic.
fn push_message_spacing(out: &mut Vec<VisualLine>, raw_len: usize) {
    if out.is_empty() || raw_len == 0 || out.last().is_some_and(is_blank_line) {
        return;
    }
    out.push(VisualLine {
        line: Line::from(String::new()),
        source_start: raw_len,
    });
}

/// Draws one user message as a full-width background block with a little
/// horizontal and vertical padding. Every padded cell carries the background;
/// the padding is display-only and never part of the copy source.
fn push_user_block(
    out: &mut Vec<VisualLine>,
    text: &str,
    images: &[ImageAttachment],
    width: usize,
    marker: &str,
    theme: theme::Theme,
) {
    let style = Style::default().fg(theme.user_fg).bg(theme.user_bg);
    push_background_line(out, width, 0, theme);
    let (first_prefix, continuation, prefix_width) = if width >= 4 {
        (format!("{marker}› "), "   ".to_owned(), 3)
    } else {
        (String::new(), String::new(), 0)
    };
    // Keep one cell of right inner padding at normal widths; narrow panes may
    // drop it rather than lose content.
    let right_padding = usize::from(width >= 8);
    let body_width = width.saturating_sub(prefix_width + right_padding).max(1);
    let mut first_content = true;
    if !text.is_empty() {
        for (body, start) in wrap_bytes(text, body_width) {
            let prefix = if first_content {
                first_prefix.as_str()
            } else {
                continuation.as_str()
            };
            let mut line = Line::from(Span::styled(format!("{prefix}{body}"), style));
            pad_line(&mut line, width, style);
            out.push(VisualLine {
                line,
                source_start: start,
            });
            first_content = false;
        }
    }
    // Attachment names are displayed instead of the base64 payload.
    for (index, image) in images.iter().enumerate() {
        let prefix = if first_content {
            first_prefix.as_str()
        } else {
            continuation.as_str()
        };
        let mut line = Line::from(Span::styled(
            format!("{prefix}[image] {}", image.name),
            style,
        ));
        pad_line(&mut line, width, style);
        out.push(VisualLine {
            line,
            source_start: text.len() + index,
        });
        first_content = false;
    }
    if first_content {
        let mut line = Line::from(Span::styled(first_prefix.clone(), style));
        pad_line(&mut line, width, style);
        out.push(VisualLine {
            line,
            source_start: 0,
        });
    }
    push_background_line(out, width, text.len() + images.len() + 1, theme);
}

fn push_background_line(
    out: &mut Vec<VisualLine>,
    width: usize,
    source: usize,
    theme: theme::Theme,
) {
    out.push(VisualLine {
        line: Line::from(Span::styled(
            " ".repeat(width.max(1)),
            Style::default().bg(theme.user_bg),
        )),
        source_start: source,
    });
}

fn pad_line(line: &mut Line<'static>, width: usize, style: Style) {
    let used: usize = line
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat(width - used), style));
    }
}

/// Whether an Assistant record is the first answer after a finished Activity
/// process and therefore needs one leading separator blank.
fn answer_after_process(app: &App, index: usize) -> bool {
    index.checked_sub(1).is_some_and(|previous| {
        app.visible_activity_for(previous)
            .is_some_and(|activity| app.activity_members(activity).end == index)
    })
}

fn is_blank_line(visual: &VisualLine) -> bool {
    visual
        .line
        .spans
        .iter()
        .all(|span| span.content.trim().is_empty())
}

fn find_line(layout: &[VisualLine], offset: usize) -> usize {
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

/// Enters the transcript. The selected record is always a visible display key.
pub fn focus_transcript(app: &mut App) {
    app.focus = Focus::Transcript;
    let keys = app.visible_record_keys();
    let Some(last) = keys.last().copied() else {
        app.selected = 0;
        return;
    };
    app.selected = if app.follow {
        last
    } else {
        let target = app
            .anchor
            .map(|anchor| anchor.record)
            .unwrap_or(app.selected);
        keys.iter()
            .copied()
            .rfind(|key| *key <= target)
            .unwrap_or(last)
    };
}

/// Moves the transcript selection by one visible item and scrolls the selected
/// record into view. Hidden/empty records are skipped.
pub fn select(app: &mut App, delta: isize) {
    let keys = app.visible_record_keys();
    if keys.is_empty() {
        app.selected = 0;
        return;
    }
    let current = keys
        .iter()
        .position(|key| *key == app.selected)
        .or_else(|| keys.iter().position(|key| *key >= app.selected))
        .unwrap_or(keys.len() - 1);
    let next = (current as isize + delta).clamp(0, keys.len() as isize - 1) as usize;
    app.selected = keys[next];
    if app.focus == Focus::Transcript {
        app.follow = false;
        app.anchor = Some(Anchor {
            record: app.selected,
            offset: 0,
        });
        app.visual_anchor = None;
    }
}

/// The raw source for the selected record, used by the copy action. Selecting
/// an Activity copies the joined raw sources of its process members.
pub fn copy_source(app: &App) -> Option<String> {
    match app.records.get(app.selected)? {
        Record::User { text, .. } | Record::Assistant { text } | Record::Reasoning { text, .. } => {
            Some(text.clone())
        }
        Record::Tool { arguments, .. } => Some(tool_source(arguments)),
        Record::Error { message } => Some(message.clone()),
        Record::Activity { .. } => {
            let sources: Vec<String> = app
                .activity_members(app.selected)
                .filter_map(|index| match &app.records[index] {
                    Record::Assistant { text } | Record::Reasoning { text, .. } => {
                        Some(text.clone())
                    }
                    Record::Tool { arguments, .. } => Some(tool_source(arguments)),
                    _ => None,
                })
                .collect();
            (!sources.is_empty()).then(|| sources.join("\n\n"))
        }
        Record::Compaction { .. } => None,
    }
}

fn tool_source(arguments: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(arguments).ok();
    let command = parsed
        .as_ref()
        .and_then(|value| value.get("command").and_then(Value::as_str));
    command
        .map(str::to_owned)
        .unwrap_or_else(|| arguments.to_owned())
}

/// Copies just the code blocks of the selected message, or a tool's captured
/// output when a tool or Activity is selected.
pub fn copy_code_source(app: &App) -> Option<String> {
    match app.records.get(app.selected)? {
        Record::Assistant { text } | Record::Reasoning { text, .. } => {
            let blocks = markdown::code_blocks(text);
            (!blocks.is_empty()).then(|| blocks.join("\n"))
        }
        Record::Tool { output, .. } => tool_output(output.as_ref()),
        Record::Activity { .. } => {
            let blocks: Vec<String> = app
                .activity_members(app.selected)
                .filter_map(|index| match &app.records[index] {
                    Record::Assistant { text } | Record::Reasoning { text, .. } => {
                        let blocks = markdown::code_blocks(text);
                        (!blocks.is_empty()).then(|| blocks.join("\n"))
                    }
                    Record::Tool { output, .. } => tool_output(output.as_ref()),
                    _ => None,
                })
                .collect();
            (!blocks.is_empty()).then(|| blocks.join("\n\n"))
        }
        _ => None,
    }
}

fn tool_output(output: Option<&Value>) -> Option<String> {
    output
        .and_then(|value| value.get("output"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Reading position at the top of the current follow-mode viewport.
/// Maps an anchor onto the display key that actually renders it. Members of a
/// collapsed Activity resolve to their marker; hidden/empty records resolve to
/// the next visible key so paging can never stall.
fn normalize_anchor_key(app: &App, keys: &[usize], anchor: Anchor) -> Anchor {
    if keys.contains(&anchor.record) {
        return anchor;
    }
    if let Some(marker) = app.visible_activity_for(anchor.record)
        && keys.contains(&marker)
    {
        return Anchor {
            record: marker,
            offset: 0,
        };
    }
    let key = keys
        .iter()
        .copied()
        .find(|key| *key >= anchor.record)
        .or_else(|| keys.last().copied())
        .unwrap_or(anchor.record);
    Anchor {
        record: key,
        offset: 0,
    }
}

/// Resolves the exact row the anchored viewport starts at. The width-keyed
/// visual anchor wins while valid; source offsets remain the reflow fallback.
fn anchored_start(app: &mut App, keys: &[usize], width: usize) -> (usize, usize) {
    if let Some(visual) = app.visual_anchor
        && visual.width == width
        && let Some(position) = keys.iter().position(|key| *key == visual.key.record)
    {
        let layout = record_layout(app, visual.key.record, width);
        if visual.key.line < layout.len() {
            return (position, visual.key.line);
        }
    }
    let anchor = normalize_anchor_key(
        app,
        keys,
        app.anchor.unwrap_or(Anchor {
            record: keys[0],
            offset: 0,
        }),
    );
    let position = keys
        .iter()
        .position(|key| *key == anchor.record)
        .unwrap_or(0);
    let line = find_line(&record_layout(app, keys[position], width), anchor.offset);
    app.visual_anchor = Some(VisualAnchor {
        key: RowKey {
            record: keys[position],
            line,
        },
        width,
    });
    (position, line)
}

/// Exact top visible row for the current follow/anchored state at `width`.
fn current_top_row(app: &mut App, width: usize) -> Option<RowKey> {
    if let Some(visual) = app.visual_anchor
        && visual.width == width
    {
        let keys = app.visible_record_keys();
        if keys.contains(&visual.key.record) {
            let layout = record_layout(app, visual.key.record, width);
            if visual.key.line < layout.len() {
                return Some(visual.key);
            }
        }
    }
    if let Some(anchor) = app.anchor {
        let keys = app.visible_record_keys();
        let normalized = normalize_anchor_key(app, &keys, anchor);
        let position = keys.iter().position(|key| *key == normalized.record)?;
        let layout = record_layout(app, keys[position], width);
        let line = find_line(&layout, normalized.offset);
        return Some(RowKey {
            record: keys[position],
            line,
        });
    }
    top_row(app, width)
}

/// Exact top row of the current follow-mode viewport.
fn top_row(app: &mut App, width: usize) -> Option<RowKey> {
    let keys = app.visible_record_keys();
    if keys.is_empty() {
        return None;
    }
    let height = app.last_height.max(1);
    let mut total = 0usize;
    let mut index = keys.len();
    while index > 0 && total < height {
        index -= 1;
        total += record_layout(app, keys[index], width).len();
    }
    let key = keys[index];
    let layout = record_layout(app, key, width);
    if layout.is_empty() {
        return None;
    }
    let local = total.saturating_sub(height).min(layout.len() - 1);
    Some(RowKey {
        record: key,
        line: local,
    })
}

fn row_offset(app: &mut App, key: RowKey, width: usize) -> usize {
    record_layout(app, key.record, width)
        .get(key.line)
        .map(|visual| visual.source_start)
        .unwrap_or(0)
}

fn previous_row(app: &mut App, keys: &[usize], width: usize, current: RowKey) -> Option<RowKey> {
    if current.line > 0 {
        return Some(RowKey {
            record: current.record,
            line: current.line - 1,
        });
    }
    let position = keys.iter().position(|key| *key == current.record)?;
    let previous = *keys.get(position.checked_sub(1)?)?;
    let layout = record_layout(app, previous, width);
    Some(RowKey {
        record: previous,
        line: layout.len().saturating_sub(1),
    })
}

fn next_row(app: &mut App, keys: &[usize], width: usize, current: RowKey) -> Option<RowKey> {
    let layout = record_layout(app, current.record, width);
    if current.line + 1 < layout.len() {
        return Some(RowKey {
            record: current.record,
            line: current.line + 1,
        });
    }
    let position = keys.iter().position(|key| *key == current.record)?;
    let next = *keys.get(position + 1)?;
    Some(RowKey {
        record: next,
        line: 0,
    })
}

/// Scrolls the reading viewport up by `page` visual rows, preserving exact row
/// identity so duplicate source offsets cannot shift or skip rows.
pub fn page_up(app: &mut App, page: usize) {
    if app.records.is_empty() {
        return;
    }
    let width = app.last_width.max(1);
    let keys = app.visible_record_keys();
    if keys.is_empty() {
        return;
    }
    let Some(mut current) = current_top_row(app, width) else {
        return;
    };
    let mut remaining = page;
    while remaining > 0 {
        let Some(previous) = previous_row(app, &keys, width, current) else {
            break;
        };
        current = previous;
        remaining -= 1;
    }
    app.follow = false;
    app.visual_anchor = Some(VisualAnchor {
        key: current,
        width,
    });
    app.anchor = Some(Anchor {
        record: current.record,
        offset: row_offset(app, current, width),
    });
}

/// Scrolls the reading viewport down by `page` visual rows; reaching the
/// bottom resumes following.
pub fn page_down(app: &mut App, page: usize) {
    if app.records.is_empty() {
        return;
    }
    if app.follow && app.anchor.is_none() {
        return;
    }
    let width = app.last_width.max(1);
    let keys = app.visible_record_keys();
    if keys.is_empty() {
        app.end();
        return;
    }
    let Some(mut current) = current_top_row(app, width) else {
        app.end();
        return;
    };
    let mut remaining = page;
    while remaining > 0 {
        match next_row(app, &keys, width, current) {
            Some(next) => {
                current = next;
                remaining -= 1;
            }
            None => {
                app.end();
                return;
            }
        }
    }
    if let Some(tail) = top_row(app, width) {
        // The exact follow-mode top across records is the furthest the reading
        // viewport may scroll without jumping backward on release.
        if current >= tail {
            app.end();
            return;
        }
    } else {
        app.end();
        return;
    }
    app.visual_anchor = Some(VisualAnchor {
        key: current,
        width,
    });
    app.anchor = Some(Anchor {
        record: current.record,
        offset: row_offset(app, current, width),
    });
}

/// Visible user-message boundaries as `(display position, record)`. This only
/// walks the existing display order; historical User layouts are not built.
fn user_positions(app: &App) -> Vec<(usize, usize)> {
    app.visible_record_keys()
        .into_iter()
        .enumerate()
        .filter(|(_, record)| matches!(app.records.get(*record), Some(Record::User { .. })))
        .collect()
}

/// First content row of the current user record, resolved only while the top
/// record itself is a User (previous navigation needs its boundary line).
fn current_user_start(app: &mut App, current: RowKey, width: usize) -> Option<usize> {
    if !matches!(app.records.get(current.record), Some(Record::User { .. })) {
        return None;
    }
    record_layout(app, current.record, width)
        .iter()
        .position(|visual| !is_blank_line(visual))
}

/// Jumps to a user boundary, resolving its first content row once.
fn jump_to_user(app: &mut App, record: usize, width: usize) {
    let layout = record_layout(app, record, width);
    let line = layout
        .iter()
        .position(|visual| !is_blank_line(visual))
        .unwrap_or(0);
    let offset = layout.get(line).map_or(0, |visual| visual.source_start);
    app.follow = false;
    app.visual_anchor = Some(VisualAnchor {
        key: RowKey { record, line },
        width,
    });
    app.anchor = Some(Anchor { record, offset });
}

/// Jumps the viewport to the previous real user-message boundary. From inside
/// a long user message it returns that message's start; from its first content
/// row it moves to the prior user message.
pub fn previous_user(app: &mut App) {
    if app.records.is_empty() {
        app.notify("No messages yet.".to_owned());
        return;
    }
    let width = app.last_width.max(1);
    let Some(current) = current_top_row(app, width) else {
        app.notify("No earlier message.".to_owned());
        return;
    };
    let keys = app.visible_record_keys();
    let Some(position) = keys.iter().position(|key| *key == current.record) else {
        app.notify("No earlier message.".to_owned());
        return;
    };
    if let Some(start) = current_user_start(app, current, width)
        && start < current.line
    {
        jump_to_user(app, current.record, width);
        return;
    }
    let target = user_positions(app)
        .into_iter()
        .rev()
        .find(|(start_position, _)| *start_position < position)
        .map(|(_, record)| record);
    match target {
        Some(record) => jump_to_user(app, record, width),
        None => app.notify("Already at the first message.".to_owned()),
    }
}

/// Jumps the viewport to the next real user-message boundary after the current
/// top record.
pub fn next_user(app: &mut App) {
    if app.records.is_empty() {
        app.notify("No messages yet.".to_owned());
        return;
    }
    let width = app.last_width.max(1);
    let Some(current) = current_top_row(app, width) else {
        app.notify("No later message.".to_owned());
        return;
    };
    let keys = app.visible_record_keys();
    let Some(position) = keys.iter().position(|key| *key == current.record) else {
        app.notify("No later message.".to_owned());
        return;
    };
    let target = user_positions(app)
        .into_iter()
        .find(|(start_position, _)| *start_position > position)
        .map(|(_, record)| record);
    match target {
        Some(record) => jump_to_user(app, record, width),
        None => app.notify("Already at the latest message.".to_owned()),
    }
}

fn push_visual(
    out: &mut Vec<VisualLine>,
    text: &str,
    base: usize,
    width: usize,
    first_prefix: &str,
    continuation_prefix: &str,
    style: Style,
) -> usize {
    let prefix_width =
        UnicodeWidthStr::width(first_prefix).max(UnicodeWidthStr::width(continuation_prefix));
    let body_width = width.saturating_sub(prefix_width).max(1);
    for (index, (body, start)) in wrap_bytes(text, body_width).into_iter().enumerate() {
        let prefix = if index == 0 {
            first_prefix
        } else {
            continuation_prefix
        };
        out.push(VisualLine {
            line: Line::from(Span::styled(format!("{prefix}{body}"), style)),
            source_start: base + start,
        });
    }
    base + text.len() + 1
}

/// Status symbol and failure classification for one tool result.
pub fn tool_status(output: Option<&Value>) -> (&'static str, bool) {
    match output {
        None => ("…", false),
        Some(value) if tool_failed(Some(value)) => ("✗", true),
        Some(_) => ("✓", false),
    }
}

fn tool_failed(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return true;
    }
    if let Some(code) = value.get("exit_code") {
        // `null` means the process was killed by a signal, which is a failure.
        if code.is_null() || code.as_i64().is_some_and(|code| code != 0) {
            return true;
        }
    }
    value
        .get("timed_out")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Compact metadata kept visible even when a spilled log is shown.
fn tool_meta(value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        parts.push(format!("error={error}"));
    }
    match value.get("exit_code") {
        Some(code) if code.is_null() => parts.push("exit=signal".to_owned()),
        Some(code) => {
            if let Some(code) = code.as_i64() {
                parts.push(format!("exit={code}"));
            }
        }
        None => {}
    }
    if value
        .get("timed_out")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        parts.push("timed_out=true".to_owned());
    }
    if value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        parts.push("truncated=true".to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// A human-readable action and object built from the tool's real arguments.
/// Paths under the working directory are shortened for display; the raw
/// arguments and output remain available in details and copy.
pub fn tool_headline(name: &str, arguments: &str, working_directory: Option<&Path>) -> String {
    let parsed: Option<Value> = serde_json::from_str(arguments).ok();
    let detail = parsed.as_ref().and_then(|value| {
        value
            .get("command")
            .and_then(Value::as_str)
            .or_else(|| value.get("path").and_then(Value::as_str))
            .or_else(|| value.get("expression").and_then(Value::as_str))
    });
    let (verb, path_like) = match name {
        "read" => ("Read", true),
        "write" => ("Write", true),
        "edit" => ("Edit", true),
        "bash" => ("Run", false),
        "" => ("Tool", false),
        other => (other, false),
    };
    match detail {
        Some(detail) => {
            let detail = if path_like {
                relative_path(detail, working_directory)
            } else {
                detail.to_owned()
            };
            format!("{verb} {detail}")
        }
        None if !arguments.is_empty() => format!("{verb} {arguments}"),
        None => verb.to_owned(),
    }
}

fn relative_path(path: &str, working_directory: Option<&Path>) -> String {
    let Some(directory) = working_directory else {
        return path.to_owned();
    };
    let path = Path::new(path);
    match path.strip_prefix(directory) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn bounded_json(value: &Value) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index >= 40 {
            out.push_str("… (truncated)");
            break;
        }
        if index > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.chars().count() > 4000 {
        out = out.chars().take(4000).collect();
        out.push_str(" …");
    }
    out
}

/// Projects the visible transcript lines for the current anchor/follow state.
pub fn visible_lines(app: &mut App, width: usize, height: usize) -> Vec<Line<'static>> {
    app.disclosure_buttons.clear();
    if app.transcript_search.is_some() {
        transcript_search::refresh(app, width);
    }
    if app.records.is_empty() {
        app.transcript_rows.clear();
        return vec![Line::from(Span::styled(
            "What would you like to work on?",
            Style::default().fg(app.theme.secondary),
        ))];
    }
    let keys = app.visible_record_keys();
    if keys.is_empty() {
        app.transcript_rows.clear();
        return Vec::new();
    }
    let mut visible: Vec<(usize, usize, VisualLine)> = Vec::new();
    if app.follow {
        let mut collected = Vec::new();
        let mut total = 0usize;
        let mut index = keys.len();
        while index > 0 && total < height {
            index -= 1;
            let layout = record_layout(app, keys[index], width);
            total += layout.len();
            collected.push((keys[index], layout));
        }
        collected.reverse();
        for (key, layout) in collected {
            visible.extend(
                layout
                    .into_iter()
                    .enumerate()
                    .map(|(line, visual)| (key, line, visual)),
            );
        }
        if visible.len() > height {
            visible.drain(0..visible.len() - height);
        }
    } else {
        let (mut position, mut start_line) = anchored_start(app, &keys, width);
        while visible.len() < height && position < keys.len() {
            let layout = record_layout(app, keys[position], width);
            for (line, visual) in layout.into_iter().enumerate().skip(start_line) {
                if visible.len() >= height {
                    break;
                }
                visible.push((keys[position], line, visual));
            }
            start_line = 0;
            position += 1;
        }
    }
    let area = app.transcript_area;
    if app.transcript_search.is_some() {
        for (key, line, visual) in &mut visible {
            transcript_search::highlight_line(
                app,
                RowKey {
                    record: *key,
                    line: *line,
                },
                &mut visual.line,
            );
        }
    }
    app.transcript_rows = visible
        .iter()
        .enumerate()
        .map(|(row, (key, line, visual))| ViewportRow {
            key: RowKey {
                record: *key,
                line: *line,
            },
            row: area.y + row as u16,
            cells: visual_cells(visual),
        })
        .collect();
    let App {
        text_selection,
        transcript_rows,
        ..
    } = app;
    text_selection.observe_viewport(transcript_rows);
    for (row, (key, line, visual)) in visible.iter().enumerate() {
        if *line != 0 || row >= area.height as usize {
            continue;
        }
        let padding = if width >= 4 { 3 } else { 0 };
        let end = visual.line.width().min(area.width as usize);
        if end <= padding {
            continue;
        }
        let area = Rect::new(
            area.x + padding as u16,
            area.y + row as u16,
            (end - padding) as u16,
            1,
        );
        match &app.records[*key] {
            Record::Reasoning { .. } | Record::Activity { .. } | Record::Tool { .. } => {
                app.disclosure_buttons.push((area, *key))
            }
            _ => {}
        }
    }
    visible
        .into_iter()
        .map(|(_, _, visual)| visual.line)
        .collect()
}

fn visual_cells(visual: &VisualLine) -> Vec<(String, usize)> {
    visual
        .line
        .spans
        .iter()
        .flat_map(|span| {
            span.content
                .graphemes(true)
                .map(|grapheme| (grapheme.to_owned(), UnicodeWidthStr::width(grapheme).max(1)))
        })
        .collect()
}

/// Updates an active transcript drag from a pointer position.
pub fn selection_drag(app: &mut App, pointer: ratatui::layout::Position) {
    let area = app.transcript_area;
    let viewport = app.transcript_rows.clone();
    app.text_selection.drag_transcript(area, pointer, &viewport);
}

/// One stationary edge auto-scroll step: scroll a single visual row, rebuild
/// the viewport mapping, and extend the selection with the stationary pointer.
/// Returns whether the document moved.
pub fn selection_scroll_tick(app: &mut App) -> bool {
    let Some(edge) = app.text_selection.edge() else {
        return false;
    };
    let width = app.last_width.max(1);
    let height = app.last_height.max(1);
    let before = viewport_bounds(app, width, height);
    if !app.text_selection.is_transcript() {
        return false;
    }
    match edge {
        Edge::Top => page_up(app, 1),
        Edge::Bottom => page_down(app, 1),
    }
    if app.follow {
        // The step reached the document tail and page_down resumed follow;
        // re-freeze the exact viewport before new output can arrive.
        page_up(app, 0);
    }
    let after = viewport_bounds(app, width, height);
    if before == after || !app.text_selection.is_transcript() {
        // Document limit: stop the timer until a new drag intent arrives, and
        // keep the view frozen instead of resuming follow.
        if app.follow {
            page_up(app, 0);
        }
        app.text_selection.clear_edge();
        return false;
    }
    let pointer = app.text_selection.pointer();
    let area = app.transcript_area;
    let viewport = app.transcript_rows.clone();
    app.text_selection.drag_transcript(area, pointer, &viewport);
    app.text_selection.reset_scroll_due();
    true
}

fn viewport_bounds(app: &mut App, width: usize, height: usize) -> Option<(RowKey, RowKey)> {
    refresh_rows(app, width, height);
    let first = app.transcript_rows.first()?.key;
    let last = app.transcript_rows.last()?.key;
    Some((first, last))
}

fn refresh_rows(app: &mut App, width: usize, height: usize) {
    let _ = visible_lines(app, width, height);
}
