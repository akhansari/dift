use super::{Action, Editor};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A message body with line-aware movement and the same edits as prompt input.
pub(crate) struct MultilineEditor {
    editor: Editor,
    column: Option<usize>,
}

impl Default for MultilineEditor {
    fn default() -> Self {
        Self {
            editor: Editor::new(String::new()),
            column: None,
        }
    }
}

impl MultilineEditor {
    pub(crate) fn editor(&self) -> &Editor {
        &self.editor
    }

    pub(crate) fn insert(&mut self, text: &str) {
        let clean: String = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
            .chars()
            .filter(|ch| *ch == '\n' || !ch.is_control())
            .collect();
        self.editor.text.insert_str(self.editor.cursor, &clean);
        self.editor.cursor += clean.len();
        self.column = None;
    }

    pub(crate) fn edit(&mut self, action: Action) {
        let start = self.editor.text[..self.editor.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let end = self.editor.text[self.editor.cursor..]
            .find('\n')
            .map_or(self.editor.text.len(), |index| self.editor.cursor + index);
        match action {
            Action::Up | Action::Down => {
                let column = *self
                    .column
                    .get_or_insert_with(|| self.editor.text[start..self.editor.cursor].width());
                let target = match action {
                    Action::Up if start > 0 => {
                        let end = start - 1;
                        let start = self.editor.text[..end].rfind('\n').map_or(0, |i| i + 1);
                        Some(start..end)
                    }
                    Action::Down if end < self.editor.text.len() => {
                        let start = end + 1;
                        let end = self.editor.text[start..]
                            .find('\n')
                            .map_or(self.editor.text.len(), |i| start + i);
                        Some(start..end)
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    let mut cells = 0;
                    let offset = self.editor.text[target.clone()]
                        .char_indices()
                        .find_map(|(index, ch)| {
                            cells += ch.width().unwrap_or(0);
                            (cells > column).then_some(index)
                        })
                        .unwrap_or(target.len());
                    self.editor.cursor = target.start + offset;
                }
                return;
            }
            Action::CursorHome => self.editor.cursor = start,
            Action::CursorEnd => self.editor.cursor = end,
            Action::ClearBefore => {
                self.editor
                    .text
                    .replace_range(start..self.editor.cursor, "");
                self.editor.cursor = start;
            }
            Action::ClearAfter => self.editor.text.replace_range(self.editor.cursor..end, ""),
            action => {
                self.editor.edit(action);
            }
        }
        self.column = None;
    }
}
