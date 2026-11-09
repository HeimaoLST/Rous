use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// True when the path's extension names a supported image type. Only the path
/// shape is decided here; the actual mime comes from the bytes.
pub fn looks_like_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

/// Detects the image mime from the file bytes with the `image` crate's sniffing,
/// so a PNG named `.jpg` is sent as `image/png` rather than trusting the
/// extension. Actual decodability is checked separately by `attach_image`.
pub fn image_mime_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::Gif => Some("image/gif"),
        image::ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

/// One image attachment kept as an immutable data-URL snapshot plus a display
/// name. The raw bytes are never re-read, so later file changes cannot alter a
/// draft or a recalled history entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageAttachment {
    pub name: String,
    /// `data:<mime>;base64,<payload>`.
    pub data_url: String,
}

impl ImageAttachment {
    pub fn new(name: impl Into<String>, data_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data_url: data_url.into(),
        }
    }

    /// Builds a data URL from already-encoded image bytes.
    pub fn from_bytes(name: impl Into<String>, mime: &str, bytes: &[u8]) -> Self {
        let payload = base64::engine::general_purpose::STANDARD.encode(bytes);
        Self::new(name, format!("data:{mime};base64,{payload}"))
    }
}

/// One caller-provided context document carried by a user turn. It is user
/// data, never a system instruction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextInput {
    pub name: String,
    pub text: String,
}

/// A user turn: text, caller-provided context, and zero or more images.
///
/// `String` and `&str` convert into a text-only message so existing callers and
/// the console front end stay unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UserInput {
    pub text: String,
    pub images: Vec<ImageAttachment>,
    pub context: Vec<ContextInput>,
}

impl UserInput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
            context: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.images.is_empty() && self.context.is_empty()
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

impl From<&UserInput> for UserInput {
    fn from(message: &UserInput) -> Self {
        message.clone()
    }
}

/// Classifies a paste as a single image path. Ordinary chat text such as
/// `please inspect /tmp/foo.png` is rejected because the whole string is not an
/// explicit path and does not correspond to an existing file. `exists` is
/// injected so the classification stays pure and testable.
pub fn classify_image_paste(text: &str, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    let candidate = unescape_path(strip_matching_quotes(trimmed));
    let path = PathBuf::from(&candidate);
    if !looks_like_image(&path) {
        return None;
    }
    if has_explicit_prefix(&candidate) || exists(path.as_path()) {
        return Some(path);
    }
    None
}

fn has_explicit_prefix(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with("./")
        || text.starts_with("../")
        || text.starts_with("~/")
}

fn strip_matching_quotes(text: &str) -> &str {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if first == last && (first == b'"' || first == b'\'') {
            return &text[1..text.len() - 1];
        }
    }
    text
}

/// Applies the common drag-and-drop shell escapes without mangling a Windows
/// drive path: only `\ `, `\\` and `\` before punctuation drop the backslash.
fn unescape_path(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some(' ') | Some('\\') | Some('(') | Some(')') | Some('[') | Some(']')
                | Some('{') | Some('}') | Some(',') | Some(';') | Some('\'') | Some('"') => {
                    out.push(chars.next().unwrap());
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
#[path = "../tests/input/mod.rs"]
mod tests;
