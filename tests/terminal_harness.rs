//! Fast parser-contract tests. These never launch dift or allocate a PTY.
#![cfg(unix)]

#[allow(dead_code)]
#[path = "terminal/screen.rs"]
mod screen;
type Result<T = ()> = dift::AppResult<T>;
use screen::Screen;

#[test]
fn fragmented_utf8_and_escape_sequences_preserve_screen_cells() {
    // Given
    let mut screen = Screen::new(12, 2);
    // When
    screen.feed(b"\x1b[3");
    screen.feed(b"1mA\xe4");
    screen.feed(b"\xbd\xa0B");
    // Then
    assert_eq!(screen.cell_text(1, 0), "你");
    assert_eq!(screen.cell_text(2, 0), "");
    assert_eq!(screen.cell_text(3, 0), "B");
    assert_eq!(
        screen.cell(0, 0).fg,
        alacritty_terminal::vte::ansi::Color::Named(alacritty_terminal::vte::ansi::NamedColor::Red)
    );
}

#[test]
fn shrinking_through_a_wide_character_then_erasing_is_supported() {
    // Given
    let mut screen = Screen::new(4, 2);
    screen.feed("你".as_bytes());
    // When
    screen.resize(1, 2);
    screen.feed(b"\x1b[H\x1b[2KX");
    // Then
    assert_eq!(screen.row(0), "X ");
}

#[test]
fn leaving_alternate_screen_restores_original_content() {
    // Given
    let mut screen = Screen::new(12, 2);
    screen.feed(b"original\x1b[?1049h\x1b[Htemporary");
    // When
    screen.feed(b"\x1b[?1049l");
    // Then
    assert!(!screen.alternate_screen());
    assert!(screen.row(0).starts_with("original"));
}

#[test]
fn inverse_selection_preserves_combining_characters() {
    // Given
    let mut screen = Screen::new(12, 2);
    // When
    screen.feed("\x1b[7me\u{301}\x1b[27m plain".as_bytes());
    // Then
    assert_eq!(screen.highlighted(), ["e\u{301}"]);
}

#[test]
fn syntax_measurement_requires_distinct_foreground_colors() {
    // Given
    let mut screen = Screen::new(120, 35);
    screen.feed(b"\x1b[1;35H    1     1 + \x1b[38;2;20;30;40mlet\x1b[38;2;80;90;100m value");
    // When
    let measurement = screen.visible_source();
    // Then
    assert_eq!(measurement, (true, true));
}
