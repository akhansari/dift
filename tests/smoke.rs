#![cfg(unix)]

mod terminal;

#[path = "smoke/editor.rs"]
mod editor;
#[path = "smoke/fixtures.rs"]
mod fixtures;
#[path = "smoke/watch.rs"]
mod watch;

use fixtures::Fixture;
use std::{fs, time::Duration};
use terminal::Result;

#[test]
fn startup_from_a_subdirectory_shows_all_categories() -> Result {
    // Given
    let fixture = Fixture::new()?;
    // When
    let mut session = fixture.start()?;
    // Then
    let text = session.screen.text();
    let unstaged = text.find("UNSTAGED").ok_or("Missing unstaged category")?;
    let untracked = text.find("UNTRACKED").ok_or("Missing untracked category")?;
    let staged = text.rfind("STAGED").ok_or("Missing staged category")?;
    assert!(unstaged < untracked && untracked < staged);
    assert!(session.screen.contains("notes.txt"));
    session.quit()
}

#[test]
fn navigation_and_split_layout_use_real_terminal_input() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    session.send("ns")?;
    // Then
    session.select("main.rs")?;
    session.wait_for("both sides of the edit", Duration::from_secs(4), |screen| {
        (0..screen.rows().saturating_sub(3)).any(|y| {
            let row = screen.row(y);
            row.contains("staged") && row.contains("working") && row.contains('│')
        })
    })?;
    session.send("n")?;
    session.select("notes.txt")?;
    session.send("p")?;
    session.select("main.rs")?;
    session.quit()
}

#[test]
fn filtering_updates_tree_and_aggregate_diff() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    session.send("fnotes\r")?;
    // Then
    session.wait_for("filtered tree and diff", Duration::from_secs(4), |screen| {
        screen.badge("All changes", "1/2")
            && screen.contains("untracked note")
            && !screen.contains("main.rs")
    })?;
    session.quit()
}

#[test]
fn search_input_keeps_the_quit_character() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.resize(160, 35)?;
    session.send("/working\r")?;
    session.wait("/working: 1 matches")?;
    // When
    session.send("/q")?;
    // Then
    session.wait("Search › workingq")?;
    session.send("\x1b")?;
    session.wait("/working: 1 matches")?;
    session.quit()
}

#[test]
fn drag_selection_copies_only_source_text() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.write(
        "selection_demo.rs",
        "first_selection_line\nsecond_selection_line\n",
    )?;
    let mut session = fixture.start()?;
    session.send("fselection_demo\r")?;
    session.wait("filter: selection_demo")?;
    session.wait_for(
        "filtered selection document",
        Duration::from_secs(4),
        |screen| {
            screen.contains("second_selection_line")
                && !screen.contains("main.rs")
                && screen.contains("Watching")
        },
    )?;
    let (x1, y1) = session.screen.locate("first_selection_line")?;
    let (x2, y2) = session.screen.locate("second_selection_line")?;
    // When
    session.send(&format!("\x1b[<0;{};{}M", x1 + 1, y1 + 1))?;
    session.send(&format!(
        "\x1b[<32;{};{}M",
        x2 + "second_selection_line".len(),
        y2 + 1
    ))?;
    // Then
    session.wait_for("code-only highlight", Duration::from_secs(4), |screen| {
        screen.highlighted() == ["first_selection_line", "second_selection_line"]
    })?;
    session.send(&format!(
        "\x1b[<0;{};{}m",
        x2 + "second_selection_line".len(),
        y2 + 1
    ))?;
    // Clipboard and rendering are independent writes; allow the same input to finish.
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while session.clipboard.is_empty() && std::time::Instant::now() < deadline {
        session.pump(Duration::from_millis(10))?;
    }
    assert_eq!(
        session.clipboard,
        ["first_selection_line\nsecond_selection_line"]
    );
    assert_eq!(
        session.screen.highlighted(),
        ["first_selection_line", "second_selection_line"]
    );
    session.quit()
}

#[test]
fn whitespace_toggle_round_trip_preserves_configuration() -> Result {
    // Given
    let fixture = Fixture::spacing()?;
    let original = fs::read(fixture.config())?;
    let mut session = fixture.start()?;
    // When
    session.send("w")?;
    // Then
    session.wait_for("hidden spacing file", Duration::from_secs(4), |screen| {
        screen.contains("Whitespace: ignore-changes · 1 hidden by whitespace")
            && !screen.contains("spacing.txt")
    })?;
    session.send("w")?;
    session.wait("Whitespace: exact · 0 hidden by whitespace")?;
    session.wait("one  two")?;
    assert_eq!(fs::read(fixture.config())?, original);
    session.quit()
}

#[test]
fn rapid_whitespace_input_applies_the_final_mode() -> Result {
    // Given
    let fixture = Fixture::spacing()?;
    let mut session = fixture.start()?;
    // When
    session.send("www")?;
    // Then
    session.wait_for(
        "last requested whitespace mode",
        Duration::from_secs(4),
        |screen| {
            screen.contains("Whitespace: ignore-changes · 1 hidden by whitespace")
                && !screen.contains("spacing.txt")
        },
    )?;
    session.quit()
}

#[test]
fn configured_whitespace_mode_is_applied_at_startup() -> Result {
    // Given
    let fixture = Fixture::spacing()?;
    fixture.write("spacing.txt", "onetwo\n")?;
    fixture.configure("diff { whitespace \"ignore-all\"; }\n")?;
    // When
    let mut session = fixture.start()?;
    // Then
    assert!(!session.screen.contains("spacing.txt"));
    assert!(session.screen.contains("No changes"));
    assert!(!session.screen.contains("Whitespace:"));
    session.quit()
}

#[test]
fn substantive_edit_reveals_a_whitespace_hidden_file() -> Result {
    // Given
    let fixture = Fixture::spacing()?;
    let mut session = fixture.start()?;
    session.send("w")?;
    session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")?;
    // When
    fixture.write("spacing.txt", "SUBSTANTIVE_EDIT\n")?;
    // Then
    session.wait_for(
        "substantive edit in tree and diff",
        Duration::from_secs(4),
        |screen| screen.contains("SUBSTANTIVE_EDIT") && screen.badge("All changes", "1"),
    )?;
    session.quit()
}

#[test]
fn whitespace_toast_expires_without_input() -> Result {
    // Given
    let fixture = Fixture::spacing()?;
    let mut session = fixture.start()?;
    session.send("w")?;
    session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")?;
    session.send("w")?;
    session.wait("Whitespace: exact · 0 hidden by whitespace")?;
    // When
    session.wait_for("expired toast", Duration::from_secs(5), |screen| {
        !screen.contains("Whitespace:")
    })?;
    // Then
    assert!(session.screen.contains("one  two"));
    session.quit()
}

#[test]
fn resize_and_exit_restore_terminal_modes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    session.resize(80, 25)?;
    // Then
    session.select("All changes")?;
    session.quit()
}

#[test]
fn idle_view_stops_writing_frames() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.observe(Duration::from_millis(100))?;
    let bytes = session.bytes_read;
    // When
    session.observe(Duration::from_millis(250))?;
    // Then
    assert_eq!(session.bytes_read, bytes);
    session.quit()
}

#[test]
fn edit_during_syntax_enrichment_keeps_latest_content() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let source = fixture.syntax_file("large.ts", 600)?;
    let mut session = fixture.start()?;
    fixture.write(
        "large.ts",
        format!("{source}export const last = \"INTERMEDIATE\";\n"),
    )?;
    session.wait("INTERMEDIATE")?;
    session.wait("Rendering")?;
    // When
    fixture.write(
        "large.ts",
        format!("{source}export const last = \"FINAL_VALUE\";\n"),
    )?;
    // Then
    session.wait_for("latest rendered edit", Duration::from_secs(15), |screen| {
        screen.contains("FINAL_VALUE")
            && screen.contains("Watching")
            && !screen.contains("INTERMEDIATE")
    })?;
    session.quit()
}

#[test]
fn navigation_cancels_large_file_enrichment() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let source = fixture.syntax_file("a/large.ts", 10000)?;
    fixture.write(
        "a/large.ts",
        format!("{source}export const last = \"LATE_VALUE\";\n"),
    )?;
    let mut session = fixture.spawn(&fixture.root().join("src"), &[], &[])?;
    session.wait("LATE_VALUE")?;
    session.wait("Rendering")?;
    session.send("n")?;
    session.select("large.ts")?;
    // When
    session.send("n")?;
    // Then
    session.wait_for(
        "small file after cancelled enrichment",
        Duration::from_secs(5),
        |screen| {
            screen.selected("main.rs")
                && screen.contains("Watching")
                && !screen.contains("LATE_VALUE")
        },
    )?;
    session.quit()
}

#[test]
fn space_stages_and_unstages_root_with_automatic_pane_refresh() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    session.send(" ")?;
    // Then
    session.wait("Staged changes")?;
    session.wait_for("staged panes", Duration::from_secs(4), |screen| {
        screen.contains("STAGED")
            && !screen.contains("UNSTAGED")
            && !screen.contains("UNTRACKED")
            && screen.contains("working")
    })?;
    assert!(fixture.git(&["diff"])?.is_empty());
    assert_eq!(fixture.git(&["show", ":notes.txt"])?, "untracked note");
    session.send(" ")?;
    session.wait("Unstaged changes")?;
    session.wait("UNTRACKED")?;
    session.wait("UNSTAGED")?;
    assert!(fixture.git(&["diff", "--cached"])?.is_empty());
    session.quit()
}

#[test]
fn comparison_mode_rejects_staging() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start_with(fixture.root(), &["diff", "HEAD", "HEAD"], &[])?;
    let before = fixture.git(&["diff", "--cached"])?;
    // When
    session.send(" ")?;
    // Then
    session.wait("Staging is unavailable in comparison mode")?;
    assert_eq!(fixture.git(&["diff", "--cached"])?, before);
    session.quit()
}

#[test]
fn repeated_space_during_a_locked_operation_does_not_queue_toggles() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    fixture.write(".git/index.lock", "")?;
    // When
    session.send("          ")?;
    // Then
    session.wait("index.lock")?;
    fs::remove_file(fixture.path(".git/index.lock"))?;
    session.send(" ")?;
    session.wait("Staged changes")?;
    assert!(fixture.git(&["diff"])?.is_empty());
    session.quit()
}

#[test]
fn filtered_root_stages_hidden_descendants() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("fnotes\r")?;
    session.wait("filter: notes")?;
    // When
    session.send(" ")?;
    // Then
    session.wait("Staged changes")?;
    assert!(fixture.git(&["diff"])?.is_empty());
    assert_eq!(
        fixture.git(&["show", ":src/main.rs"])?,
        "fn main() { println!(\"working\"); }"
    );
    session.quit()
}

#[test]
fn diff_focus_stages_only_the_visible_file() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("fnotes\r\t")?;
    session.wait("untracked note")?;
    // When
    session.send(" ")?;
    // Then
    session.wait("Staged changes")?;
    assert_eq!(fixture.git(&["show", ":notes.txt"])?, "untracked note");
    assert_eq!(
        fixture.git(&["show", ":src/main.rs"])?,
        "fn main() { println!(\"staged\"); }"
    );
    session.quit()
}

#[test]
fn hidden_whitespace_changes_can_be_staged_from_root() -> Result {
    // Given
    let fixture = Fixture::spacing()?;
    fixture.configure("diff { whitespace \"ignore-all\"; }")?;
    let mut session = fixture.start()?;
    // When
    session.send(" ")?;
    // Then
    session.wait("Staged changes")?;
    assert!(fixture.git(&["diff"])?.is_empty());
    assert_eq!(fixture.git(&["show", ":spacing.txt"])?, "one  two");
    session.quit()
}

#[test]
fn collapsed_directory_stages_every_descendant() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("src/deep/hidden.txt", "hidden descendant")?;
    let mut session = fixture.start()?;
    session.send("j")?;
    session.select("src")?;
    session.send("h")?;
    // When
    session.send(" ")?;
    // Then
    session.wait("Staged changes")?;
    assert_eq!(
        fixture.git(&["show", ":src/deep/hidden.txt"])?,
        "hidden descendant"
    );
    assert!(
        fixture
            .git(&["status", "--porcelain"])?
            .contains("?? notes.txt")
    );
    session.quit()
}

fn tree_background(
    screen: &terminal::Screen,
    name: &str,
) -> Option<alacritty_terminal::vte::ansi::Color> {
    (0..screen.rows())
        .find(|y| {
            screen
                .row(*y)
                .chars()
                .take(32)
                .collect::<String>()
                .contains(name)
        })
        .map(|y| screen.cell(2, y).bg)
}

#[test]
fn internal_staging_is_quiet_but_external_edits_highlight() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    let background = tree_background(&session.screen, "main.rs").ok_or("Missing tree file")?;
    // When
    session.send(" ")?;
    session.wait("Staged changes")?;
    session.observe(Duration::from_millis(500))?;
    // Then
    assert_eq!(
        tree_background(&session.screen, "main.rs"),
        Some(background)
    );
    assert_eq!(
        tree_background(&session.screen, "notes.txt"),
        Some(background)
    );
    session.send(" ")?;
    session.wait("Unstaged changes")?;
    session.observe(Duration::from_millis(500))?;
    assert_eq!(
        tree_background(&session.screen, "main.rs"),
        Some(background)
    );
    fixture.write("src/main.rs", "external edit after unstage\n")?;
    session.wait_for(
        "external activity highlight",
        Duration::from_secs(4),
        |screen| tree_background(screen, "main.rs").is_some_and(|color| color != background),
    )?;
    session.quit()
}

#[test]
fn external_staging_still_highlights_the_tree() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    let background = tree_background(&session.screen, "main.rs").ok_or("Missing tree file")?;
    // When
    fixture.git(&["add", "src/main.rs"])?;
    // Then
    session.wait_for(
        "external staging highlight",
        Duration::from_secs(4),
        |screen| tree_background(screen, "main.rs").is_some_and(|color| color != background),
    )?;
    session.quit()
}
