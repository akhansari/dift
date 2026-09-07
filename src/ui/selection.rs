use super::ViewAreas;
use crate::{
    render::{CodeRegion, DiffSide, Document, Palette},
    state::State,
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::{Modifier, Style},
};
use std::{ops::Range, path::PathBuf, sync::Arc};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    row: usize,
    column: usize,
}

struct Drag {
    document: Arc<Document>,
    selected_path: PathBuf,
    area: Rect,
    section: Range<usize>,
    side: Option<DiffSide>,
    start: Point,
    end: Point,
    dragging: bool,
}

/// Source selection excludes diff decorations and copies on release.
#[derive(Default)]
pub(super) struct Selection {
    drag: Option<Drag>,
    pending_copy: Option<String>,
}

impl Selection {
    pub(super) fn take_copy(&mut self) -> Option<String> {
        self.pending_copy.take()
    }

    pub(super) fn clear(&mut self) -> bool {
        self.pending_copy = None;
        self.drag.take().is_some()
    }

    pub(super) fn reconcile(&mut self, state: &State, view: &ViewAreas) -> bool {
        if self.drag.as_ref().is_some_and(|drag| {
            !Arc::ptr_eq(&drag.document, state.shared_document())
                || drag.selected_path != state.selection()
                || drag.area != view.diff_content
                || state.help()
                || state.prompt().is_some()
        }) {
            self.clear()
        } else {
            false
        }
    }

    pub(super) fn handle_mouse(
        &mut self,
        state: &mut State,
        view: &ViewAreas,
        mouse: MouseEvent,
    ) -> bool {
        let cleared = self.reconcile(state, view);
        if !mouse.modifiers.is_empty() {
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left))
                && let Some(drag) = &mut self.drag
            {
                drag.dragging = false;
            }
            return cleared;
        }
        match mouse.kind {
            MouseEventKind::Down(_) => {
                let cleared = self.clear() || cleared;
                if mouse.kind != MouseEventKind::Down(MouseButton::Left)
                    || !view.diff_content.contains((mouse.column, mouse.row).into())
                    || view
                        .header(state)
                        .is_some_and(|(_, area)| area.contains((mouse.column, mouse.row).into()))
                {
                    return cleared;
                }
                let point = Point {
                    row: state.scroll() + usize::from(mouse.row - view.diff_content.y),
                    column: usize::from(mouse.column - view.diff_content.x),
                };
                let document = state.shared_document();
                let Some(region) = document.rows.get(point.row).and_then(|row| {
                    row.code
                        .iter()
                        .find(|region| region.columns.contains(&point.column))
                }) else {
                    return cleared;
                };
                let Some(section) = document
                    .file_sections
                    .iter()
                    .find(|section| section.contains(&point.row))
                else {
                    return cleared;
                };
                self.drag = Some(Drag {
                    document: Arc::clone(document),
                    selected_path: state.selection().into(),
                    area: view.diff_content,
                    section: section.clone(),
                    side: region.side,
                    start: point,
                    end: point,
                    dragging: true,
                });
                state.focus_diff();
                true
            }
            MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown => {
                let Some(drag) = &mut self.drag else {
                    return cleared;
                };
                if !drag.dragging {
                    return cleared;
                }
                let previous = drag.end;
                drag.update(state, view, mouse);
                if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                    drag.dragging = false;
                    if drag.start != drag.end {
                        let text = drag.text();
                        self.pending_copy = (!text.is_empty()).then_some(text);
                    }
                }
                let changed = previous != drag.end;
                if !drag.dragging && drag.start == drag.end {
                    self.drag = None;
                }
                cleared || changed
            }
            _ => cleared,
        }
    }

    pub(super) fn draw(
        &self,
        buffer: &mut Buffer,
        state: &State,
        view: &ViewAreas,
        palette: Palette,
    ) {
        let Some(drag) = &self.drag else {
            return;
        };
        if drag.start == drag.end {
            return;
        }
        let start = drag.start.min(drag.end);
        let end = drag.start.max(drag.end);
        let area = view.diff_content;
        let header = view.header(state).map(|(_, area)| area);
        for y in area.y..area.bottom() {
            if header.is_some_and(|header| y >= header.y && y < header.bottom()) {
                continue;
            }
            let index = state.scroll() + usize::from(y - area.y);
            if index < start.row || index > end.row {
                continue;
            }
            let Some(region) = drag.region(index) else {
                continue;
            };
            let left = if index == start.row {
                start.column
            } else {
                region.columns.start
            };
            let right = if index == end.row {
                end.column.saturating_add(1)
            } else {
                region.columns.end
            };
            let mut column = region.columns.start;
            while column < region.columns.end.min(area.width as usize) {
                let x = area.x + column as u16;
                // Buffer symbols already contain combining marks; style wide glyphs as a whole.
                let width = buffer[(x, y)].symbol().width().max(1);
                if column < right && column + width > left {
                    for selected in column
                        ..(column + width)
                            .min(region.columns.end)
                            .min(area.width as usize)
                    {
                        // Reverse video also remains visible when NO_COLOR suppresses colors.
                        buffer[(area.x + selected as u16, y)].set_style(
                            Style::default()
                                .fg(palette.accent)
                                .bg(palette.panel)
                                .add_modifier(Modifier::REVERSED),
                        );
                    }
                }
                column += width;
            }
        }
    }
}

impl Drag {
    fn text(&self) -> String {
        let start = self.start.min(self.end);
        let end = self.start.max(self.end);
        let mut text = String::new();
        let mut previous: Option<(&crate::render::Anchor, usize)> = None;
        for index in start.row..=end.row {
            let Some(row) = self.document.rows.get(index) else {
                continue;
            };
            let Some(region) = row.code.iter().find(|region| region.side == self.side) else {
                continue;
            };
            let Some(anchor) = row.anchors.get(region.anchor) else {
                continue;
            };
            let left = if index == start.row {
                start.column
            } else {
                region.columns.start
            };
            let right = if index == end.row {
                end.column + 1
            } else {
                region.columns.end
            };
            let mut cells = region
                .source
                .iter()
                .filter(|cell| cell.columns.start < right && cell.columns.end > left);
            let first = cells.next();
            let bytes = first
                .map(|first| first.bytes.start..cells.next_back().unwrap_or(first).bytes.end)
                .unwrap_or(anchor.byte..anchor.byte);
            let same_line = previous.is_some_and(|(previous, _)| {
                previous.path == anchor.path
                    && previous.category == anchor.category
                    && previous.old == anchor.old
                    && previous.number == anchor.number
            });
            let byte_start = if same_line {
                previous.map_or(bytes.start, |(_, end)| end.max(bytes.start))
            } else {
                if previous.is_some() {
                    text.push('\n');
                }
                bytes.start
            };
            if let Some(fragment) = anchor.text.get(byte_start..bytes.end) {
                text.push_str(fragment);
            }
            previous = Some((anchor, bytes.end));
        }
        text
    }

    fn region(&self, row: usize) -> Option<&CodeRegion> {
        self.section.contains(&row).then_some(())?;
        self.document
            .rows
            .get(row)?
            .code
            .iter()
            .find(|region| region.side == self.side && !region.columns.is_empty())
    }

    fn update(&mut self, state: &State, view: &ViewAreas, mouse: MouseEvent) {
        let area = view.diff_content;
        if area.is_empty() {
            return;
        }
        let mut y = mouse.row.clamp(area.y, area.bottom() - 1);
        if let Some((_, header)) = view.header(state)
            && y >= header.y
            && y < header.bottom()
        {
            y = header.bottom().min(area.bottom() - 1);
        }
        let target = state.scroll() + usize::from(y - area.y);
        let candidate = self
            .region(target)
            .map(|region| (target, region))
            .or_else(|| {
                self.section
                    .clone()
                    .filter_map(|row| self.region(row).map(|region| (row, region)))
                    .min_by_key(|(row, _)| row.abs_diff(target))
            });
        if let Some((row, region)) = candidate {
            self.end = Point {
                row,
                column: usize::from(mouse.column.saturating_sub(area.x))
                    .clamp(region.columns.start, region.columns.end.saturating_sub(1)),
            };
        }
    }
}
