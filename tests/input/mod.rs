use super::*;

#[test]
fn classify_rejects_prose_and_accepts_explicit_or_existing() {
    // Ordinary chat text that merely mentions a path is not an attachment.
    assert!(classify_image_paste("please inspect /tmp/foo.png", |_| false).is_none());
    // Explicit prefixes are accepted even when the file does not exist; the
    // loader reports the error and keeps the paste as text.
    assert_eq!(
        classify_image_paste("/tmp/foo.png", |_| false),
        Some(PathBuf::from("/tmp/foo.png"))
    );
    assert_eq!(
        classify_image_paste("./img/pic.jpg", |_| false),
        Some(PathBuf::from("./img/pic.jpg"))
    );
    assert_eq!(
        classify_image_paste("~/pic.webp", |_| false),
        Some(PathBuf::from("~/pic.webp"))
    );
    // A bare name only counts when the file exists.
    assert!(classify_image_paste("photo.png", |_| false).is_none());
    assert_eq!(
        classify_image_paste("photo.png", |_| true),
        Some(PathBuf::from("photo.png"))
    );
    // Non-image extensions and multi-line pastes stay text.
    assert!(classify_image_paste("/tmp/notes.txt", |_| true).is_none());
    assert!(classify_image_paste("/tmp/a.png\n/tmp/b.png", |_| true).is_none());
}

#[test]
fn classify_handles_quotes_and_escapes() {
    assert_eq!(
        classify_image_paste("\"/tmp/my photo.png\"", |_| false),
        Some(PathBuf::from("/tmp/my photo.png"))
    );
    assert_eq!(
        classify_image_paste("/tmp/my\\ photo.png", |_| false),
        Some(PathBuf::from("/tmp/my photo.png"))
    );
}
