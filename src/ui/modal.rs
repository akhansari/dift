//! Shared geometry and frame for centered dialogs; content and scrolling stay with each caller.
use crate::{input::Action, render::Palette};
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

pub(super) struct Modal {
    pub(super) rect: Rect,
    footer: Vec<Line<'static>>,
}

impl Modal {
    /// Size from content, leaving one cell above it and between it and the footer.
    pub(super) fn new(area: Rect, width: usize, height: usize, footer: Vec<Line<'static>>) -> Self {
        Self::sized(area, width, height, footer, false)
    }

    /// Keep the commit dialog's minimum and maximum percentage sizes.
    pub(super) fn commit(
        area: Rect,
        width: usize,
        height: usize,
        footer: Vec<Line<'static>>,
    ) -> Self {
        Self::sized(area, width, height, footer, true)
    }

    fn sized(
        area: Rect,
        width: usize,
        height: usize,
        footer: Vec<Line<'static>>,
        commit_limits: bool,
    ) -> Self {
        let width = width
            .max(footer.iter().map(Line::width).max().unwrap_or(0))
            .saturating_add(4);
        let height = height
            .saturating_add(footer.len())
            .saturating_add(3 + usize::from(!footer.is_empty()));
        let limit = |available: u16, desired: usize, minimum_percent: u32| {
            let available = u32::from(available);
            let minimum = (available * minimum_percent).div_ceil(100);
            let maximum = (available * 4 / 5).max(minimum);
            desired.clamp(minimum as usize, maximum as usize) as u16
        };
        let width = if commit_limits {
            limit(area.width, width, 50)
        } else {
            width.min(usize::from(area.width)) as u16
        };
        let height = if commit_limits {
            limit(area.height, height, 35)
        } else {
            height.min(usize::from(area.height)) as u16
        };
        Self {
            rect: Rect::new(
                area.x + (area.width - width) / 2,
                area.y + (area.height - height) / 2,
                width,
                height,
            ),
            footer,
        }
    }

    fn interior(&self) -> Rect {
        Block::default()
            .borders(Borders::ALL)
            .inner(self.rect)
            .inner(Margin::new(1, 0))
    }

    fn footer_start(&self, interior: Rect) -> u16 {
        interior
            .bottom()
            .saturating_sub(self.footer.len().min(usize::from(interior.height)) as u16)
    }

    pub(super) fn inner(&self) -> Rect {
        let inner = self.interior();
        let y = inner.y.saturating_add(1).min(inner.bottom());
        let bottom = if self.footer.is_empty() {
            inner.bottom()
        } else {
            self.footer_start(inner).saturating_sub(1)
        };
        Rect {
            y,
            height: bottom.saturating_sub(y),
            ..inner
        }
    }

    pub(super) fn draw(&self, frame: &mut Frame, title: &str, palette: &Palette) -> Rect {
        let style = palette.base().bg(palette.panel);
        frame.render_widget(Clear, self.rect);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(style)
                .border_style(style.fg(palette.accent)),
            self.rect,
        );
        let interior = self.interior();
        let start = self.footer_start(interior);
        for (index, line) in self
            .footer
            .iter()
            .skip(
                self.footer
                    .len()
                    .saturating_sub(usize::from(interior.height)),
            )
            .enumerate()
        {
            frame.render_widget(
                line,
                Rect::new(interior.x, start + index as u16, interior.width, 1),
            );
        }
        self.inner()
    }
}

pub(super) fn hint(text: impl Into<String>, palette: &Palette) -> Line<'static> {
    Line::styled(
        text.into(),
        palette.base().bg(palette.panel).fg(palette.muted),
    )
}

pub(super) fn scroll_hint(keys: impl Fn(Action) -> String) -> String {
    format!(
        "{}/{} scroll · {}/{} page · {} close",
        keys(Action::Down),
        keys(Action::Up),
        keys(Action::PageDown),
        keys(Action::PageUp),
        keys(Action::Cancel),
    )
}

pub(super) fn widest(text: &str) -> usize {
    text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

/// Measure rows with the same wrapping algorithm used to draw the paragraph.
pub(super) fn wrapped_rows(text: &str, width: usize) -> usize {
    Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .line_count(width.min(usize::from(u16::MAX)) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_content_uses_only_its_own_size() {
        // Given: a terminal larger than the desired dialog.
        let area = Rect::new(10, 5, 120, 40);
        // When: centering the dialog.
        let modal = Modal::new(area, 4, 3, Vec::new());
        // Then: the border and padding fit the content without percentage minimums.
        assert_eq!(modal.rect, Rect::new(66, 22, 8, 6));
    }

    #[test]
    fn long_content_uses_the_available_area() {
        // Given: a terminal smaller than the desired content.
        let area = Rect::new(10, 5, 120, 40);
        // When: centering the dialog.
        let modal = Modal::new(area, 400, 100, Vec::new());
        // Then: the modal can occupy the full area.
        assert_eq!(modal.rect, area);
    }

    #[test]
    fn short_commit_keeps_percentage_minimums() {
        // Given: content smaller than a commit dialog's minimum size.
        let area = Rect::new(10, 5, 120, 40);
        // When: sizing the commit dialog.
        let modal = Modal::commit(area, 4, 3, Vec::new());
        // Then: the minimum width and height still apply.
        assert_eq!(modal.rect, Rect::new(40, 18, 60, 14));
    }

    #[test]
    fn long_commit_keeps_percentage_maximums() {
        // Given: content larger than a commit dialog's maximum size.
        let area = Rect::new(10, 5, 120, 40);
        // When: sizing the commit dialog.
        let modal = Modal::commit(area, 400, 100, Vec::new());
        // Then: the maximum width and height still apply.
        assert_eq!(modal.rect, Rect::new(22, 9, 96, 32));
    }

    #[test]
    fn tiny_terminals_still_keep_the_dialog_on_screen() {
        // Given: a terminal with only one cell in each dimension.
        let area = Rect::new(0, 0, 1, 1);
        // When: sizing a dialog with more content than the screen.
        let modal = Modal::new(area, 100, 100, Vec::new());
        // Then: the modal occupies the available cell rather than disappearing.
        assert_eq!(modal.rect, area);
    }

    #[test]
    fn content_has_one_cell_of_horizontal_and_top_padding() {
        // Given: a terminal with room for the requested content.
        let area = Rect::new(0, 0, 100, 40);
        // When: sizing the modal around that content.
        let modal = Modal::new(area, 50, 16, Vec::new());
        // Then: content starts one row below the top border and horizontal padding is preserved.
        assert_eq!(modal.rect, Rect::new(23, 10, 54, 19));
        assert_eq!(modal.inner(), Rect::new(25, 12, 50, 16));
    }

    #[test]
    fn footer_has_one_blank_row_between_it_and_the_content() {
        // Given: a dialog with a shortcut hint wider than its content.
        let area = Rect::new(0, 0, 80, 24);
        // When: sizing the modal.
        let modal = Modal::new(area, 4, 3, vec![Line::raw("esc close")]);
        // Then: the footer affects width and height but not the content area.
        assert_eq!(modal.rect, Rect::new(33, 8, 13, 8));
        assert_eq!(modal.inner(), Rect::new(35, 10, 9, 3));
    }

    #[test]
    fn long_words_wrap_at_word_boundaries_before_sizing_the_modal() {
        // Given: several words that each fit on their own but not two abreast.
        let text = "longword anotherlongword thirdlongword";
        // When: measuring their displayed rows.
        let rows = wrapped_rows(text, 15);
        // Then: each word gets a row instead of using only total character width.
        assert_eq!(rows, 3);
    }

    #[test]
    fn preserved_spaces_are_measured_with_the_rendered_text() {
        // Given: whitespace that the details dialog does not trim.
        let text = "a    b";
        // When: sizing the text at a narrow viewport width.
        let rows = wrapped_rows(text, 3);
        // Then: the measurement uses the paragraph's actual wrapping behavior.
        assert_eq!(rows, 2);
    }

    #[test]
    fn empty_lines_remain_visible_in_the_measured_height() {
        // Given: a blank line in a multiline commit message.
        let text = "subject\n\nbody";
        // When: measuring the dialog content.
        let rows = wrapped_rows(text, 20);
        // Then: the blank row remains part of its scrollable content.
        assert_eq!(rows, 3);
    }
}
