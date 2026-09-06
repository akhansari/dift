use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::BTreeMap, fmt};

/// An error parsing or validating keyboard shortcuts.
#[derive(Debug)]
pub struct BindingError(
    /// Human-readable error details.
    pub String,
);
impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BindingError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Global,
    Tree,
    Diff,
    Input,
}
impl Scope {
    pub const ALL: [Self; 4] = [Self::Global, Self::Tree, Self::Diff, Self::Input];
    pub fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Tree => "tree",
            Self::Diff => "diff",
            Self::Input => "input",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Quit,
    Sidebar,
    Layout,
    Focus,
    NextFile,
    PreviousFile,
    Down,
    Up,
    Left,
    Right,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
    First,
    Last,
    Filter,
    Search,
    NextMatch,
    PreviousMatch,
    Cancel,
    Accept,
    Help,
    Refresh,
    CursorLeft,
    CursorRight,
    CursorHome,
    CursorEnd,
    Backspace,
    Delete,
    DeleteWord,
    ClearBefore,
    ClearAfter,
}
impl Action {
    pub const ALL: [Self; 33] = [
        Self::Quit,
        Self::Sidebar,
        Self::Layout,
        Self::Focus,
        Self::NextFile,
        Self::PreviousFile,
        Self::Down,
        Self::Up,
        Self::Left,
        Self::Right,
        Self::HalfDown,
        Self::HalfUp,
        Self::PageDown,
        Self::PageUp,
        Self::First,
        Self::Last,
        Self::Filter,
        Self::Search,
        Self::NextMatch,
        Self::PreviousMatch,
        Self::Cancel,
        Self::Accept,
        Self::Help,
        Self::Refresh,
        Self::CursorLeft,
        Self::CursorRight,
        Self::CursorHome,
        Self::CursorEnd,
        Self::Backspace,
        Self::Delete,
        Self::DeleteWord,
        Self::ClearBefore,
        Self::ClearAfter,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Sidebar => "toggle-sidebar",
            Self::Layout => "toggle-layout",
            Self::Focus => "switch-focus",
            Self::NextFile => "next-file",
            Self::PreviousFile => "previous-file",
            Self::Down => "down",
            Self::Up => "up",
            Self::Left => "collapse",
            Self::Right => "expand",
            Self::HalfDown => "half-page-down",
            Self::HalfUp => "half-page-up",
            Self::PageDown => "page-down",
            Self::PageUp => "page-up",
            Self::First => "first",
            Self::Last => "last",
            Self::Filter => "filter",
            Self::Search => "search",
            Self::NextMatch => "next-match",
            Self::PreviousMatch => "previous-match",
            Self::Cancel => "cancel",
            Self::Accept => "accept",
            Self::Help => "help",
            Self::Refresh => "refresh",
            Self::CursorLeft => "cursor-left",
            Self::CursorRight => "cursor-right",
            Self::CursorHome => "cursor-home",
            Self::CursorEnd => "cursor-end",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::DeleteWord => "delete-word",
            Self::ClearBefore => "clear-before",
            Self::ClearAfter => "clear-after",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == s)
    }
    pub fn input_only(self) -> bool {
        matches!(
            self,
            Self::CursorLeft
                | Self::CursorRight
                | Self::CursorHome
                | Self::CursorEnd
                | Self::Backspace
                | Self::Delete
                | Self::DeleteWord
                | Self::ClearBefore
                | Self::ClearAfter
        )
    }
}

/// A normalized key press used to match keyboard shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// The character or special key pressed.
    pub code: KeyCode,
    /// Control, Alt, and Shift modifiers after normalization.
    pub modifiers: KeyModifiers,
}
impl Key {
    pub fn event(event: KeyEvent) -> Self {
        let mut key = Self {
            code: event.code,
            modifiers: event.modifiers
                & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT),
        };
        if let KeyCode::Char(c) = key.code {
            if key.modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_alphabetic() {
                key.code = KeyCode::Char(c.to_ascii_uppercase());
            }
            if !c.is_ascii_alphabetic()
                || c.is_uppercase()
                || key.modifiers.contains(KeyModifiers::SHIFT)
            {
                key.modifiers.remove(KeyModifiers::SHIFT);
            }
        }
        if key.code == KeyCode::BackTab {
            key.code = KeyCode::Tab;
            key.modifiers.insert(KeyModifiers::SHIFT);
        }
        key
    }
    pub fn parse(raw: &str) -> Result<Self, BindingError> {
        let mut rest = raw.to_owned();
        let mut mods = KeyModifiers::empty();
        loop {
            let lower = rest.to_ascii_lowercase();
            let prefix = [
                ("ctrl-", KeyModifiers::CONTROL),
                ("alt-", KeyModifiers::ALT),
                ("shift-", KeyModifiers::SHIFT),
            ]
            .into_iter()
            .find(|(p, _)| lower.starts_with(p));
            if let Some((prefix, modifier)) = prefix {
                mods.insert(modifier);
                rest = rest[prefix.len()..].to_owned();
            } else {
                break;
            }
        }
        let code = match rest.to_ascii_lowercase().as_str() {
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "space" => KeyCode::Char(' '),
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "page-up" => KeyCode::PageUp,
            "pagedown" | "page-down" => KeyCode::PageDown,
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            _ => {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) => KeyCode::Char(ch),
                    _ => {
                        return Err(BindingError(format!(
                            "Invalid key {raw:?}; use a single key with optional ctrl-/alt-/shift- modifiers"
                        )));
                    }
                }
            }
        };
        Ok(Self::event(KeyEvent::new(code, mods)))
    }
}
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            write!(f, "ctrl-")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            write!(f, "alt-")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            write!(f, "shift-")?;
        }
        match self.code {
            KeyCode::Char(' ') => write!(f, "space"),
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                write!(f, "shift-{}", c.to_ascii_lowercase())
            }
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::Enter => write!(f, "enter"),
            KeyCode::Esc => write!(f, "escape"),
            KeyCode::Tab | KeyCode::BackTab => write!(f, "tab"),
            KeyCode::Up => write!(f, "up"),
            KeyCode::Down => write!(f, "down"),
            KeyCode::Left => write!(f, "left"),
            KeyCode::Right => write!(f, "right"),
            KeyCode::Home => write!(f, "home"),
            KeyCode::End => write!(f, "end"),
            KeyCode::PageUp => write!(f, "page-up"),
            KeyCode::PageDown => write!(f, "page-down"),
            KeyCode::Backspace => write!(f, "backspace"),
            KeyCode::Delete => write!(f, "delete"),
            _ => write!(f, "unsupported"),
        }
    }
}

/// Keyboard shortcuts available in each input scope.
#[derive(Debug, Clone, Default)]
pub struct Bindings(
    /// Keys assigned to each action, grouped by scope.
    pub BTreeMap<Scope, BTreeMap<Action, Vec<Key>>>,
);
impl Bindings {
    pub fn defaults() -> Self {
        let normal = [
            (Action::Quit, "q ctrl-c"),
            (Action::Sidebar, "e"),
            (Action::Layout, "s"),
            (Action::Focus, "tab shift-tab"),
            (Action::NextFile, "n"),
            (Action::PreviousFile, "p"),
            (Action::Down, "down j"),
            (Action::Up, "up k"),
            (Action::HalfDown, "d"),
            (Action::HalfUp, "u"),
            (Action::PageDown, "page-down"),
            (Action::PageUp, "page-up"),
            (Action::First, "home"),
            (Action::Last, "end"),
            (Action::Filter, "f"),
            (Action::Search, "/"),
            (Action::NextMatch, "]"),
            (Action::PreviousMatch, "["),
            (Action::Cancel, "escape"),
            (Action::Accept, "enter"),
            (Action::Help, "?"),
            (Action::Refresh, "r"),
        ];
        let tree = [(Action::Left, "left h"), (Action::Right, "right l")];
        let input = [
            (Action::Quit, "ctrl-c"),
            (Action::Cancel, "escape"),
            (Action::Accept, "enter"),
            (Action::CursorLeft, "left"),
            (Action::CursorRight, "right"),
            (Action::CursorHome, "home ctrl-a"),
            (Action::CursorEnd, "end ctrl-e"),
            (Action::Backspace, "backspace"),
            (Action::Delete, "delete"),
            (Action::DeleteWord, "ctrl-w"),
            (Action::ClearBefore, "ctrl-u"),
            (Action::ClearAfter, "ctrl-k"),
        ];
        Self(
            [
                (Scope::Global, normal.as_slice()),
                (Scope::Tree, tree.as_slice()),
                (Scope::Diff, &[]),
                (Scope::Input, input.as_slice()),
            ]
            .into_iter()
            .map(|(scope, entries)| {
                (
                    scope,
                    entries
                        .iter()
                        .map(|(action, keys)| {
                            (
                                *action,
                                keys.split_whitespace()
                                    .filter_map(|s| Key::parse(s).ok())
                                    .collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
        )
    }

    pub fn action(&self, scope: Scope, event: KeyEvent) -> Option<Action> {
        let key = Key::event(event);
        let find = |s| {
            self.0.get(&s).and_then(|actions| {
                actions
                    .iter()
                    .find(|(_, keys)| keys.contains(&key))
                    .map(|(a, _)| *a)
            })
        };
        find(scope).or_else(|| {
            if scope == Scope::Input {
                None
            } else {
                find(Scope::Global)
            }
        })
    }
    pub fn keys(&self, scope: Scope, action: Action) -> String {
        self.0
            .get(&scope)
            .and_then(|s| s.get(&action))
            .map(|keys| {
                keys.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default()
    }
    pub fn validate(&self) -> Result<(), BindingError> {
        for scope in Scope::ALL {
            let entries = self
                .0
                .get(&scope)
                .into_iter()
                .chain(if matches!(scope, Scope::Tree | Scope::Diff) {
                    self.0.get(&Scope::Global)
                } else {
                    None
                })
                .flat_map(|map| map.iter());
            let mut seen: Vec<(&Key, Action)> = Vec::new();
            for (action, keys) in entries {
                if scope == Scope::Input
                    && !action.input_only()
                    && !matches!(action, Action::Quit | Action::Accept | Action::Cancel)
                    && !keys.is_empty()
                {
                    return Err(BindingError(format!(
                        "{} is unavailable while editing input",
                        action.name()
                    )));
                }
                if scope != Scope::Input && action.input_only() {
                    return Err(BindingError(format!(
                        "{} belongs in the input scope",
                        action.name()
                    )));
                }
                for key in keys {
                    if let Some((_, other)) = seen.iter().find(|(k, a)| *k == key && a != action) {
                        return Err(BindingError(format!(
                            "Conflicting {} key {key}: {} and {}",
                            scope.name(),
                            other.name(),
                            action.name()
                        )));
                    }
                    seen.push((key, *action));
                }
            }
        }
        for (scope, action) in [
            (Scope::Global, Action::Quit),
            (Scope::Input, Action::Quit),
            (Scope::Input, Action::Cancel),
            (Scope::Input, Action::Accept),
        ] {
            if self
                .0
                .get(&scope)
                .and_then(|s| s.get(&action))
                .is_none_or(Vec::is_empty)
            {
                return Err(BindingError(format!(
                    "{}.{} needs at least one key",
                    scope.name(),
                    action.name()
                )));
            }
        }
        Ok(())
    }
}

/// A single-line text editor for filter and search prompts.
#[derive(Debug, Clone)]
pub struct Editor {
    /// Current prompt text.
    pub text: String,
    /// Cursor byte offset at a UTF-8 character boundary.
    pub cursor: usize,
}
impl Editor {
    pub fn new(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }
    pub fn insert(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }
    pub fn edit(&mut self, action: Action) {
        let previous = self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        let next = self.text[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
            .unwrap_or(self.cursor);
        match action {
            Action::CursorLeft => self.cursor = previous,
            Action::CursorRight => self.cursor = next,
            Action::CursorHome => self.cursor = 0,
            Action::CursorEnd => self.cursor = self.text.len(),
            Action::Backspace => {
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            Action::Delete => self.text.replace_range(self.cursor..next, ""),
            Action::ClearBefore => {
                self.text.replace_range(..self.cursor, "");
                self.cursor = 0;
            }
            Action::ClearAfter => self.text.truncate(self.cursor),
            Action::DeleteWord => {
                let prefix = self.text[..self.cursor].trim_end();
                let start = prefix
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backspace_deletes_a_unicode_character_without_corrupting_utf8() {
        // Given
        let mut editor = Editor::new("été界".into());
        // When
        editor.edit(Action::Backspace);
        // Then
        assert_eq!(editor.text, "été");
        assert_eq!(editor.cursor, 5);
    }
    #[test]
    fn query_input_keeps_normal_quit_key_available_as_text() {
        // Given
        let bindings = Bindings::defaults();
        // When
        let action = bindings.action(
            Scope::Input,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        // Then
        assert_eq!(action, None);
    }
}
