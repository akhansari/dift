use ratatui::{
    style::{Modifier, Style},
    text::Span,
};
use unicode_width::UnicodeWidthStr;

/// Fit a name without splitting graphemes, keeping the extension where possible.
/// Match indices remain relative to the original name after middle truncation.
pub(crate) fn name_spans<'a>(
    name: &'a str,
    matches: &[usize],
    width: usize,
    style: Style,
) -> Vec<Span<'a>> {
    if width == 0 {
        return Vec::new();
    }
    let source = Span::raw(name);
    let mut byte = 0;
    let graphemes: Vec<_> = source
        .styled_graphemes(Style::default())
        .enumerate()
        .map(|(index, grapheme)| {
            let start = byte;
            byte += grapheme.symbol.len();
            (start..byte, index, grapheme.symbol.width())
        })
        .collect();
    let truncated = name.width() > width;
    let (head, tail) = if truncated {
        let remaining = width - 1;
        let extension = name
            .rfind('.')
            .filter(|index| *index > 0)
            .map_or(0, |index| name[index..].width());
        let tail_budget = (remaining / 2)
            .max(extension)
            .min(remaining.saturating_sub(1));
        let mut tail_width = 0;
        let tail = graphemes
            .iter()
            .rev()
            .take_while(|(_, _, cells)| {
                if tail_width + cells > tail_budget {
                    return false;
                }
                tail_width += cells;
                true
            })
            .count();
        let mut head_width = 0;
        let head = graphemes
            .iter()
            .take_while(|(_, _, cells)| {
                if head_width + cells > remaining - tail_width {
                    return false;
                }
                head_width += cells;
                true
            })
            .count();
        (head, graphemes.len() - tail)
    } else {
        (graphemes.len(), graphemes.len())
    };
    let styled = |(bytes, index, _): &(std::ops::Range<usize>, usize, usize)| {
        Span::styled(
            &name[bytes.clone()],
            if matches.binary_search(index).is_ok() {
                style.add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            } else {
                style
            },
        )
    };
    let mut spans: Vec<_> = graphemes[..head].iter().map(styled).collect();
    if truncated {
        spans.push(Span::styled("…", style));
        spans.extend(graphemes[tail..].iter().map(styled));
    }
    spans
}
