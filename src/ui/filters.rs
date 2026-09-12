use crate::{
    render::Palette,
    state::{STATUSES, State},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub(super) fn draw(frame: &mut Frame, state: &State, palette: &Palette) {
    let Some(menu) = state.filter_menu() else {
        return;
    };
    let area = frame.area();
    let width = area.width.saturating_sub(2).min(64);
    let height = area
        .height
        .saturating_sub(2)
        .min(if state.comparison() { 16 } else { 20 });
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let style = palette.base().bg(palette.panel);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Change filters ")
        .style(style)
        .border_style(style.fg(palette.accent));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.width < 48 || inner.height < if state.comparison() { 14 } else { 18 } {
        frame.render_widget(
            Paragraph::new("Enlarge terminal to edit filters. Esc cancels.")
                .style(style)
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
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
    lines.extend([
        Line::raw(""),
        Line::raw("None or all checked = no restriction"),
        Line::raw("↑/↓ move · Space toggle · Tab/Shift-Tab group"),
        Line::raw("Enter accept · Esc cancel · Ctrl-R reset selections"),
    ]);
    frame.render_widget(Paragraph::new(lines).style(style), inner);
}
