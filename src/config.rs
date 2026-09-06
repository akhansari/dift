use crate::{
    git::Limits,
    input::{Action, Bindings, Key, Scope},
};
use kdl::{KdlDocument, KdlNode};
use std::{collections::BTreeSet, fmt, fs, path::PathBuf, time::Duration};

/// Application settings for input, appearance, and repository updates.
#[derive(Debug, Clone)]
pub struct Config {
    /// Keyboard shortcuts grouped by scope and action.
    pub bindings: Bindings,
    /// Whether to use the light UI palette.
    pub light: bool,
    /// Name of the syntax highlighting theme.
    pub theme: String,
    /// Whether to start with a side-by-side diff layout.
    pub split: bool,
    /// Thresholds for automatically loading file contents.
    pub limits: Limits,
    /// Interval between periodic repository scans in watch mode.
    pub reconcile: Duration,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bindings: Bindings::defaults(),
            light: false,
            theme: "base16-ocean.dark".into(),
            split: false,
            limits: Limits::default(),
            reconcile: Duration::from_secs(30),
        }
    }
}

/// An error loading or validating configuration.
#[derive(Debug)]
pub struct ConfigError(
    /// Human-readable error details.
    pub String,
);
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ConfigError {}
impl From<crate::input::BindingError> for ConfigError {
    fn from(error: crate::input::BindingError) -> Self {
        Self(error.to_string())
    }
}
impl From<String> for ConfigError {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl Config {
    pub fn load(explicit: Option<PathBuf>) -> Result<Self, ConfigError> {
        let required = explicit.is_some();
        let Some(path) = explicit.or_else(default_path) else {
            return Ok(Self::default());
        };
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text)
                .map_err(|e| ConfigError(format!("Invalid config {}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => Ok(Self::default()),
            Err(e) => Err(ConfigError(format!(
                "Cannot read config {}: {e}",
                path.display()
            ))),
        }
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let doc: KdlDocument = text.parse().map_err(|e| ConfigError(format!("{e}")))?;
        let mut config = Self::default();
        let mut seen = BTreeSet::new();
        let mut explicit_theme = false;
        for node in doc.nodes() {
            let name = node.name().value();
            if !seen.insert(name) {
                return Err(ConfigError(format!("Duplicate {name} group")));
            }
            if !node.entries().is_empty() {
                return Err(ConfigError(format!(
                    "{name} is a group; values belong inside it"
                )));
            }
            let children = node
                .children()
                .ok_or_else(|| ConfigError(format!("{name} requires a child block")))?;
            let mut child_names = BTreeSet::new();
            for child in children.nodes() {
                let child_name = child.name().value();
                if !child_names.insert(child_name) {
                    return Err(ConfigError(format!("Duplicate {name}.{child_name}")));
                }
                match name {
                    "keybindings" => {
                        let scope = Scope::parse(child_name).ok_or_else(|| {
                            ConfigError(format!("Unknown keybinding scope {child_name}"))
                        })?;
                        if !child.entries().is_empty() {
                            return Err(ConfigError(
                                "Keybinding scopes cannot have arguments".into(),
                            ));
                        }
                        let actions = child.children().ok_or_else(|| {
                            ConfigError("A keybinding scope requires a child block".into())
                        })?;
                        for binding in actions.nodes() {
                            let action =
                                Action::parse(binding.name().value()).ok_or_else(|| {
                                    ConfigError(format!(
                                        "Unknown action {}",
                                        binding.name().value()
                                    ))
                                })?;
                            if binding.children().is_some() {
                                return Err(ConfigError(
                                    "Bindings cannot have child blocks".into(),
                                ));
                            }
                            let operation = binding
                                .get("do")
                                .and_then(|v| v.as_string())
                                .unwrap_or("set");
                            let mut keys = Vec::new();
                            for entry in binding.entries() {
                                if let Some(property) = entry.name() {
                                    if property.value() != "do"
                                        || entry.value().as_string().is_none()
                                    {
                                        return Err(ConfigError(
                                            "Only a string do property is supported on bindings"
                                                .into(),
                                        ));
                                    }
                                } else {
                                    keys.push(Key::parse(entry.value().as_string().ok_or_else(
                                        || ConfigError("Keys must be strings".into()),
                                    )?)?);
                                }
                            }
                            let current = config
                                .bindings
                                .0
                                .entry(scope)
                                .or_default()
                                .entry(action)
                                .or_default();
                            match operation {
                                "set" => *current = keys,
                                "add" => {
                                    for key in keys {
                                        if !current.contains(&key) {
                                            current.push(key);
                                        }
                                    }
                                }
                                "remove" => current.retain(|key| !keys.contains(key)),
                                _ => {
                                    return Err(ConfigError(format!(
                                        "Unknown binding operation {operation}"
                                    )));
                                }
                            }
                        }
                    }
                    "appearance" => match child_name {
                        "mode" => {
                            config.light = match string_value(child)? {
                                "dark" => false,
                                "light" => true,
                                _ => {
                                    return Err(ConfigError(
                                        "appearance.mode must be dark or light".into(),
                                    ));
                                }
                            }
                        }
                        "theme" => {
                            config.theme = string_value(child)?.into();
                            explicit_theme = true;
                        }
                        "layout" => {
                            config.split = match string_value(child)? {
                                "unified" => false,
                                "split" => true,
                                _ => {
                                    return Err(ConfigError(
                                        "appearance.layout must be unified or split".into(),
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(ConfigError(format!(
                                "Unknown appearance setting {child_name}"
                            )));
                        }
                    },
                    "limits" => match child_name {
                        "max-file-bytes" => config.limits.bytes = number_value(child)?,
                        "max-diff-lines" => {
                            config.limits.lines = usize::try_from(number_value(child)?)
                                .map_err(|_| ConfigError("max-diff-lines is too large".into()))?
                        }
                        _ => {
                            return Err(ConfigError(format!(
                                "Unknown limits setting {child_name}"
                            )));
                        }
                    },
                    "watch" if child_name == "reconcile-seconds" => {
                        config.reconcile = Duration::from_secs(number_value(child)?)
                    }
                    _ => return Err(ConfigError(format!("Unknown setting {name}.{child_name}"))),
                }
            }
            if !matches!(name, "keybindings" | "appearance" | "limits" | "watch") {
                return Err(ConfigError(format!("Unknown configuration group {name}")));
            }
        }
        if config.light && !explicit_theme {
            config.theme = "InspiredGitHub".into();
        }
        config.bindings.validate()?;
        Ok(config)
    }

    pub fn template() -> String {
        let config = Self::default();
        let mut text = String::from(
            "// dift configuration. Omitted settings retain defaults.\n// Keys are single events: ctrl-q, shift-tab, alt-j, etc.\n// Bindings support do=\"set\" (default), do=\"add\", do=\"remove\".\nappearance {\n    mode \"dark\"\n    theme \"base16-ocean.dark\"\n    layout \"unified\"\n}\nlimits {\n    max-file-bytes 2097152\n    max-diff-lines 20000\n}\nwatch {\n    reconcile-seconds 30\n}\nkeybindings {\n",
        );
        for scope in Scope::ALL {
            text.push_str(&format!("    {} {{\n", scope.name()));
            if let Some(actions) = config.bindings.0.get(&scope) {
                for (action, keys) in actions {
                    text.push_str(&format!("        {}", action.name()));
                    for key in keys {
                        text.push_str(&format!(" \"{key}\""));
                    }
                    text.push('\n');
                }
            }
            text.push_str("    }\n");
        }
        text.push_str("}\n");
        text
    }
}

fn value(node: &KdlNode) -> Result<&kdl::KdlValue, ConfigError> {
    if node.children().is_some() || node.entries().len() != 1 || node.entries()[0].name().is_some()
    {
        return Err(ConfigError(format!(
            "{} expects exactly one value",
            node.name().value()
        )));
    }
    Ok(node.entries()[0].value())
}
fn string_value(node: &KdlNode) -> Result<&str, ConfigError> {
    value(node)?
        .as_string()
        .ok_or_else(|| ConfigError(format!("{} expects a string", node.name().value())))
}
fn number_value(node: &KdlNode) -> Result<u64, ConfigError> {
    value(node)?
        .as_integer()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            ConfigError(format!(
                "{} expects a positive integer",
                node.name().value()
            ))
        })
}
pub fn default_path() -> Option<PathBuf> {
    let env_path = |name| {
        std::env::var_os(name)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    env_path("XDG_CONFIG_HOME")
        .or_else(|| {
            if cfg!(windows) {
                env_path("APPDATA")
                    .or_else(|| env_path("USERPROFILE").map(|p| p.join("AppData/Roaming")))
            } else {
                env_path("HOME").map(|p| p.join(".config"))
            }
        })
        .map(|p| p.join("dift/config.kdl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
        assert!(matches!(result, Err(ConfigError(message)) if message.contains("Conflicting")));
    }
    #[test]
    fn input_scope_rejects_actions_that_cannot_run_in_an_editor() {
        // Given
        let source = "keybindings { input { next-file \"alt-n\"; }; }";
        // When
        let result = Config::parse(source);
        // Then
        assert!(
            matches!(result, Err(ConfigError(message)) if message.contains("unavailable while editing input"))
        );
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
}
