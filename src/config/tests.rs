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
    assert_eq!(config.limits.lines, 20_000);
    Ok(())
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
