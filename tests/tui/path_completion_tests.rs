use super::*;
use crate::frontend::tui::path_completion;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("saki28-{}-{label}-{unique}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_at(dir: &Path) -> App {
    let mut app = App::new();
    app.working_directory = Some(dir.to_path_buf());
    app.last_width = 80;
    app.last_height = 20;
    app
}

fn editor_with(text: &str) -> Editor {
    let mut editor = Editor::new();
    editor.insert_str(text);
    editor
}

fn cursor_after(text: &str, needle: &str) -> usize {
    text.find(needle).unwrap() + needle.len()
}

#[test]
fn single_directory_completes_with_separator() {
    let dir = workspace("single");
    fs::create_dir(dir.join("projects")).unwrap();
    let mut app = app_at(&dir);
    let mut editor = editor_with("cd pro");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(outcome.send.is_none());
    assert_eq!(editor.text(), "cd projects/");
    assert_eq!(editor.global_offset(), "cd projects/".len());
    assert!(!path_completion::active(&app));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn multiple_candidates_sort_directories_first_and_confirm_without_sending() {
    let dir = workspace("multi");
    fs::create_dir(dir.join("alpha-dir")).unwrap();
    fs::write(dir.join("alpha-file.txt"), "x").unwrap();
    let mut app = app_at(&dir);
    let mut editor = editor_with("open a");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(outcome.send.is_none());
    assert!(path_completion::active(&app));
    // Directories sort first, so the first confirm inserts the directory.
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "open alpha-dir/");
    assert!(!path_completion::active(&app));

    let mut app = app_at(&dir);
    let mut editor = editor_with("open a");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    handle_key(&mut app, &mut editor, key(KeyCode::Down));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "open alpha-file.txt");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn case_insensitive_sort_and_dotfile_rules() {
    let dir = workspace("sort");
    fs::write(dir.join("Bravo.txt"), "x").unwrap();
    fs::write(dir.join("alpha.txt"), "x").unwrap();
    fs::write(dir.join(".hidden"), "x").unwrap();
    // A trailing separator lists the directory; lowercase ordering puts
    // alpha.txt before Bravo.txt even though ASCII would order them opposite.
    let mut app = app_at(&dir);
    let mut editor = editor_with("open ./");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(path_completion::active(&app));
    handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(editor.text(), "open ./alpha.txt");

    // Without a leading dot, dotfiles are excluded (single visible match here).
    let mut app = app_at(&dir);
    let mut editor = editor_with("open B");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "open Bravo.txt");

    // A leading dot searches dotfiles.
    let mut app = app_at(&dir);
    let mut editor = editor_with("open .");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "open .hidden");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn quoted_spaces_unicode_and_escaped_unquoted_names_stay_valid() {
    let dir = workspace("quotes");
    fs::create_dir(dir.join("my project")).unwrap();
    fs::create_dir(dir.join("项目")).unwrap();
    fs::create_dir(dir.join("my dir")).unwrap();

    let text = "read \"./my pro\" tail";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "\"./my pro") + 1);
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "read \"./my project/\" tail");

    let text = "open \"项\"";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(text.len() - 1);
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "open \"项目/\"");

    let mut app = app_at(&dir);
    let mut editor = editor_with("cd my\\ d");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "cd my\\ dir/");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn mid_component_cursor_preserves_suffix_and_does_not_duplicate() {
    let dir = workspace("mid");
    fs::create_dir(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "x").unwrap();

    let text = "read ./src/main.rs tail";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "./sr"));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "read ./src/main.rs tail");
    assert_eq!(editor.global_offset(), cursor_after(text, "./src"));

    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "./src/ma"));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "read ./src/main.rs tail");
    assert_eq!(editor.global_offset(), "read ./src/main.rs".len());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn absolute_prefix_completes_against_the_filesystem() {
    let dir = workspace("absolute");
    fs::create_dir(dir.join("absolute-target")).unwrap();
    let prefix = format!("open {}/abso", dir.display());
    let mut app = app_at(&dir);
    let mut editor = editor_with(&prefix);
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(
        editor.text(),
        format!("open {}/absolute-target/", dir.display())
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn escape_dismisses_unchanged_and_busy_esc_wins_before_cancel() {
    let dir = workspace("esc");
    fs::create_dir(dir.join("alpha-dir")).unwrap();
    fs::write(dir.join("alpha-file.txt"), "x").unwrap();
    let mut app = app_at(&dir);
    app.running = true;
    let mut editor = editor_with("open a");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(path_completion::active(&app));
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(!outcome.cancel);
    assert!(!path_completion::active(&app));
    assert_eq!(editor.text(), "open a");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Esc));
    assert!(outcome.cancel, "next Esc cancels the running task");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn no_candidates_and_empty_composer_keep_the_original_tab_focus() {
    let dir = workspace("fallback");
    let mut app = app_at(&dir);
    let mut editor = editor_with("zzz");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(outcome.send.is_none());
    assert!(!path_completion::active(&app));
    assert_eq!(app.focus, Focus::Transcript);
    assert_eq!(editor.text(), "zzz");

    let mut app = app_at(&dir);
    let mut editor = Editor::new();
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Transcript);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn completion_is_one_undo_and_keeps_images_and_noop_redo() {
    let dir = workspace("undo");
    fs::create_dir(dir.join("undo-dir")).unwrap();
    fs::write(dir.join("exact.txt"), "x").unwrap();
    let mut app = app_at(&dir);
    let mut editor = editor_with("cd un");
    editor.insert_image(crate::input::ImageAttachment::new(
        "a.png",
        "data:image/png;base64,AA",
    ));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "cd undo-dir/");
    assert_eq!(editor.images().len(), 1);
    assert!(editor.undo());
    assert_eq!(editor.text(), "cd un");
    assert_eq!(editor.images().len(), 1);
    assert!(editor.redo());
    assert_eq!(editor.text(), "cd undo-dir/");

    let mut app = app_at(&dir);
    let mut editor = Editor::new();
    editor.insert_str("cat exact.txt");
    editor.insert_str(" tail");
    assert!(editor.undo());
    assert_eq!(editor.text(), "cat exact.txt");
    editor.set_global_offset(cursor_after("cat exact.txt", "exa"));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "cat exact.txt");
    assert!(
        editor.redo(),
        "a no-op completion must not discard redo history"
    );
    assert_eq!(editor.text(), "cat exact.txt tail");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn commands_popup_has_priority_then_path_completion_runs() {
    use crate::frontend::tui::commands::Commands;

    let dir = workspace("commands");
    fs::create_dir(dir.join("projects")).unwrap();
    let mut app = app_at(&dir);
    let mut editor = editor_with("/");
    let mut commands = Commands::default();
    commands.sync(&app, &editor);
    assert!(commands.key(&mut editor, key(KeyCode::Tab)));
    assert!(
        editor.text().starts_with('/') && editor.text().ends_with(' '),
        "slash command completion won: {:?}",
        editor.text()
    );
    assert!(!path_completion::active(&app));

    let mut editor = editor_with("cd pro");
    commands.sync(&app, &editor);
    assert!(!commands.key(&mut editor, key(KeyCode::Tab)));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "cd projects/");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn search_help_and_history_take_the_keys_and_close_the_popup() {
    let dir = workspace("overlays");
    fs::create_dir(dir.join("alpha-dir")).unwrap();
    fs::write(dir.join("alpha-file.txt"), "x").unwrap();

    let mut app = app_at(&dir);
    let mut editor = editor_with("open a");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    handle_key(&mut app, &mut editor, key(KeyCode::F(3)));
    assert!(!path_completion::active(&app));
    assert!(crate::frontend::tui::transcript_search::active(&app));
    handle_key(&mut app, &mut editor, key(KeyCode::Esc));

    let mut app = app_at(&dir);
    let mut editor = editor_with("open a");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    handle_key(&mut app, &mut editor, key(KeyCode::F(1)));
    assert!(!path_completion::active(&app));
    assert!(app.help_open);

    let mut app = app_at(&dir);
    let mut editor = editor_with("open a");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    handle_key(
        &mut app,
        &mut editor,
        key_with(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert!(!path_completion::active(&app));
    assert!(app.history.searching());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn truncated_listing_reports_more_and_type_to_narrow() {
    let dir = workspace("truncated");
    for index in 0..60 {
        fs::write(dir.join(format!("file{index:02}")), "x").unwrap();
    }
    let mut app = app_at(&dir);
    let mut editor = editor_with("open file");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(path_completion::active(&app));
    app.input_area = Rect::new(0, 20, 80, 3);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| path_completion::draw(frame, &app))
        .unwrap();
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("Paths"), "{text}");
    assert!(
        text.contains("more") && text.contains("type to narrow"),
        "{text}"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn mid_component_typo_still_matches_on_the_prefix_through_the_cursor() {
    let dir = workspace("typo");
    fs::create_dir(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "x").unwrap();
    let text = "read ./src/maTYPO.rs tail";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "./src/ma"));
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(outcome.send.is_none());
    assert_eq!(app.focus, Focus::Input, "the path token is completed");
    assert_eq!(editor.text(), "read ./src/main.rs tail");
    assert_eq!(editor.global_offset(), "read ./src/main.rs".len());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_following_child_allows_only_directories_and_keeps_the_separator() {
    let dir = workspace("child");
    fs::create_dir_all(dir.join("ab/child")).unwrap();
    fs::write(dir.join("abc"), "x").unwrap();
    let text = "open ab/child";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "open ab"));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(
        !path_completion::active(&app),
        "a file must not join the child path or open a mixed list"
    );
    assert_eq!(editor.text(), "open ab/child");
    assert_eq!(app.focus, Focus::Input);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn quoted_component_before_a_child_preserves_its_closing_quote() {
    let dir = workspace("quotechild");
    fs::create_dir(dir.join("my proj")).unwrap();
    let text = "read \"./my proj\"/file.rs";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "\"./my pro"));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "read \"./my proj\"/file.rs");
    assert_eq!(editor.global_offset(), cursor_after(text, "\"./my proj"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn quoted_unicode_prefix_uses_the_component_quote_for_escaping() {
    let dir = workspace("unicodemode");
    fs::create_dir_all(dir.join("项目/my dir")).unwrap();
    let text = "open \"项目/my";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), "open \"项目/my dir/");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn mixed_quoting_inside_one_component_declines_without_mutation() {
    let dir = workspace("mixedquote");
    fs::create_dir(dir.join("my proj")).unwrap();
    let text = "open my\" pro\"j";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    let before = editor.text();
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(outcome.send.is_none());
    assert_eq!(
        editor.text(),
        before,
        "unsupported mixed quoting is not edited"
    );
    assert!(!path_completion::active(&app));
    assert_eq!(app.focus, Focus::Input);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn opening_quote_after_dot_slash_is_not_consumed() {
    let dir = workspace("openquote");
    fs::create_dir(dir.join("my proj")).unwrap();
    let text = r#"read ./"my proj"/file.rs"#;
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, r#"./"my pro"#));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(editor.text(), text);
    assert_eq!(editor.global_offset(), cursor_after(text, r#"./"my proj"#));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn empty_quoted_component_is_declined_without_mutation() {
    let dir = workspace("emptyquote");
    fs::create_dir(dir.join("my proj")).unwrap();
    for text in [r#"open ./""#, r#"open ./"""#] {
        let mut app = app_at(&dir);
        let mut editor = editor_with(text);
        let before = editor.text();
        handle_key(&mut app, &mut editor, key(KeyCode::Tab));
        assert_eq!(
            editor.text(),
            before,
            "unsupported empty quote keeps the draft"
        );
        assert!(!path_completion::active(&app));
        assert_eq!(app.focus, Focus::Input);
    }
    fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[test]
fn directory_symlinks_complete_as_directories() {
    use std::os::unix::fs::symlink;

    let dir = workspace("symlink");
    fs::create_dir_all(dir.join("real-dir/child")).unwrap();
    symlink(dir.join("real-dir"), dir.join("link-dir")).unwrap();

    let mut app = app_at(&dir);
    let mut editor = editor_with("open link");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(
        editor.text(),
        "open link-dir/",
        "a symlinked directory gets the directory separator"
    );

    let text = "open link-dir/child";
    let mut app = app_at(&dir);
    let mut editor = editor_with(text);
    editor.set_global_offset(cursor_after(text, "open link-dir"));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(
        !path_completion::active(&app),
        "a symlinked directory is not filtered out before a child path"
    );
    assert_eq!(editor.text(), "open link-dir/child");
    assert_eq!(app.focus, Focus::Input);
    fs::remove_dir_all(dir).ok();
}
