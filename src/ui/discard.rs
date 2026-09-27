use super::modal;
use crate::{
    git::{ChangeTarget, DISCARD_MODES},
    input::{Action, Bindings, Scope},
    render::Palette,
    state::State,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

pub(super) fn draw(frame: &mut Frame, state: &State, bindings: &Bindings, palette: &Palette) {
    let Some(menu) = state.discard_menu() else {
        return;
    };
    let area = frame.area();
    let target = match &menu.target {
        ChangeTarget::File(path) => format!("File: {:?}", path.as_os_str()),
        ChangeTarget::Subtree(path) if path.as_os_str().is_empty() => {
            "All changes in repository".into()
        }
        ChangeTarget::Subtree(path) => format!("Folder: {:?}", path.as_os_str()),
    };
    let mut details = vec![Line::raw(target)];
    if let Some(options) = &menu.options {
        for (mode, option) in DISCARD_MODES.iter().zip(options) {
            match option {
                Ok(preview) => {
                    for path in preview.outside_paths() {
                        details.push(Line::raw(format!(
                            "  Outside target: {:?}",
                            path.as_os_str()
                        )));
                    }
                }
                Err(error) => details.push(Line::raw(format!(
                    "{} unavailable: {}",
                    mode.label(),
                    error.escape_debug()
                ))),
            }
        }
    }
    let style = palette.base().bg(palette.panel);
    let status = if menu.executing {
        Line::styled("Discarding changes…", style.fg(palette.accent))
    } else if let Some(notice) = &menu.notice {
        Line::styled(notice.clone(), style.fg(palette.removed_emphasis))
    } else if menu.options.is_none() {
        Line::styled("Preparing discard preview…", style.fg(palette.muted))
    } else {
        Line::default()
    };
    let keys = |action| bindings.labels_for(state.focus(), action);
    let hint = modal::hint(
        format!(
            "{}/{} move · {} select · {} cancel",
            keys(Action::Down),
            keys(Action::Up),
            bindings.labels(Scope::Input, Action::Accept),
            bindings.labels(Scope::Input, Action::Cancel),
        ),
        palette,
    );
    let options: Vec<_> = ["Discard unstaged changes", "Discard all changes", "Cancel"]
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            let enabled = index == 2
                || menu
                    .options
                    .as_ref()
                    .and_then(|options| options.get(index))
                    .is_some_and(|option| option.as_ref().is_ok_and(|preview| preview.files() > 0));
            let selected = index == menu.selected;
            let row_style = if selected {
                style.fg(palette.accent).add_modifier(Modifier::BOLD)
            } else if !enabled {
                style.fg(palette.muted)
            } else {
                style
            };
            let counts = menu
                .options
                .as_ref()
                .and_then(|options| options.get(index))
                .and_then(|option| option.as_ref().ok())
                .filter(|_| enabled)
                .map(|preview| {
                    format!(
                        " ({} files, {} deletions)",
                        preview.files(),
                        preview.deletions()
                    )
                })
                .unwrap_or_default();
            let text = format!(
                "{} {}{}{}",
                if selected { "›" } else { " " },
                label,
                counts,
                if enabled { "" } else { " (unavailable)" }
            );
            (text, row_style)
        })
        .collect();
    let width = details
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(
            options
                .iter()
                .map(|(text, _)| text.width())
                .max()
                .unwrap_or(0),
        )
        .max(hint.width());
    let sized = modal::Modal::new(area, width, 6, Vec::new());
    let details: Vec<_> = details
        .into_iter()
        .flat_map(|line| wrap(line, usize::from(sized.inner().width)))
        .collect();
    let modal = modal::Modal::new(area, width, details.len() + 3, vec![status, hint]);
    let inner = modal.draw(frame, " Discard changes ", palette);
    if area.width < 40 || area.height < 12 || inner.width < 32 || inner.height < 4 {
        frame.render_widget(
            Paragraph::new(format!(
                "Enlarge terminal to review discard. {} cancels.",
                bindings.labels(Scope::Input, Action::Cancel)
            ))
            .wrap(Wrap { trim: false })
            .style(style),
            inner,
        );
        return;
    }
    let detail_rect = Rect {
        height: inner.height.saturating_sub(3),
        ..inner
    };
    let maximum = details.len().saturating_sub(detail_rect.height as usize);
    let paragraph = Paragraph::new(details).style(style);
    frame.render_widget(
        paragraph.scroll((menu.scroll.min(maximum).min(u16::MAX as usize) as u16, 0)),
        detail_rect,
    );
    for (index, (text, row_style)) in options.into_iter().enumerate() {
        frame.render_widget(
            Paragraph::new(text).style(row_style),
            Rect::new(inner.x, detail_rect.bottom() + index as u16, inner.width, 1),
        );
    }
}

fn wrap(line: Line<'_>, width: usize) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthChar;
    let mut lines = Vec::new();
    let mut text = String::new();
    let mut cells = 0;
    for ch in line.spans.iter().flat_map(|span| span.content.chars()) {
        let size = ch.width().unwrap_or(0);
        if cells + size > width && !text.is_empty() {
            lines.push(Line::raw(std::mem::take(&mut text)));
            cells = 0;
        }
        text.push(ch);
        cells += size;
    }
    lines.push(Line::raw(text));
    lines
}
