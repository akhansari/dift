use super::*;
use crate::input::BindingError;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    error::Error,
    io,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> io::Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "dift-config-test-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root)?;
        Ok(Self { root })
    }
    fn path(&self) -> PathBuf {
        self.root.join("config.kdl")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn generated_config_restores_working_defaults() -> Result<(), ConfigError> {
    // Given
    let template = Config::template();
    // When
    let config = Config::parse(&template)?;
    // Then
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)
        ),
        Some(Action::NextFile)
    );
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT)
        ),
        Some(Action::Find)
    );
    assert_eq!(
        config.bindings.action(
            Scope::Diff,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)
        ),
        Some(Action::Filter)
    );
    assert_eq!(
        config.bindings.action(
            Scope::Diff,
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)
        ),
        Some(Action::Search)
    );
    assert_eq!(config.limits.lines, 20_000);
    assert_eq!(config.emphasis, Emphasis::Word);
    assert_eq!(config.icons, Icons::None);
    assert_eq!(config.whitespace, Whitespace::Exact);
    Ok(())
}

#[test]
fn omitted_icons_work_with_an_unpatched_font() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { layout \"split\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.icons, Icons::None);
    Ok(())
}

#[test]
fn nerd_font_icons_can_be_enabled_explicitly() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { icons \"nerd\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.icons, Icons::Nerd);
    Ok(())
}

#[test]
fn icons_can_be_disabled_explicitly() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { icons \"none\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.icons, Icons::None);
    Ok(())
}

#[test]
fn unsupported_icon_modes_report_the_setting() {
    // Given
    let source = "appearance { icons \"auto\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(
        matches!(result, Err(ConfigError::Validation(ValidationError::ExpectedValue { setting, .. })) if setting == "appearance.icons")
    );
}

#[test]
fn icon_modes_must_be_strings() {
    // Given
    let source = "appearance { icons 1; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Validation(
            ValidationError::ExpectedValue { .. }
        ))
    ));
}

#[test]
fn duplicate_icon_modes_are_rejected() {
    // Given
    let source = "appearance { icons \"nerd\"; icons \"none\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(
        matches!(result, Err(ConfigError::Validation(ValidationError::Duplicate { setting })) if setting == "appearance.icons")
    );
}

macro_rules! whitespace_config {
    ($name:ident, $source:expr, $expected:expr) => {
        #[test]
        fn $name() -> Result<(), ConfigError> {
            // Given
            let source = $source;
            // When
            let config = Config::parse(source)?;
            // Then
            assert_eq!(config.whitespace, $expected);
            Ok(())
        }
    };
}

whitespace_config!(omitted_whitespace_is_exact, "", Whitespace::Exact);
whitespace_config!(
    exact_whitespace_is_selectable,
    "diff { whitespace \"exact\"; }",
    Whitespace::Exact
);
whitespace_config!(
    whitespace_amounts_can_be_ignored,
    "diff { whitespace \"ignore-changes\"; }",
    Whitespace::IgnoreChanges
);
whitespace_config!(
    all_whitespace_can_be_ignored,
    "diff { whitespace \"ignore-all\"; }",
    Whitespace::IgnoreAll
);

#[test]
fn invalid_whitespace_lists_supported_modes() {
    // Given
    let source = "diff { whitespace \"trim\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(
        matches!(result, Err(ConfigError::Validation(ValidationError::ExpectedValue { setting, expected: "exact, ignore-changes, or ignore-all" })) if setting == "diff.whitespace")
    );
}

#[test]
fn duplicate_whitespace_is_rejected() {
    // Given
    let source = "diff { whitespace \"exact\"; whitespace \"ignore-all\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(
        matches!(result, Err(ConfigError::Validation(ValidationError::Duplicate { setting })) if setting == "diff.whitespace")
    );
}

#[test]
fn whitespace_requires_one_string() {
    // Given
    let source = "diff { whitespace 1; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Validation(ValidationError::ExpectedValue {
            expected: "a string",
            ..
        }))
    ));
}

#[test]
fn whitespace_toggle_binding_can_be_replaced() -> Result<(), ConfigError> {
    // Given
    let source = "keybindings { global { toggle-whitespace \"alt-w\"; }; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT)
        ),
        Some(Action::Whitespace)
    );
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)
        ),
        None
    );
    Ok(())
}

#[test]
fn omitted_emphasis_preserves_whole_word_highlighting() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { layout \"split\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.emphasis, Emphasis::Word);
    Ok(())
}

#[test]
fn word_emphasis_can_be_selected_explicitly() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { emphasis \"word\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.emphasis, Emphasis::Word);
    Ok(())
}

#[test]
fn grapheme_emphasis_can_be_selected() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { emphasis \"grapheme\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.emphasis, Emphasis::Grapheme);
    Ok(())
}

#[test]
fn inline_emphasis_can_be_disabled() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { emphasis \"off\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.emphasis, Emphasis::Off);
    Ok(())
}

#[test]
fn unsupported_emphasis_lists_the_available_choices() {
    // Given
    let source = "appearance { emphasis \"chars\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Validation(ValidationError::ExpectedValue {
            setting,
            expected: "word, grapheme, or off",
        })) if setting == "appearance.emphasis"
    ));
}

#[test]
fn emphasis_requires_a_string_value() {
    // Given
    let source = "appearance { emphasis 1; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Validation(ValidationError::ExpectedValue {
            setting,
            expected: "a string",
        })) if setting == "emphasis"
    ));
}

#[test]
fn duplicate_emphasis_settings_are_rejected() {
    // Given
    let source = "appearance { emphasis \"word\"; emphasis \"off\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Validation(ValidationError::Duplicate { setting }))
            if setting == "appearance.emphasis"
    ));
}
#[test]
fn bindings_add_and_remove_without_replacing_other_defaults() -> Result<(), ConfigError> {
    // Given
    let source =
        "keybindings { global { quit do=\"add\" \"ctrl-q\"; quit do=\"remove\" \"q\"; }; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)
        ),
        Some(Action::Quit)
    );
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
        ),
        None
    );
    Ok(())
}
#[test]
fn conflicting_effective_bindings_are_rejected() {
    // Given
    let source = "keybindings { tree { expand \"n\"; }; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Binding(BindingError::Conflict {
            scope: Scope::Tree,
            key: Key {
                code: KeyCode::Char('n'),
                ..
            },
            first: Action::Right,
            second: Action::NextFile
        }))
    ));
}
#[test]
fn input_scope_rejects_actions_that_cannot_run_in_an_editor() {
    // Given
    let source = "keybindings { input { next-file \"alt-n\"; }; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Binding(BindingError::UnavailableAction {
            scope: Scope::Input,
            action: Action::NextFile
        }))
    ));
}
#[test]
fn light_palette_uses_a_light_syntax_theme_despite_comments() -> Result<(), ConfigError> {
    // Given
    let source = "// theme is intentionally omitted\nappearance { mode \"light\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.theme, "InspiredGitHub");
    Ok(())
}
#[test]
fn explicit_theme_with_tab_separator_is_preserved() -> Result<(), ConfigError> {
    // Given
    let source = "appearance { mode \"light\"; theme\t\"Solarized (light)\"; }";
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(config.theme, "Solarized (light)");
    Ok(())
}

#[test]
fn invalid_kdl_retains_the_original_parser_error() -> crate::AppResult<()> {
    // Given
    let source = "appearance {";
    // When
    let result = Config::parse(source);
    // Then
    let error = result.err().ok_or("Invalid KDL was accepted")?;
    assert!(matches!(&error, ConfigError::Parse(_)));
    assert!(
        error
            .source()
            .is_some_and(|cause| cause.is::<kdl::KdlError>())
    );
    Ok(())
}

#[test]
fn invalid_settings_are_distinct_from_kdl_syntax_errors() -> crate::AppResult<()> {
    // Given
    let source = "limits { max-file-bytes 0; }";
    // When
    let result = Config::parse(source);
    // Then
    let error = result.err().ok_or("A zero byte limit was accepted")?;
    assert!(
        matches!(&error, ConfigError::Validation(ValidationError::ExpectedValue { setting, expected: "a positive integer" }) if setting == "max-file-bytes")
    );
    assert_eq!(
        error.to_string(),
        "max-file-bytes expects a positive integer"
    );
    assert!(
        error
            .source()
            .is_some_and(|cause| cause.is::<ValidationError>())
    );
    Ok(())
}

#[test]
fn invalid_binding_keys_retain_the_rejected_input() -> crate::AppResult<()> {
    // Given
    let source = "keybindings { global { quit \"ctrl-invalid-key\"; }; }";
    // When
    let result = Config::parse(source);
    // Then
    let error = result.err().ok_or("An invalid key was accepted")?;
    assert!(
        matches!(&error, ConfigError::Binding(BindingError::InvalidKey { input }) if input == "ctrl-invalid-key")
    );
    assert!(
        error
            .source()
            .is_some_and(|cause| cause.is::<BindingError>())
    );
    Ok(())
}

#[test]
fn an_explicit_missing_config_retains_its_path_and_io_error() -> crate::AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    // When
    let result = Config::load(Some(fixture.path()));
    // Then
    let error = result
        .err()
        .ok_or("A missing explicit config was accepted")?;
    assert!(
        matches!(&error, ConfigError::Read { path, source } if path == &fixture.path() && source.kind() == io::ErrorKind::NotFound)
    );
    assert!(error.source().is_some_and(|cause| cause.is::<io::Error>()));
    assert!(
        error
            .to_string()
            .contains(&fixture.path().display().to_string())
    );
    Ok(())
}

#[test]
fn a_config_with_invalid_utf8_retains_the_read_failure() -> crate::AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fs::write(fixture.path(), [0xff])?;
    // When
    let result = Config::load(Some(fixture.path()));
    // Then
    let error = result.err().ok_or("Non-UTF-8 config was accepted")?;
    assert!(
        matches!(&error, ConfigError::Read { path, source } if path == &fixture.path() && source.kind() == io::ErrorKind::InvalidData)
    );
    assert!(error.source().is_some_and(|cause| cause.is::<io::Error>()));
    Ok(())
}

#[test]
fn a_file_parse_error_keeps_both_path_context_and_the_parser_cause() -> crate::AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fs::write(fixture.path(), "appearance {")?;
    // When
    let result = Config::load(Some(fixture.path()));
    // Then
    let error = result.err().ok_or("Invalid KDL file was accepted")?;
    assert!(
        matches!(&error, ConfigError::AtPath { path, source } if path == &fixture.path() && matches!(source.as_ref(), ConfigError::Parse(_)))
    );
    assert!(
        error
            .source()
            .and_then(Error::source)
            .is_some_and(|cause| cause.is::<kdl::KdlError>())
    );
    assert!(
        error
            .to_string()
            .starts_with(&format!("Invalid config {}:", fixture.path().display()))
    );
    Ok(())
}

#[test]
fn a_file_validation_error_keeps_the_binding_cause() -> crate::AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fs::write(fixture.path(), "keybindings { input { accept; }; }")?;
    // When
    let result = Config::load(Some(fixture.path()));
    // Then
    let error = result
        .err()
        .ok_or("A missing accept binding was accepted")?;
    assert!(
        matches!(&error, ConfigError::AtPath { path, source } if path == &fixture.path() && matches!(source.as_ref(), ConfigError::Binding(BindingError::MissingBinding { scope: Scope::Input, action: Action::Accept })))
    );
    assert!(
        error
            .source()
            .and_then(Error::source)
            .is_some_and(|cause| cause.is::<BindingError>())
    );
    assert!(
        error
            .to_string()
            .ends_with("input.accept needs at least one key")
    );
    Ok(())
}

#[test]
fn input_only_actions_are_rejected_in_browsing_scopes() {
    // Given
    let source = "keybindings { tree { cursor-left \"alt-x\"; }; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(matches!(
        result,
        Err(ConfigError::Binding(BindingError::UnavailableAction {
            scope: Scope::Tree,
            action: Action::CursorLeft
        }))
    ));
}

#[test]
fn duplicate_settings_have_a_validation_error_with_the_setting_name() {
    // Given
    let source = "appearance { mode \"light\"; mode \"dark\"; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(
        matches!(result, Err(ConfigError::Validation(ValidationError::Duplicate { setting })) if setting == "appearance.mode")
    );
}

#[test]
fn unknown_binding_operations_are_rejected_before_applying_bindings() {
    // Given
    let source = "keybindings { global { quit do=\"append\" \"ctrl-q\"; }; }";
    // When
    let result = Config::parse(source);
    // Then
    assert!(
        matches!(result, Err(ConfigError::Validation(ValidationError::UnknownBindingOperation { operation })) if operation == "append")
    );
}

#[test]
fn line_number_toggle_binding_can_be_replaced() -> Result<(), ConfigError> {
    // Given
    let source = r#"keybindings { global { toggle-line-numbers "alt-l"; }; }"#;
    // When
    let config = Config::parse(source)?;
    // Then
    assert_eq!(
        config.bindings.action(
            Scope::Tree,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::ALT)
        ),
        Some(Action::LineNumbers)
    );
    assert_eq!(
        config.bindings.action(
            Scope::Diff,
            KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE)
        ),
        None
    );
    Ok(())
}
