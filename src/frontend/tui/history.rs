use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;

use super::{state::App, textwrap};
use crate::input::UserInput;

/// Preview rows shown for one history entry before a truncation marker.
const PREVIEW_LINES: usize = 4;

/// One active local prompt-history search. The composer `Editor` is never
/// touched: only the query, cached match indices (newest first) and the
/// selected match live here, so cancel restores the draft for free.
struct HistorySearch {
    query: String,
    matches: Vec<usize>,
    selected: usize,
}

/// Per-process input history owned by the TUI.
///
/// It is built only from submitted drafts and Ctrl+C-cleared drafts (text plus
/// attachments), never from the conversation, so compaction cannot rewrite it.
/// It is not persisted.
#[derive(Default)]
pub struct InputHistory {
    entries: Vec<UserInput>,
    /// Index of the entry currently recalled, or `None` when not navigating.
    position: Option<usize>,
    search: Option<HistorySearch>,
}

impl InputHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn navigating(&self) -> bool {
        self.position.is_some()
    }

    /// The recalled draft, used to decide whether Up/Down may keep navigating.
    pub fn recalled(&self) -> Option<&UserInput> {
        self.position.and_then(|index| self.entries.get(index))
    }

    /// Adds a submitted or cleared draft. Adjacent duplicates are collapsed.
    pub fn push(&mut self, message: impl Into<UserInput>) {
        let message = message.into();
        if message.is_empty() {
            return;
        }
        if self.entries.last() != Some(&message) {
            self.entries.push(message);
        }
        self.position = None;
        self.search = None;
    }

    /// Older entry, or `None` when there is nothing older to recall.
    pub fn older(&mut self) -> Option<UserInput> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.position {
            None => self.entries.len() - 1,
            Some(0) => return None,
            Some(index) => index - 1,
        };
        self.position = Some(next);
        Some(self.entries[next].clone())
    }

    /// Newer entry, or `None` when navigating past the newest (the caller then
    /// restores an empty draft).
    pub fn newer(&mut self) -> Option<UserInput> {
        let position = self.position?;
        if position + 1 < self.entries.len() {
            self.position = Some(position + 1);
            Some(self.entries[position + 1].clone())
        } else {
            self.position = None;
            None
        }
    }

    pub fn searching(&self) -> bool {
        self.search.is_some()
    }

    /// Ctrl+R: opens an empty search, or steps to an older match when already
    /// open.
    pub fn open_or_older(&mut self) {
        match &mut self.search {
            Some(search) => {
                if search.selected + 1 < search.matches.len() {
                    search.selected += 1;
                }
            }
            None => {
                self.search = Some(HistorySearch {
                    query: String::new(),
                    matches: Vec::new(),
                    selected: 0,
                });
                self.rescan();
            }
        }
    }

    pub fn query_push(&mut self, character: char) {
        if let Some(search) = &mut self.search {
            search.query.push(character);
        }
        self.rescan();
    }

    /// Grapheme-safe query Backspace.
    pub fn query_backspace(&mut self) {
        if let Some(search) = &mut self.search
            && let Some((index, _)) = search.query.grapheme_indices(true).next_back()
        {
            search.query.truncate(index);
        }
        self.rescan();
    }

    /// Extends the query from a paste; the single-line field folds newlines.
    pub fn query_paste(&mut self, text: &str) {
        if let Some(search) = &mut self.search {
            search.query.push_str(&text.replace(['\r', '\n'], " "));
        }
        self.rescan();
    }

    pub fn move_older(&mut self) {
        if let Some(search) = &mut self.search
            && search.selected + 1 < search.matches.len()
        {
            search.selected += 1;
        }
    }

    pub fn move_newer(&mut self) {
        if let Some(search) = &mut self.search {
            search.selected = search.selected.saturating_sub(1);
        }
    }

    /// Esc / Ctrl+C: discard the search and leave the draft and ordinary
    /// navigation position untouched.
    pub fn cancel_search(&mut self) {
        self.search = None;
    }

    /// Enter: accept the selected match into the composer. Returns a clone so
    /// the caller can set the draft; ordinary navigation then continues from
    /// the accepted entry. `None` when there is no match, in which case the
    /// search stays open with the query and draft unchanged.
    pub fn accept_search(&mut self) -> Option<UserInput> {
        let index = {
            let search = self.search.as_ref()?;
            *search.matches.get(search.selected)?
        };
        self.search = None;
        self.position = Some(index);
        Some(self.entries[index].clone())
    }

    /// Recomputes cached match indices (newest first) for the current query.
    fn rescan(&mut self) {
        let Some(search) = &mut self.search else {
            return;
        };
        let query = search.query.to_lowercase();
        search.matches = (0..self.entries.len())
            .rev()
            .filter(|index| {
                query.is_empty() || self.entries[*index].text.to_lowercase().contains(&query)
            })
            .collect();
        search.selected = 0;
    }
}

/// Draws the local history-search overlay above the input when active.
pub fn draw(frame: &mut Frame, app: &App) {
    let Some(search) = app.history.search.as_ref() else {
        return;
    };
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(76);
    if width < 4 {
        return;
    }
    let available = app.input_area.y.saturating_sub(screen.y) as usize;
    // Two borders plus the query line and the exit hint must always remain
    // visible, even with a tall composer above a short terminal.
    if available < 4 {
        return;
    }
    let inner_width = width.saturating_sub(2) as usize;
    let preview_rows = available.saturating_sub(4);

    // Keep the visible end of the query so the caret stays on screen.
    let query_room = inner_width.saturating_sub("Search: ".len() + 1);
    let shown = textwrap::visible_suffix(&search.query, query_room.max(1));
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled("Search: ", Style::default().fg(app.theme.accent)),
        Span::styled(shown, Style::default().fg(app.theme.secondary)),
        Span::styled("▌", Style::default().fg(app.theme.accent)),
    ])];

    let secondary = Style::default().fg(app.theme.secondary);
    let mut preview: Vec<Line> = Vec::new();
    if app.history.entries.is_empty() {
        if preview_rows > 0 {
            preview.push(Line::from(Span::styled(
                "No history yet.".to_owned(),
                secondary,
            )));
        }
    } else if search.matches.is_empty() {
        if preview_rows > 0 {
            preview.push(Line::from(Span::styled(
                "No matching history.".to_owned(),
                secondary,
            )));
        }
    } else if preview_rows > 0 {
        let entry = &app.history.entries[search.matches[search.selected]];
        let badge = !entry.images.is_empty();
        // Reserve the image count first, then fit the text to the rows that
        // remain (up to the fixed cap). A clipped preview keeps a visible
        // omission marker when a row is left for it.
        let mut text_rows = preview_rows
            .saturating_sub(usize::from(badge))
            .min(PREVIEW_LINES);
        let budget = text_rows.max(1) * inner_width.max(1) + 1;
        let (source, clipped) = textwrap::visible_prefix(&entry.text, budget);
        let mut wrapped = textwrap::wrap_bytes(&source, inner_width.max(1));
        let truncated = clipped || wrapped.len() > text_rows;
        let mut ellipsis = false;
        if truncated && text_rows > 0 {
            text_rows -= 1;
            ellipsis = true;
        }
        wrapped.truncate(text_rows);
        for (line, _) in wrapped {
            preview.push(Line::from(Span::styled(line, secondary)));
        }
        if ellipsis {
            preview.push(Line::from(Span::styled("…".to_owned(), secondary)));
        }
        if badge {
            preview.push(Line::from(Span::styled(
                format!(
                    "[{} image{}]",
                    entry.images.len(),
                    if entry.images.len() == 1 { "" } else { "s" }
                ),
                secondary,
            )));
        }
    }
    lines.extend(preview.into_iter().take(preview_rows));
    lines.push(Line::from(Span::styled(
        if inner_width >= 56 {
            "Enter insert · Ctrl+R/Up older · Down newer · Esc cancel".to_owned()
        } else {
            "Enter insert · Esc cancel".to_owned()
        },
        secondary,
    )));

    let height = lines.len() as u16 + 2;
    let area = Rect::new(screen.x, app.input_area.y - height, width, height);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" History search ")
        .border_style(Style::default().fg(app.theme.accent));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}
