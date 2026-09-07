use crate::{model::LineStats, render::Palette};
use ratatui::text::Span;

/// Append a right-aligned summary only when the existing status text fits intact.
pub(super) fn append(spans: &mut Vec<Span<'_>>, totals: LineStats, width: usize, palette: Palette) {
    let used: usize = spans.iter().map(Span::width).sum();
    // Keep two spaces between sections and one cell of right padding.
    let Some(available) = width.checked_sub(used + 3) else {
        return;
    };
    let muted = palette.base().fg(palette.muted);
    let added = palette.base().fg(palette.added_foreground);
    let removed = palette.base().fg(palette.removed_foreground);
    let mut summary = vec![
        Span::styled("Tracked ", muted),
        Span::styled(format!("+{}", grouped(totals.added)), added),
        Span::styled(" ", muted),
        Span::styled(format!("−{}", grouped(totals.removed)), removed),
    ];
    let mut summary_width: usize = summary.iter().map(Span::width).sum();
    if summary_width > available {
        return;
    }
    if summary_width + 6 <= available {
        let total = u128::from(totals.added) + u128::from(totals.removed);
        let green = match (totals.added, totals.removed) {
            (0, _) => 0,
            (_, 0) => 5,
            (a, _) => ((u128::from(a) * 5 + total / 2) / total).clamp(1, 4) as usize,
        };
        summary.push(Span::styled(" ", muted));
        summary.extend((0..5).map(|index| {
            Span::styled(
                "■",
                if total == 0 {
                    muted
                } else if index < green {
                    added
                } else {
                    removed
                },
            )
        }));
        summary_width += 6;
    }
    spans.push(Span::styled(
        " ".repeat(width - used - summary_width - 1),
        muted,
    ));
    spans.extend(summary);
    spans.push(Span::styled(" ", muted));
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    digits
        .chars()
        .enumerate()
        .fold(String::new(), |mut result, (index, digit)| {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                result.push(',');
            }
            result.push(digit);
            result
        })
}
