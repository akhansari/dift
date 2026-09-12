use crate::{
    config::Config,
    git::ChangeTarget,
    git::DISCARD_MODES,
    input::{Action, Scope},
    render::Palette,
    state::State,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub(super) fn draw(frame: &mut Frame, state: &State, config: &Config, palette: &Palette) {
    let Some(menu) = state.discard_menu() else {
        return;
    };
    let area = frame.area();
    let width = area.width.saturating_sub(2).min(96);
    let target = match &menu.target {
        ChangeTarget::File(path) => format!("File: {:?}", path.as_os_str()),
        ChangeTarget::Subtree(path) if path.as_os_str().is_empty() => {
            "All changes in repository".into()
        }
        ChangeTarget::Subtree(path) => format!("Folder: {:?}", path.as_os_str()),
    };
    let mut details = vec![Line::raw(target)];
    if let Some(notice) = &menu.notice {
        details.push(Line::raw(notice.clone()));
    }
    if menu.executing {
        details.push(Line::raw("Discarding changes…"));
    }
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
    } else {
        details.push(Line::raw("Preparing discard preview…"));
    }
    let details: Vec<_> = details
        .into_iter()
        .flat_map(|line| wrap(line, width.saturating_sub(2) as usize))
        .collect();
    let height = area
        .height
        .saturating_sub(2)
        .min((details.len() + 6).min(26) as u16);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let style = palette.base().bg(palette.panel);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Discard changes ")
        .style(style)
        .border_style(style.fg(palette.accent));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if area.width < 40 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("Enlarge terminal to review discard. Escape cancels.")
                .wrap(Wrap { trim: false })
                .style(style),
            inner,
        );
        return;
    }
    let detail_rect = Rect {
        height: inner.height.saturating_sub(4),
        ..inner
    };
    let maximum = details.len().saturating_sub(detail_rect.height as usize);
    let paragraph = Paragraph::new(details).style(style);
    frame.render_widget(
        paragraph.scroll((menu.scroll.min(maximum).min(u16::MAX as usize) as u16, 0)),
        detail_rect,
    );
    let options = ["Discard unstaged changes", "Discard all changes", "Cancel"];
    for (index, label) in options.iter().enumerate() {
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
        frame.render_widget(
            Paragraph::new(text).style(row_style),
            Rect::new(inner.x, detail_rect.bottom() + index as u16, inner.width, 1),
        );
    }
    let keys = &config.bindings;
    let hint = format!(
        "{} accept · {} cancel{}",
        keys.keys(Scope::Input, Action::Accept),
        keys.keys(Scope::Input, Action::Cancel),
        if maximum > 0 {
            " · PgUp/PgDn details"
        } else {
            ""
        }
    );
    frame.render_widget(
        Paragraph::new(hint).style(style.fg(palette.muted)),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
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
