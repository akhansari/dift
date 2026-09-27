//! Names used for shortcuts shown in the terminal; configuration keeps `Key`'s canonical format.
use super::{Action, Bindings, Key, Scope};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl Key {
    pub(crate) fn label(&self) -> String {
        let mut label = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            label.push_str("ctrl-");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            label.push_str("alt-");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            label.push_str("shift-");
        }
        match self.code {
            KeyCode::Char(' ') => label.push_str("space"),
            KeyCode::Char(ch) => label.push(ch),
            code => label.push_str(match code {
                KeyCode::Enter => "enter",
                KeyCode::Esc => "esc",
                KeyCode::Tab | KeyCode::BackTab => "tab",
                KeyCode::Up => "up",
                KeyCode::Down => "down",
                KeyCode::Left => "left",
                KeyCode::Right => "right",
                KeyCode::Home => "home",
                KeyCode::End => "end",
                KeyCode::PageUp => "pgUp",
                KeyCode::PageDown => "pgDn",
                KeyCode::Backspace => "backspace",
                KeyCode::Delete => "del",
                _ => "unsupported",
            }),
        }
        label
    }
}

pub(crate) fn fixed_key_label(code: KeyCode, modifiers: KeyModifiers) -> String {
    Key::event(KeyEvent::new(code, modifiers)).label()
}

impl Bindings {
    pub(crate) fn labels(&self, scope: Scope, action: Action) -> String {
        self.0
            .get(&scope)
            .and_then(|actions| actions.get(&action))
            .map(|keys| keys.iter().map(Key::label).collect::<Vec<_>>().join("|"))
            .unwrap_or_default()
    }

    /// Follow the same scope fallback as input handling, except in text input.
    pub(crate) fn labels_for(&self, scope: Scope, action: Action) -> String {
        let local = self.labels(scope, action);
        if local.is_empty() && scope != Scope::Input {
            self.labels(Scope::Global, action)
        } else {
            local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_names_do_not_change_configuration_names() -> Result<(), super::super::BindingError> {
        // Given: a configured page-down shortcut.
        let key = Key::parse("page-down")?;
        // When: naming that key for a terminal hint.
        let label = key.label();
        // Then: the short label is distinct from the configuration spelling.
        assert_eq!(label, "pgDn");
        assert_eq!(key.to_string(), "page-down");
        Ok(())
    }

    #[test]
    fn shifted_letters_display_as_uppercase() -> Result<(), super::super::BindingError> {
        // Given: a shifted letter shortcut.
        let key = Key::parse("shift-i")?;
        // When: naming that key for a terminal hint.
        let label = key.label();
        // Then: the screen shows the letter, while configuration retains its canonical name.
        assert_eq!(label, "I");
        assert_eq!(key.to_string(), "shift-i");
        Ok(())
    }

    #[test]
    fn other_modifiers_remain_on_shifted_letters() -> Result<(), super::super::BindingError> {
        // Given: a control-shift-letter shortcut.
        let key = Key::parse("ctrl-shift-i")?;
        // When: naming that key for a terminal hint.
        let label = key.label();
        // Then: control remains visible, while shift is expressed by the uppercase letter.
        assert_eq!(label, "ctrl-I");
        Ok(())
    }

    #[test]
    fn shifted_special_keys_keep_the_shift_modifier() -> Result<(), super::super::BindingError> {
        // Given: a shifted tab shortcut.
        let key = Key::parse("shift-tab")?;
        // When: naming that key for a terminal hint.
        let label = key.label();
        // Then: non-letter keys still display shift explicitly.
        assert_eq!(label, "shift-tab");
        Ok(())
    }

    #[test]
    fn alternative_keys_have_one_consistent_separator() {
        // Given: the default navigation bindings.
        let bindings = Bindings::defaults();
        // When: formatting a single action's alternatives.
        let label = bindings.labels(Scope::Global, Action::Down);
        // Then: the key names and separator come from one formatter.
        assert_eq!(label, "down|j");
    }

    #[test]
    fn local_labels_fall_back_to_global_shortcuts() {
        // Given: a history scope without a local page-down binding.
        let bindings = Bindings::defaults();
        // When: finding the shortcut that handles history paging.
        let label = bindings.labels_for(Scope::History, Action::PageDown);
        // Then: it shows the active global binding.
        assert_eq!(label, "pgDn");
    }

    #[test]
    fn fixed_modified_keys_use_the_same_names() {
        // Given: a hard-coded command shortcut.
        // When: naming it for the commit footer.
        let label = fixed_key_label(KeyCode::Char('s'), KeyModifiers::CONTROL);
        // Then: it uses the same modifier style as configured keys.
        assert_eq!(label, "ctrl-s");
    }
}
