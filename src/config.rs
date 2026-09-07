mod error;
pub use error::{ConfigError, ValidationError};

use crate::{
    git::Limits,
    input::{Action, Bindings, Key, Scope},
};
use kdl::{KdlDocument, KdlNode};
use std::{collections::BTreeSet, fs, path::PathBuf, time::Duration};

/// Whitespace policy used by Git when comparing tracked files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Whitespace {
    /// Show every whitespace edit.
    #[default]
    Exact,
    /// Ignore changes in whitespace amounts and trailing whitespace.
    IgnoreChanges,
    /// Also ignore separators being inserted or removed.
    IgnoreAll,
}

impl Whitespace {
    pub fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::IgnoreChanges => "ignore-changes",
            Self::IgnoreAll => "ignore-all",
        }
    }

    pub(crate) fn git_flag(self) -> Option<&'static str> {
        match self {
            Self::Exact => None,
            Self::IgnoreChanges => Some("--ignore-space-change"),
            Self::IgnoreAll => Some("--ignore-all-space"),
        }
    }
}

/// Granularity of emphasis within changed lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Emphasis {
    /// Emphasize whole Unicode words.
    #[default]
    Word,
    /// Emphasize changed user-perceived characters.
    Grapheme,
    /// Retain line colors without within-line emphasis.
    Off,
}

/// Optional file-tree glyphs; the terminal must provide the selected font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Icons {
    #[default]
    None,
    Nerd,
}

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
    /// Granularity of emphasis within changed lines in either layout.
    pub emphasis: Emphasis,
    /// File-tree icons, enabled explicitly for terminals using Nerd Fonts.
    pub icons: Icons,
    /// Initial whitespace comparison policy; session toggles do not persist.
    pub whitespace: Whitespace,
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
            emphasis: Emphasis::default(),
            icons: Icons::default(),
            whitespace: Whitespace::default(),
            limits: Limits::default(),
            reconcile: Duration::from_secs(30),
        }
    }
}

impl Config {
    pub fn load(explicit: Option<PathBuf>) -> Result<Self, ConfigError> {
        let required = explicit.is_some();
        let Some(path) = explicit.or_else(default_path) else {
            return Ok(Self::default());
        };
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|source| ConfigError::AtPath {
                path,
                source: Box::new(source),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read { path, source }),
        }
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let doc: KdlDocument = text.parse()?;
        let mut config = Self::default();
        let mut seen = BTreeSet::new();
        let mut explicit_theme = false;
        for node in doc.nodes() {
            let name = node.name().value();
            if !seen.insert(name) {
                return Err(ValidationError::Duplicate {
                    setting: format!("{name} group"),
                }
                .into());
            }
            if !node.entries().is_empty() {
                return Err(ValidationError::ArgumentsNotAllowed {
                    setting: format!("{name} group"),
                }
                .into());
            }
            let children = node
                .children()
                .ok_or_else(|| ValidationError::ChildBlockRequired {
                    setting: name.into(),
                })?;
            let mut child_names = BTreeSet::new();
            for child in children.nodes() {
                let child_name = child.name().value();
                if !child_names.insert(child_name) {
                    return Err(ValidationError::Duplicate {
                        setting: format!("{name}.{child_name}"),
                    }
                    .into());
                }
                match name {
                    "keybindings" => {
                        let scope =
                            Scope::parse(child_name).ok_or_else(|| ValidationError::Unknown {
                                setting: format!("keybinding scope {child_name}"),
                            })?;
                        if !child.entries().is_empty() {
                            return Err(ValidationError::ArgumentsNotAllowed {
                                setting: "Keybinding scopes".into(),
                            }
                            .into());
                        }
                        let actions = child.children().ok_or_else(|| {
                            ValidationError::ChildBlockRequired {
                                setting: "A keybinding scope".into(),
                            }
                        })?;
                        for binding in actions.nodes() {
                            let action =
                                Action::parse(binding.name().value()).ok_or_else(|| {
                                    ValidationError::Unknown {
                                        setting: format!("action {}", binding.name().value()),
                                    }
                                })?;
                            if binding.children().is_some() {
                                return Err(ValidationError::ChildBlockNotAllowed {
                                    setting: "Bindings".into(),
                                }
                                .into());
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
                                        return Err(
                                            ValidationError::UnsupportedBindingProperty.into()
                                        );
                                    }
                                } else {
                                    keys.push(Key::parse(entry.value().as_string().ok_or_else(
                                        || ValidationError::ExpectedValue {
                                            setting: "Keys".into(),
                                            expected: "strings",
                                        },
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
                                    return Err(ValidationError::UnknownBindingOperation {
                                        operation: operation.into(),
                                    }
                                    .into());
                                }
                            }
                        }
                    }
                    "appearance" => match child_name {
                        "icons" => {
                            config.icons = match string_value(child)? {
                                "none" => Icons::None,
                                "nerd" => Icons::Nerd,
                                _ => {
                                    return Err(ValidationError::ExpectedValue {
                                        setting: "appearance.icons".into(),
                                        expected: "none or nerd",
                                    }
                                    .into());
                                }
                            }
                        }
                        "mode" => {
                            config.light = match string_value(child)? {
                                "dark" => false,
                                "light" => true,
                                _ => {
                                    return Err(ValidationError::ExpectedValue {
                                        setting: "appearance.mode".into(),
                                        expected: "dark or light",
                                    }
                                    .into());
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
                                    return Err(ValidationError::ExpectedValue {
                                        setting: "appearance.layout".into(),
                                        expected: "unified or split",
                                    }
                                    .into());
                                }
                            }
                        }
                        "emphasis" => {
                            config.emphasis = match string_value(child)? {
                                "word" => Emphasis::Word,
                                "grapheme" => Emphasis::Grapheme,
                                "off" => Emphasis::Off,
                                _ => {
                                    return Err(ValidationError::ExpectedValue {
                                        setting: "appearance.emphasis".into(),
                                        expected: "word, grapheme, or off",
                                    }
                                    .into());
                                }
                            }
                        }
                        _ => {
                            return Err(ValidationError::Unknown {
                                setting: format!("appearance setting {child_name}"),
                            }
                            .into());
                        }
                    },
                    "diff" if child_name == "whitespace" => {
                        config.whitespace = match string_value(child)? {
                            "exact" => Whitespace::Exact,
                            "ignore-changes" => Whitespace::IgnoreChanges,
                            "ignore-all" => Whitespace::IgnoreAll,
                            _ => {
                                return Err(ValidationError::ExpectedValue {
                                    setting: "diff.whitespace".into(),
                                    expected: "exact, ignore-changes, or ignore-all",
                                }
                                .into());
                            }
                        };
                    }
                    "limits" => {
                        match child_name {
                            "max-file-bytes" => config.limits.bytes = number_value(child)?,
                            "max-diff-lines" => {
                                config.limits.lines = usize::try_from(number_value(child)?)
                                    .map_err(|source| ValidationError::ValueTooLarge {
                                        setting: "max-diff-lines",
                                        source,
                                    })?
                            }
                            _ => {
                                return Err(ValidationError::Unknown {
                                    setting: format!("limits setting {child_name}"),
                                }
                                .into());
                            }
                        }
                    }
                    "watch" if child_name == "reconcile-seconds" => {
                        config.reconcile = Duration::from_secs(number_value(child)?)
                    }
                    _ => {
                        return Err(ValidationError::Unknown {
                            setting: format!("setting {name}.{child_name}"),
                        }
                        .into());
                    }
                }
            }
            if !matches!(
                name,
                "keybindings" | "appearance" | "diff" | "limits" | "watch"
            ) {
                return Err(ValidationError::Unknown {
                    setting: format!("configuration group {name}"),
                }
                .into());
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
            r#"// dift configuration. Omitted settings retain defaults.
// Keys are single events: ctrl-q, shift-tab, alt-j, etc.
// Bindings support do="set" (default), do="add", do="remove".
appearance {
    mode "dark"
    theme "base16-ocean.dark"
    layout "unified"
    emphasis "word" // word, grapheme, or off
    icons "none" // none or nerd (requires a Nerd Font)
}
diff {
    whitespace "exact" // exact, ignore-changes, or ignore-all
}
limits {
    max-file-bytes 2097152
    max-diff-lines 20000
}
watch {
    reconcile-seconds 30
}
keybindings {
"#,
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
        return Err(ValidationError::ExpectedValue {
            setting: node.name().value().into(),
            expected: "exactly one value",
        }
        .into());
    }
    Ok(node.entries()[0].value())
}
fn string_value(node: &KdlNode) -> Result<&str, ConfigError> {
    value(node)?.as_string().ok_or_else(|| {
        ValidationError::ExpectedValue {
            setting: node.name().value().into(),
            expected: "a string",
        }
        .into()
    })
}
fn number_value(node: &KdlNode) -> Result<u64, ConfigError> {
    value(node)?
        .as_integer()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            ValidationError::ExpectedValue {
                setting: node.name().value().into(),
                expected: "a positive integer",
            }
            .into()
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
mod tests;
