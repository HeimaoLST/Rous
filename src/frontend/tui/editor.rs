use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::input::{ImageAttachment, UserInput};

/// Codex-style word separator run; word motion treats these as one piece.
const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// Combined retained-history caps: entry count and a hard text-byte budget
/// shared by undo and redo. An edit whose pre-state alone exceeds the budget
/// resets and skips history; undo/redo refuses (draft unchanged) when the live
/// draft cannot fit.
const MAX_UNDO_ENTRIES: usize = 100;
const MAX_UNDO_BYTES: usize = 8 * 1024 * 1024;

/// One draft state kept for undo/redo. Text lines are owned; attachments are
/// shared by pointer so consecutive text edits never clone base64 payloads. The
/// preferred display column is captured so undo/redo restores vertical-movement
/// state, not just the logical cursor.
struct Snapshot {
    lines: Vec<String>,
    row: usize,
    col: usize,
    preferred_col: Option<usize>,
    images: Arc<Vec<ImageAttachment>>,
}

impl Snapshot {
    /// Serialized text bytes, including the newline between lines.
    fn text_bytes(&self) -> usize {
        self.lines.iter().map(String::len).sum::<usize>() + self.lines.len().saturating_sub(1)
    }
}

/// Bounded undo/redo stacks. `typing` is true while consecutive typed
/// characters extend the most recent undo entry. `byte_cap`/`entry_cap` bound
/// the *combined* retained history across both stacks.
struct EditHistory {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    typing: bool,
    byte_cap: usize,
    entry_cap: usize,
}

impl Default for EditHistory {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            byte_cap: MAX_UNDO_BYTES,
            entry_cap: MAX_UNDO_ENTRIES,
        }
    }
}

impl EditHistory {
    /// Records the pre-edit state. An oversized pre-edit snapshot resets and
    /// skips history (documented oversized-edit behavior); otherwise redo is
    /// discarded and the combined stacks are re-trimmed.
    fn record(&mut self, snapshot: Snapshot) {
        if snapshot.text_bytes() > self.byte_cap {
            self.clear();
            return;
        }
        self.redo.clear();
        self.undo.push(snapshot);
        self.trim();
    }

    /// Drops entries until the combined stacks fit both caps; older undo entries
    /// are dropped before older redo entries. Every retained snapshot is
    /// individually <= `byte_cap`, so this is a real bound.
    fn trim(&mut self) {
        while self.len() > self.entry_cap {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
        while self.total_bytes() > self.byte_cap {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
    }

    fn len(&self) -> usize {
        self.undo.len() + self.redo.len()
    }

    fn total_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .map(Snapshot::text_bytes)
            .sum()
    }

    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.typing = false;
    }
}

/// Multi-line draft editor.
///
/// Each logical line keeps its original text; grapheme boundaries are derived
/// from the final line after every edit. This keeps separately typed combining
/// marks and ZWJ sequences as one unit for movement and deletion. The editor
/// also owns one kill buffer and the preferred display column; the TUI owns
/// input history.
pub struct Editor {
    lines: Vec<String>,
    /// Grapheme index within the cursor row.
    col: usize,
    row: usize,
    /// Single-entry kill buffer shared by word/line kills and Ctrl+Y.
    kill: Option<String>,
    /// Preferred display column for consecutive vertical moves.
    preferred_col: Option<usize>,
    /// Draft image attachments, shared by pointer with undo snapshots so text
    /// edits never clone base64 payloads; collection changes build a new Arc.
    images: Arc<Vec<ImageAttachment>>,
    /// Bounded local draft history owned by the editor.
    history: EditHistory,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// One wrapped input line. `cells` holds the cell offset of every grapheme
/// boundary, so display and navigation share one wrap calculation.
pub struct DisplayLine {
    pub text: String,
    pub row: usize,
    pub grapheme_start: usize,
    pub grapheme_end: usize,
    pub cells: Vec<usize>,
    /// True when this display line ends its logical line, so the end-of-line
    /// position is selectable; a soft-wrap fragment must not claim the next
    /// fragment's start.
    pub is_logical_end: bool,
}

impl DisplayLine {
    /// Maps a target display cell to a (logical row, grapheme index). A soft
    /// wrap fragment clamps into its last grapheme instead of returning the
    /// boundary owned by the next visual line; only the logical end may return
    /// EOL.
    pub fn position_for_cell(&self, cell: usize) -> (usize, usize) {
        let count = self.grapheme_end - self.grapheme_start;
        let mut index = 0usize;
        while index < count && self.cells[index + 1] <= cell {
            index += 1;
        }
        if index == count && !self.is_logical_end && count > 0 {
            index = count - 1;
        }
        (self.row, self.grapheme_start + index)
    }
}

/// Wrapped input layout used by both rendering and vertical navigation.
pub struct EditorLayout {
    pub lines: Vec<DisplayLine>,
    pub cursor_line: usize,
    pub cursor_cell: usize,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            col: 0,
            row: 0,
            kill: None,
            preferred_col: None,
            images: Arc::new(Vec::new()),
            history: EditHistory::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty() && self.images.is_empty()
    }

    pub fn images(&self) -> &[ImageAttachment] {
        self.images.as_slice()
    }

    /// Appends a draft attachment as one undoable collection change.
    pub fn insert_image(&mut self, image: ImageAttachment) {
        self.checkpoint(false);
        let mut images = (*self.images).clone();
        images.push(image);
        self.images = Arc::new(images);
        self.preferred_col = None;
    }

    /// Removes the last draft attachment as one undoable collection change;
    /// returns `false` when there was none.
    pub fn remove_last_image(&mut self) -> bool {
        if self.images.is_empty() {
            return false;
        }
        self.checkpoint(false);
        let mut images = (*self.images).clone();
        images.pop();
        self.images = Arc::new(images);
        self.preferred_col = None;
        true
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Clears the draft as one undoable edit; keeps the kill buffer and
    /// resets vertical state. A no-op clear leaves redo intact.
    pub fn clear(&mut self) {
        if self.is_empty() {
            return;
        }
        self.checkpoint(false);
        self.clear_state();
    }

    /// Clears the draft and drops all undo/redo history (executed command).
    pub fn reset(&mut self) {
        self.clear_state();
        self.history.clear();
    }

    /// Drops undo/redo history but keeps the draft (send, session switch).
    pub fn reset_history(&mut self) {
        self.history.clear();
    }

    /// Clears the draft for the Ctrl+C path: the clear stays undoable while the
    /// returned draft is stored in the input history.
    pub fn clear_for_history(&mut self) -> UserInput {
        let text = self.text();
        if self.is_empty() {
            return UserInput {
                context: Vec::new(),
                text,
                images: Vec::new(),
            };
        }
        self.checkpoint(false);
        let images = self.take_images();
        self.clear_state();
        UserInput {
            context: Vec::new(),
            text,
            images,
        }
    }

    /// Copy of the current draft for a pending send. The editor keeps its
    /// contents until the Runtime acceptance reply.
    pub fn draft_message(&self) -> UserInput {
        UserInput {
            context: Vec::new(),
            text: self.text(),
            images: self.images().to_vec(),
        }
    }

    pub fn matches_message(&self, message: &UserInput) -> bool {
        self.text() == message.text && self.images() == message.images.as_slice()
    }

    /// Removes and returns the draft, e.g. when submitting a message. The
    /// image snapshots are moved, not cloned, and undo history is reset so an
    /// executed task can never be undone.
    pub fn take(&mut self) -> UserInput {
        let text = self.text();
        self.reset_history();
        let images = self.take_images();
        self.clear_state();
        UserInput {
            context: Vec::new(),
            text,
            images,
        }
    }

    /// Replaces the whole draft (history recall) with the cursor at the end.
    /// The replacement is one undoable edit; the kill buffer survives.
    pub fn set_draft(&mut self, message: impl Into<UserInput>) {
        let message = message.into();
        if self.text() == message.text && self.images.as_slice() == message.images.as_slice() {
            self.end_typing();
        } else {
            self.checkpoint(false);
        }
        self.lines = if message.text.is_empty() {
            vec![String::new()]
        } else {
            message.text.split('\n').map(str::to_owned).collect()
        };
        self.images = Arc::new(message.images);
        self.row = self.lines.len() - 1;
        self.col = grapheme_count(&self.lines[self.row]);
        self.preferred_col = None;
    }

    fn clear_state(&mut self) {
        self.lines = vec![String::new()];
        self.row = 0;
        self.col = 0;
        self.preferred_col = None;
        self.images = Arc::new(Vec::new());
    }

    fn take_images(&mut self) -> Vec<ImageAttachment> {
        let images = std::mem::replace(&mut self.images, Arc::new(Vec::new()));
        Arc::try_unwrap(images).unwrap_or_else(|shared| (*shared).clone())
    }

    /// Records the pre-edit state. `coalesce` extends the current typing group
    /// instead of starting a new undo entry. Callers invoke this only when the
    /// edit actually changes state, so no-op edits keep redo intact.
    fn checkpoint(&mut self, coalesce: bool) {
        if coalesce && self.history.typing {
            return;
        }
        let snapshot = self.snapshot();
        self.history.record(snapshot);
        self.history.typing = coalesce;
    }

    /// Ends the current typing group without recording an edit (navigation).
    fn end_typing(&mut self) {
        self.history.typing = false;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            row: self.row,
            col: self.col,
            preferred_col: self.preferred_col,
            images: Arc::clone(&self.images),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.lines = snapshot.lines;
        self.row = snapshot.row;
        self.col = snapshot.col;
        self.preferred_col = snapshot.preferred_col;
        self.images = snapshot.images;
    }

    /// Serialized byte size of the live draft, computed before any clone.
    fn live_text_bytes(&self) -> usize {
        self.lines.iter().map(String::len).sum::<usize>() + self.lines.len().saturating_sub(1)
    }

    /// Restores the previous draft state. Returns `false` when nothing is left
    /// to undo, or when the live draft alone cannot fit the byte budget (the
    /// draft is then left unchanged rather than losing recoverable content).
    /// Undo never records a new checkpoint.
    pub fn undo(&mut self) -> bool {
        if self.history.undo.is_empty() || self.live_text_bytes() > self.history.byte_cap {
            return false;
        }
        let current = self.snapshot();
        let previous = self.history.undo.pop().expect("checked non-empty");
        self.history.redo.push(current);
        self.history.trim();
        self.restore(previous);
        self.history.typing = false;
        true
    }

    /// Re-applies an undone draft state. Returns `false` when nothing is left
    /// to redo, or when the live draft alone cannot fit the byte budget (the
    /// draft is then left unchanged). Redo never records a new checkpoint.
    pub fn redo(&mut self) -> bool {
        if self.history.redo.is_empty() || self.live_text_bytes() > self.history.byte_cap {
            return false;
        }
        let current = self.snapshot();
        let next = self.history.redo.pop().expect("checked non-empty");
        self.history.undo.push(current);
        self.history.trim();
        self.restore(next);
        self.history.typing = false;
        true
    }

    /// Test-only: proves text checkpoints share the attachment payload Arc.
    #[cfg(test)]
    pub fn images_shared_count(&self) -> usize {
        Arc::strong_count(&self.images)
    }

    /// Test-only: combined retained text across undo and redo.
    #[cfg(test)]
    pub fn retained_bytes(&self) -> usize {
        self.history.total_bytes()
    }

    /// Test-only seam: exercise the retention policy at small limits.
    #[cfg(test)]
    pub fn set_history_limits(&mut self, byte_cap: usize, entry_cap: usize) {
        self.history.clear();
        self.history.byte_cap = byte_cap;
        self.history.entry_cap = entry_cap;
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// True when the cursor is at the start or the end of the whole draft.
    pub fn at_document_boundary(&self) -> bool {
        (self.row == 0 && self.col == 0)
            || (self.row + 1 == self.lines.len()
                && self.col == grapheme_count(&self.lines[self.row]))
    }

    /// Byte offset of the cursor in the full draft text.
    pub fn global_offset(&self) -> usize {
        self.global_offset_of(self.row, self.col)
    }

    fn global_offset_of(&self, row: usize, col: usize) -> usize {
        let mut offset = 0;
        for index in 0..row {
            offset += self.lines[index].len() + 1;
        }
        offset + byte_offset(&self.lines[row], col)
    }

    pub fn set_global_offset(&mut self, mut offset: usize) {
        self.end_typing();
        let mut row = 0;
        loop {
            let line_len = self.lines[row].len();
            if offset <= line_len || row + 1 >= self.lines.len() {
                break;
            }
            offset -= line_len + 1;
            row += 1;
        }
        self.row = row;
        self.col = grapheme_count(&self.lines[row][..offset.min(self.lines[row].len())]);
    }

    fn set_text_with_cursor(&mut self, text: &str, cursor: usize) {
        self.lines = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(str::to_owned).collect()
        };
        self.row = 0;
        self.col = 0;
        self.set_global_offset(cursor.min(text.len()));
        self.preferred_col = None;
    }

    fn replace_global_range(&mut self, start: usize, end: usize, insert: &str) {
        let text = self.text();
        let mut replaced = String::with_capacity(text.len() + insert.len());
        replaced.push_str(&text[..start]);
        replaced.push_str(insert);
        replaced.push_str(&text[end..]);
        let cursor = start + insert.len();
        self.set_text_with_cursor(&replaced, cursor);
    }

    fn kill_range(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        self.kill = Some(self.text()[start..end].to_owned());
        self.replace_global_range(start, end, "");
    }

    /// Replaces one byte range as a single undo step without touching images or
    /// the kill buffer. Returns whether the text changed; the cursor always
    /// lands after the inserted text (a no-op still moves it there).
    pub fn complete_global_range(&mut self, start: usize, end: usize, insert: &str) -> bool {
        let before = self.text();
        let changed = before[start..end] != *insert;
        if changed {
            self.checkpoint(false);
            self.replace_global_range(start, end, insert);
        } else {
            self.set_global_offset(start + insert.len());
        }
        changed
    }

    /// Replaces the whole draft after an external editor run. A changed text is
    /// one undo step with the cursor at the end; an exact no-op does not
    /// checkpoint or move the cursor. Images and the kill buffer are untouched.
    pub fn replace_text(&mut self, text: &str) -> bool {
        if self.text() == text {
            return false;
        }
        self.checkpoint(false);
        self.set_text_with_cursor(text, text.len());
        self.preferred_col = None;
        true
    }

    /// Inserts pasted text as one atomic undo step. Newlines never submit.
    pub fn insert_str(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }
        self.checkpoint(false);
        let mut parts = normalized.split('\n');
        self.insert_at_cursor(parts.next().unwrap_or(""));
        for part in parts {
            self.insert_newline_raw();
            self.insert_at_cursor(part);
        }
        self.preferred_col = None;
    }

    /// Inserts one typed character, coalescing consecutive typing into one
    /// undo step. Bracketed/text paste uses `insert_str` and stays atomic.
    pub fn insert_typed(&mut self, character: char) {
        self.checkpoint(true);
        self.insert_at_cursor(&character.to_string());
        self.preferred_col = None;
    }

    fn insert_at_cursor(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let line = &mut self.lines[self.row];
        let offset = byte_offset(line, self.col);
        line.insert_str(offset, text);
        // Re-segment the final logical line: a combining mark typed separately
        // must not advance the cursor past a grapheme that now merges.
        self.col = grapheme_count(&line[..offset + text.len()]);
    }

    pub fn insert_newline(&mut self) {
        self.checkpoint(false);
        self.insert_newline_raw();
    }

    fn insert_newline_raw(&mut self) {
        let line = self.lines[self.row].clone();
        let offset = byte_offset(&line, self.col);
        let tail = line[offset..].to_owned();
        self.lines[self.row].truncate(offset);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
        self.preferred_col = None;
    }

    pub fn backspace(&mut self) {
        if self.lines.len() == 1 && self.lines[0].is_empty() && !self.images.is_empty() {
            self.remove_last_image();
            return;
        }
        if self.col == 0 && self.row == 0 {
            return;
        }
        self.checkpoint(false);
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let end = byte_offset(line, self.col);
            let start = byte_offset(line, self.col - 1);
            line.replace_range(start..end, "");
            self.col -= 1;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = grapheme_count(&self.lines[self.row]);
            self.lines[self.row].push_str(&line);
        }
        self.preferred_col = None;
    }

    pub fn delete(&mut self) {
        if self.col >= grapheme_count(&self.lines[self.row]) && self.row + 1 >= self.lines.len() {
            return;
        }
        self.checkpoint(false);
        if self.col < grapheme_count(&self.lines[self.row]) {
            let line = &mut self.lines[self.row];
            let start = byte_offset(line, self.col);
            let end = byte_offset(line, self.col + 1);
            line.replace_range(start..end, "");
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
        self.preferred_col = None;
    }

    pub fn move_left(&mut self) {
        self.end_typing();
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = grapheme_count(&self.lines[self.row]);
        }
        self.preferred_col = None;
    }

    pub fn move_right(&mut self) {
        self.end_typing();
        if self.col < grapheme_count(&self.lines[self.row]) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
        self.preferred_col = None;
    }

    /// Ctrl+A: line start; already there moves to the previous line start.
    pub fn move_line_start(&mut self) {
        self.end_typing();
        if self.col > 0 {
            self.col = 0;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = 0;
        }
        self.preferred_col = None;
    }

    /// Ctrl+E: line end; already there moves to the next line end.
    pub fn move_line_end(&mut self) {
        self.end_typing();
        let end = grapheme_count(&self.lines[self.row]);
        if self.col < end {
            self.col = end;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = grapheme_count(&self.lines[self.row]);
        }
        self.preferred_col = None;
    }

    /// Home: start of the current logical line only.
    pub fn home(&mut self) {
        self.end_typing();
        self.col = 0;
        self.preferred_col = None;
    }

    /// End: end of the current logical line only.
    pub fn end(&mut self) {
        self.end_typing();
        self.col = grapheme_count(&self.lines[self.row]);
        self.preferred_col = None;
    }

    pub fn move_word_left(&mut self) {
        let text = self.text();
        let cursor = self.global_offset();
        let target = snap_grapheme_start(&text, beginning_of_previous_word(&text, cursor));
        self.set_global_offset(target);
        self.preferred_col = None;
    }

    pub fn move_word_right(&mut self) {
        let text = self.text();
        let cursor = self.global_offset();
        let target = snap_grapheme_end(&text, end_of_next_word(&text, cursor));
        self.set_global_offset(target);
        self.preferred_col = None;
    }

    pub fn delete_word_backward(&mut self) {
        let text = self.text();
        let cursor = self.global_offset();
        let start = snap_grapheme_start(&text, beginning_of_previous_word(&text, cursor));
        if start >= cursor {
            return;
        }
        self.checkpoint(false);
        self.kill_range(start, cursor);
    }

    pub fn delete_word_forward(&mut self) {
        let text = self.text();
        let cursor = self.global_offset();
        let end = snap_grapheme_end(&text, end_of_next_word(&text, cursor));
        if end <= cursor {
            return;
        }
        self.checkpoint(false);
        self.kill_range(cursor, end);
    }

    /// Ctrl+U: kill to the logical line start; at the boundary kill the
    /// preceding newline instead.
    pub fn kill_to_line_start(&mut self) {
        let cursor = self.global_offset();
        if self.col > 0 {
            let start = self.global_offset_of(self.row, 0);
            if start < cursor {
                self.checkpoint(false);
                self.kill_range(start, cursor);
            }
        } else if self.row > 0 {
            self.checkpoint(false);
            self.kill_range(cursor - 1, cursor);
        }
    }

    /// Ctrl+K: kill to the logical line end; at the boundary kill the trailing
    /// newline instead.
    pub fn kill_to_line_end(&mut self) {
        let cursor = self.global_offset();
        let end = grapheme_count(&self.lines[self.row]);
        if self.col < end {
            let target = self.global_offset_of(self.row, end);
            if cursor < target {
                self.checkpoint(false);
                self.kill_range(cursor, target);
            }
        } else if self.row + 1 < self.lines.len() {
            self.checkpoint(false);
            self.kill_range(cursor, cursor + 1);
        }
    }

    /// Ctrl+Y: insert the single kill buffer at the cursor.
    pub fn yank(&mut self) {
        if let Some(text) = self.kill.clone() {
            self.insert_str(&text);
        }
    }

    /// Moves the cursor one wrapped display line up, restoring the preferred
    /// display column when a longer line follows a shorter one. On the first
    /// visual line it goes to the document start; the preferred column is
    /// clamped to the current width and reset at that boundary.
    pub fn move_visual_up(&mut self, width: usize) {
        self.end_typing();
        let layout = self.layout(width);
        if layout.cursor_line == 0 {
            self.row = 0;
            self.col = 0;
            self.preferred_col = None;
            return;
        }
        let desired = self
            .preferred_col
            .unwrap_or(layout.cursor_cell)
            .min(width.saturating_sub(1));
        let target = &layout.lines[layout.cursor_line - 1];
        let (row, col) = target.position_for_cell(desired);
        self.row = row;
        self.col = col;
        self.preferred_col = Some(desired);
    }

    pub fn move_visual_down(&mut self, width: usize) {
        self.end_typing();
        let layout = self.layout(width);
        if layout.cursor_line + 1 >= layout.lines.len() {
            let last = self.lines.len() - 1;
            self.row = last;
            self.col = grapheme_count(&self.lines[last]);
            self.preferred_col = None;
            return;
        }
        let desired = self
            .preferred_col
            .unwrap_or(layout.cursor_cell)
            .min(width.saturating_sub(1));
        let target = &layout.lines[layout.cursor_line + 1];
        let (row, col) = target.position_for_cell(desired);
        self.row = row;
        self.col = col;
        self.preferred_col = Some(desired);
    }

    /// Wraps the draft for both rendering and navigation. A last logical line
    /// that exactly fills the width gets a stable virtual line for the
    /// end-of-text cursor, so the layout never jumps as the cursor moves.
    pub fn layout(&self, width: usize) -> EditorLayout {
        let width = width.max(1);
        let mut lines: Vec<DisplayLine> = Vec::new();
        for row in 0..self.lines.len() {
            let text = &self.lines[row];
            let graphemes: Vec<&str> = text.graphemes(true).collect();
            if graphemes.is_empty() {
                lines.push(DisplayLine {
                    text: String::new(),
                    row,
                    grapheme_start: 0,
                    grapheme_end: 0,
                    cells: vec![0],
                    is_logical_end: true,
                });
                continue;
            }
            let mut start = 0usize;
            let mut used = 0usize;
            let mut current = String::new();
            let mut cells = vec![0usize];
            for (index, grapheme) in graphemes.iter().enumerate() {
                let cell_width = UnicodeWidthStr::width(*grapheme).max(1);
                if used + cell_width > width && !current.is_empty() {
                    lines.push(DisplayLine {
                        text: std::mem::take(&mut current),
                        row,
                        grapheme_start: start,
                        grapheme_end: index,
                        cells: std::mem::replace(&mut cells, vec![0usize]),
                        is_logical_end: false,
                    });
                    start = index;
                    used = 0;
                }
                current.push_str(grapheme);
                used += cell_width;
                cells.push(used);
            }
            lines.push(DisplayLine {
                text: current,
                row,
                grapheme_start: start,
                grapheme_end: graphemes.len(),
                cells,
                is_logical_end: true,
            });
        }
        if let Some(last) = lines.last() {
            let full = last.grapheme_end > last.grapheme_start
                && last.cells.last().copied().unwrap_or(0) >= width;
            if full {
                let row = last.row;
                let end = last.grapheme_end;
                lines.push(DisplayLine {
                    text: String::new(),
                    row,
                    grapheme_start: end,
                    grapheme_end: end,
                    cells: vec![0],
                    is_logical_end: true,
                });
            }
        }

        let (cursor_row, cursor_col) = self.cursor();
        let mut cursor_line = 0usize;
        let mut cursor_cell = 0usize;
        for (index, line) in lines.iter().enumerate() {
            if line.row == cursor_row
                && cursor_col >= line.grapheme_start
                && cursor_col <= line.grapheme_end
            {
                cursor_line = index;
                let local = (cursor_col - line.grapheme_start).min(line.cells.len() - 1);
                cursor_cell = line.cells[local];
            }
        }
        EditorLayout {
            lines,
            cursor_line,
            cursor_cell,
        }
    }
}

fn is_word_separator(ch: char) -> bool {
    WORD_SEPARATORS.contains(ch)
}

/// Splits a non-whitespace run into word/separator pieces at Unicode word
/// boundaries, further separating mixed punctuation where Codex does.
fn split_word_pieces(run: &str) -> Vec<(usize, &str)> {
    let mut pieces = Vec::new();
    for (segment_start, segment) in run.split_word_bound_indices() {
        let mut piece_start = 0;
        let mut chars = segment.char_indices();
        let Some((_, first_char)) = chars.next() else {
            continue;
        };
        let mut in_separator = is_word_separator(first_char);
        for (index, ch) in chars {
            let is_sep = is_word_separator(ch);
            if is_sep == in_separator {
                continue;
            }
            pieces.push((segment_start + piece_start, &segment[piece_start..index]));
            piece_start = index;
            in_separator = is_sep;
        }
        pieces.push((segment_start + piece_start, &segment[piece_start..]));
    }
    pieces
}

fn beginning_of_previous_word(text: &str, cursor: usize) -> usize {
    let prefix = &text[..cursor.min(text.len())];
    let Some((first_non_ws_idx, ch)) = prefix
        .char_indices()
        .rev()
        .find(|&(_, ch)| !ch.is_whitespace())
    else {
        return 0;
    };
    let run_start = prefix[..first_non_ws_idx]
        .char_indices()
        .rev()
        .find(|&(_, ch)| ch.is_whitespace())
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let run_end = first_non_ws_idx + ch.len_utf8();
    let pieces = split_word_pieces(&prefix[run_start..run_end]);
    let mut pieces = pieces.into_iter().rev().peekable();
    let Some((piece_start, piece)) = pieces.next() else {
        return run_start;
    };
    let mut start = run_start + piece_start;
    if piece.chars().all(is_word_separator) {
        while let Some((index, piece)) = pieces.peek() {
            if !piece.chars().all(is_word_separator) {
                break;
            }
            start = run_start + *index;
            pieces.next();
        }
    }
    start
}

fn end_of_next_word(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    let suffix = &text[cursor..];
    let Some(first_non_ws) = suffix.find(|ch: char| !ch.is_whitespace()) else {
        return text.len();
    };
    let run = &suffix[first_non_ws..];
    let run = &run[..run.find(char::is_whitespace).unwrap_or(run.len())];
    let mut pieces = split_word_pieces(run).into_iter().peekable();
    let Some((start, piece)) = pieces.next() else {
        return cursor + first_non_ws;
    };
    let word_start = cursor + first_non_ws + start;
    let mut end = word_start + piece.len();
    if piece.chars().all(is_word_separator) {
        while let Some((index, piece)) = pieces.peek() {
            if !piece.chars().all(is_word_separator) {
                break;
            }
            end = cursor + first_non_ws + *index + piece.len();
            pieces.next();
        }
    }
    end
}

fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Largest grapheme boundary at or before `offset`.
fn snap_grapheme_start(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let mut boundary = 0;
    for (index, _) in text.grapheme_indices(true) {
        if index > offset {
            break;
        }
        boundary = index;
    }
    boundary
}

/// Smallest grapheme boundary at or after `offset`.
fn snap_grapheme_end(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    for (index, grapheme) in text.grapheme_indices(true) {
        let end = index + grapheme.len();
        if offset <= index {
            return index;
        }
        if offset < end {
            return end;
        }
    }
    text.len()
}

fn byte_offset(text: &str, grapheme_index: usize) -> usize {
    text.grapheme_indices(true)
        .nth(grapheme_index)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}
