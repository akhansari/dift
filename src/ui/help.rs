use super::modal;
use crate::{
    input::{Action, Bindings, Scope},
    render::Palette,
    state::State,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The same wrapped rows and viewport drive drawing and input bounds.
pub(super) struct Help {
    modal: modal::Modal,
    content: Rect,
    lines: Vec<Line<'static>>,
}

impl Help {
    pub(super) fn new(area: Rect, bindings: &Bindings, palette: &Palette) -> Self {
        let title = concat!(" Dift ", env!("CARGO_PKG_VERSION"), " Keybindings ");
        let footer = modal::hint(
            modal::scroll_hint(|action| bindings.labels(Scope::Global, action)),
            palette,
        );
        let width = bindings
            .0
            .iter()
            .flat_map(|(scope, actions)| {
                actions.keys().map(|action| {
                    bindings.labels(*scope, *action).width() + 2 + description(*action).width()
                })
            })
            .max()
            .unwrap_or(0)
            .max("GENERAL · available in tree, diff and history".width())
            .max(title.width())
            .max(footer.width())
            + 1;
        let modal = modal::Modal::new(area, width, 0, Vec::new());
        let inner = modal.inner();
        let available = usize::from(inner.width.saturating_sub(1));
        let key_width = bindings
            .0
            .iter()
            .flat_map(|(scope, actions)| {
                actions
                    .keys()
                    .map(|action| bindings.labels(*scope, *action).width())
            })
            .max()
            .unwrap_or(0)
            .min(available / 2)
            .max(1);
        let mut lines = Vec::new();
        for scope in Scope::ALL {
            let Some(actions) = bindings.0.get(&scope).filter(|actions| !actions.is_empty()) else {
                continue;
            };
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            let heading = match scope {
                Scope::Global => "GENERAL · available in tree, diff and history",
                Scope::Tree => "FILE TREE",
                Scope::Diff => "DIFF VIEW",
                Scope::History => "COMMIT HISTORY AND DETAIL",
                Scope::Input => "TEXT INPUT · filter, find and search",
            };
            lines.extend(wrap(heading, available).into_iter().map(|text| {
                Line::styled(
                    text,
                    palette
                        .base()
                        .bg(palette.panel)
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                )
            }));
            for action in actions.keys() {
                let keys = bindings.labels(scope, *action);
                if available < 32 {
                    lines.extend(wrap(&keys, available).into_iter().map(|text| {
                        Line::styled(text, palette.base().bg(palette.panel).fg(palette.accent))
                    }));
                    lines.extend(
                        wrap(description(*action), available.saturating_sub(2))
                            .into_iter()
                            .map(|text| Line::raw(format!("  {text}"))),
                    );
                } else {
                    let keys = wrap(&keys, key_width);
                    let descriptions = wrap(
                        description(*action),
                        available.saturating_sub(key_width + 2),
                    );
                    for row in 0..keys.len().max(descriptions.len()) {
                        let key = keys.get(row).map_or("", String::as_str);
                        let description = descriptions.get(row).map_or("", String::as_str);
                        lines.push(Line::from(vec![
                            Span::styled(
                                key.to_owned(),
                                palette.base().bg(palette.panel).fg(palette.accent),
                            ),
                            Span::raw(" ".repeat(key_width.saturating_sub(key.width()) + 2)),
                            Span::raw(description.to_owned()),
                        ]));
                    }
                }
            }
        }
        let modal = modal::Modal::new(area, width, lines.len(), vec![footer]);
        let inner = modal.inner();
        let content = Rect {
            width: inner.width.saturating_sub(1),
            ..inner
        };
        Self {
            modal,
            content,
            lines,
        }
    }

    pub(super) fn rect(&self) -> Rect {
        self.modal.rect
    }

    pub(super) fn sync(&self, state: &mut State) {
        state.resize_help(self.lines.len(), usize::from(self.content.height));
    }

    pub(super) fn draw(self, frame: &mut Frame, offset: usize, palette: &Palette) {
        self.modal.draw(
            frame,
            concat!(" Dift ", env!("CARGO_PKG_VERSION"), " Keybindings "),
            palette,
        );
        let maximum = self
            .lines
            .len()
            .saturating_sub(usize::from(self.content.height));
        let offset = offset.min(maximum);
        let content: Vec<_> = self
            .lines
            .into_iter()
            .skip(offset)
            .take(usize::from(self.content.height))
            .collect();
        frame.render_widget(
            Paragraph::new(content).style(palette.base().bg(palette.panel)),
            self.content,
        );
        if !self.content.is_empty() && maximum > 0 {
            let mut state = ScrollbarState::new(maximum + 1)
                .position(offset)
                .viewport_content_length(usize::from(self.content.height));
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(Some(" "))
                    .thumb_symbol("┃")
                    .track_style(palette.base().bg(palette.panel))
                    .thumb_style(palette.base().bg(palette.panel).fg(palette.accent)),
                Rect {
                    x: self.modal.rect.right().saturating_sub(2),
                    width: 1,
                    ..self.content
                },
                &mut state,
            );
        }
    }
}

/// Wrap words using terminal-cell widths, splitting unusually long bindings as needed.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        if used > 0 && used + 1 + word.width() > width {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if used > 0 {
            line.push(' ');
            used += 1;
        }
        for character in word.chars() {
            let cells = character.width().unwrap_or(0);
            if used > 0 && used + cells > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(character);
            used += cells;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn description(action: Action) -> &'static str {
    match action {
        Action::Quit => "Quit dift",
        Action::Sidebar => "Show or hide the file tree",
        Action::Layout => "Switch unified / split diff",
        Action::LineNumbers => "Show or hide line numbers",
        Action::Whitespace => "Cycle whitespace comparison modes",
        Action::Focus => "Switch focus between tree and diff",
        Action::NextFile => "Next file",
        Action::PreviousFile => "Previous file",
        Action::Down => "Move down",
        Action::Up => "Move up",
        Action::Left => "Collapse directory",
        Action::Right => "Expand directory",
        Action::HalfDown => "Half page down",
        Action::HalfUp => "Half page up",
        Action::PageDown => "Page down",
        Action::PageUp => "Page up",
        Action::First => "Go to the beginning",
        Action::Last => "Go to the end",
        Action::Filter => "Filter files",
        Action::ChangeFilter => "Filter changes by category and status",
        Action::Find => "Find a file",
        Action::NextFindMatch => "Next matching file",
        Action::Search => "Search the diff",
        Action::NextMatch => "Next search match",
        Action::PreviousMatch => "Previous search match",
        Action::Cancel => "Cancel or close",
        Action::Accept => "Confirm input",
        Action::Help => "Open or close help",
        Action::Refresh => "Refresh changes",
        Action::ToggleStage => "Stage or unstage changes",
        Action::Discard => "Choose changes to discard",
        Action::Commit => "Commit staged changes",
        Action::OpenEditor => "Open editor at the visible change",
        Action::OpenEditorPlain => "Open editor without a line position",
        Action::History => "Toggle commit history (Cancel returns from a commit)",
        Action::MarkBase => "Mark or clear the comparison base",
        Action::Compare => "Compare base with highlighted target",
        Action::ToggleFiles => "Toggle commit changes / complete file tree",
        Action::Swap => "Swap comparison base and target",
        Action::CommitInfo => "Show full commit message and metadata",
        Action::CursorLeft => "Move cursor left",
        Action::CursorRight => "Move cursor right",
        Action::CursorHome => "Move cursor to the beginning",
        Action::CursorEnd => "Move cursor to the end",
        Action::Backspace => "Delete previous character",
        Action::Delete => "Delete next character",
        Action::DeleteWord => "Delete previous word",
        Action::ClearBefore => "Delete text before the cursor",
        Action::ClearAfter => "Delete text after the cursor",
    }
}
