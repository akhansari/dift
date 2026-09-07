use super::{Change, LineKind, Palette};
use crate::{model::display_path, text::name_spans};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use std::borrow::Cow;
use unicode_width::UnicodeWidthStr;

pub(super) fn line(change: &Change, width: usize, palette: Palette) -> Line<'static> {
    let style = palette.base().bg(palette.panel);
    let muted = style.fg(palette.muted);
    let path = display_path(&change.path);
    let old_path = change.old_path.as_deref().map(display_path);
    let status = status_label(change.status);
    let category = change.category.label();
    // Conflict views contain working-tree text, not added and removed lines.
    let counts =
        (!change.oversized && change.status != 'U' && !change.hunks.is_empty()).then(|| {
            let (added, removed) = change.hunks.iter().flat_map(|hunk| &hunk.lines).fold(
                (0usize, 0usize),
                |(added, removed), line| {
                    (
                        added + usize::from(line.kind == LineKind::Added),
                        removed + usize::from(line.kind == LineKind::Removed),
                    )
                },
            );
            (format!("+{added}"), format!("−{removed}"))
        });
    // Reserve a focus marker, a separating space, and right padding.
    let available = width.saturating_sub(3);
    let name_width = basename(&path).width();
    let path_width = path.width() + old_path.as_ref().map_or(0, |old| old.width() + 3);
    let labels_width = status.width() + 3 + category.width();
    let counts = counts.filter(|(added, removed)| {
        path_width + 2 + labels_width + 2 + added.width() + 1 + removed.width() <= available
    });
    let mut metadata = Vec::new();
    if name_width + 2 + labels_width <= available {
        let color = match change.status {
            'A' | '?' => palette.added_foreground,
            'D' | 'U' => palette.removed_foreground,
            _ => palette.muted,
        };
        metadata.push(Span::styled(status.as_ref(), style.fg(color)));
        metadata.push(Span::styled(" · ", muted));
    }
    if name_width + 2 + category.width() <= available {
        metadata.push(Span::styled(category, muted));
    }
    if let Some((added, removed)) = &counts {
        metadata.push(Span::styled("  ", muted));
        metadata.push(Span::styled(
            added.as_str(),
            style.fg(palette.added_foreground),
        ));
        metadata.push(Span::styled(" ", muted));
        metadata.push(Span::styled(
            removed.as_str(),
            style.fg(palette.removed_foreground),
        ));
    }
    let metadata_width: usize = metadata.iter().map(Span::width).sum();
    let path_budget =
        available.saturating_sub(metadata_width + usize::from(!metadata.is_empty()) * 2);
    let mut spans = vec![Span::styled("  ", style)];
    if let Some(old) = old_path
        .as_deref()
        .filter(|_| path_budget >= name_width + 4)
    {
        let new_width = path.width().min(path_budget - 4);
        spans.extend(path_spans(old, path_budget - new_width - 3, muted, muted));
        spans.push(Span::styled(" → ", muted));
        spans.extend(path_spans(
            &path,
            new_width,
            muted,
            style.add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.extend(path_spans(
            &path,
            path_budget,
            muted,
            style.add_modifier(Modifier::BOLD),
        ));
    }
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used + metadata_width + 1)),
        style,
    ));
    spans.extend(metadata);
    spans.push(Span::styled(" ", style));
    Line::from(
        spans
            .into_iter()
            .map(|span| Span::styled(span.content.into_owned(), span.style))
            .collect::<Vec<_>>(),
    )
    .style(style)
}

fn basename(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn path_spans<'a>(
    path: &'a str,
    width: usize,
    directory_style: Style,
    name_style: Style,
) -> Vec<Span<'a>> {
    let name = basename(path);
    let directory = &path[..path.len() - name.len()];
    let directory_width = width.saturating_sub(name.width()).min(directory.width());
    let mut spans = if directory_width >= directory.width() {
        vec![Span::styled(directory, directory_style)]
    } else if directory_width > 1 {
        let mut spans = name_spans(
            directory.trim_end_matches('/'),
            &[],
            directory_width - 1,
            directory_style,
        );
        spans.push(Span::styled("/", directory_style));
        spans
    } else if directory_width == 1 {
        vec![Span::styled("…", directory_style)]
    } else {
        Vec::new()
    };
    spans.extend(name_spans(
        name,
        &[],
        width.saturating_sub(directory_width),
        name_style,
    ));
    spans
}

fn status_label(status: char) -> Cow<'static, str> {
    match status {
        'M' => "Modified".into(),
        'A' => "Added".into(),
        'D' => "Deleted".into(),
        'R' => "Renamed".into(),
        'C' => "Copied".into(),
        'T' => "Type changed".into(),
        'U' => "Conflict".into(),
        '?' => "Untracked".into(),
        other => format!("Changed ({})", other.escape_default()).into(),
    }
}
