use super::modal;
use crate::{
    input::fixed_key_label,
    render::Palette,
    state::{STATUSES, State},
};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyModifiers},
    style::Modifier,
    text::Line,
    widgets::{Paragraph, Wrap},
};

pub(super) fn draw(frame: &mut Frame, state: &State, palette: &Palette) {
    let Some(menu) = state.filter_menu() else {
        return;
    };
    let style = palette.base().bg(palette.panel);
    let filter = state.change_filter();
    let option = |index, checked, label| {
        Line::styled(
            format!(
                "{} [{}] {label}",
                if menu.selected == index { "›" } else { " " },
                if checked { "x" } else { " " }
            ),
            if menu.selected == index {
                style.fg(palette.accent).add_modifier(Modifier::BOLD)
            } else {
                style
            },
        )
    };
    let mut lines = Vec::new();
    if !state.comparison() {
        lines.push(Line::raw("Category"));
        lines.push(option(
            0,
            filter.categories[0],
            "Unstaged/Untracked".to_owned(),
        ));
        lines.push(option(1, filter.categories[1], "Staged".to_owned()));
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw("Status"));
    for (index, (status, label)) in STATUSES.iter().enumerate() {
        lines.push(option(
            index + if state.comparison() { 0 } else { 2 },
            filter.statuses[index],
            format!("{status}  {label}"),
        ));
    }
    let key = |code, modifiers| fixed_key_label(code, modifiers);
    let plain = KeyModifiers::NONE;
    let close = key(KeyCode::Esc, plain);
    let footer = vec![
        modal::hint(
            format!(
                "{}/{} move · {} toggle · {}|{} switch group",
                key(KeyCode::Up, plain),
                key(KeyCode::Down, plain),
                key(KeyCode::Char(' '), plain),
                key(KeyCode::Tab, plain),
                key(KeyCode::BackTab, plain),
            ),
            palette,
        ),
        modal::hint(
            format!(
                "{} accept · {} cancel · {} reset selections",
                key(KeyCode::Enter, plain),
                close,
                key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            ),
            palette,
        ),
    ];
    let area = frame.area();
    let width = lines.iter().map(Line::width).max().unwrap_or(0);
    let modal = modal::Modal::new(area, width, lines.len(), footer);
    let inner = modal.draw(frame, " Change filters ", palette);
    if inner.width < 48 || usize::from(inner.height) < lines.len() {
        frame.render_widget(
            Paragraph::new(format!(
                "Enlarge terminal to edit filters. {close} cancels."
            ))
            .style(style)
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    frame.render_widget(Paragraph::new(lines).style(style), inner);
}
