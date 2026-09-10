use crate::{
    model::Snapshot,
    render::{
        Document,
        worker::{RenderRequest, RenderResult},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default)]
struct Displayed {
    selection: PathBuf,
    query: String,
    pending_search_jump: bool,
}

/// Owns the displayed document, search position, and render request lifecycle.
#[derive(Default)]
pub(super) struct DiffState {
    displayed: Displayed,
    pub(super) search: String,
    pub(super) split: bool,
    pub(super) show_line_numbers: bool,
    /// Zero-based document row at the top of the viewport.
    pub(super) scroll: usize,
    pub(super) document: Arc<Document>,
    /// Anchors retained while a whitespace mode awaits its first preview.
    transition_document: Option<Arc<Document>>,
    /// Latest request identifier used to reject stale results.
    generation: u64,
    pub(super) rendering: bool,
    pub(super) match_index: Option<usize>,
}

impl DiffState {
    pub(super) fn new(split: bool) -> Self {
        Self {
            split,
            show_line_numbers: true,
            ..Self::default()
        }
    }

    pub(super) fn invalidate(&mut self, whitespace_changed: bool) {
        if whitespace_changed {
            // Keep source anchors until the first preview of the new mode arrives.
            self.transition_document
                .get_or_insert_with(|| Arc::clone(&self.document));
            self.document = Arc::new(Document::default());
            self.match_index = None;
        }
        // Reject results from the old snapshot before its replacement is requested.
        self.generation += 1;
    }

    pub(super) fn request(
        &mut self,
        snapshot: Arc<Snapshot>,
        selected: Vec<usize>,
        selection: &Path,
        width: usize,
        height: usize,
    ) -> RenderRequest {
        self.generation += 1;
        self.rendering = true;
        self.displayed.pending_search_jump |=
            self.displayed.query != self.search && !self.search.is_empty();
        RenderRequest {
            generation: self.generation,
            snapshot,
            selected,
            width,
            split: self.split,
            show_line_numbers: self.show_line_numbers,
            query: self.search.clone(),
            // A prefix preview cannot preserve an anchor beyond that prefix.
            preview_rows: if self.displayed.selection == selection && self.scroll > 0 {
                0
            } else {
                height.saturating_mul(2).max(16)
            },
        }
    }

    pub(super) fn apply_render(
        &mut self,
        result: RenderResult,
        height: usize,
        selection: &Path,
    ) -> bool {
        if result.generation != self.generation
            || (result.document.partial && self.displayed.selection == selection && self.scroll > 0)
        {
            // Scrolling can happen after requesting a prefix preview. Keep the
            // current source position until a complete document is available.
            return false;
        }
        self.scroll = if self.displayed.selection == selection {
            result.document.reanchored_scroll(
                self.transition_document.as_ref().unwrap_or(&self.document),
                self.scroll,
            )
        } else {
            0
        }
        .min(result.document.rows.len().saturating_sub(height));
        self.document = result.document;
        self.transition_document = None;
        self.rendering = !result.finished;
        if self.displayed.query != self.search || self.displayed.selection != selection {
            self.match_index = None;
        } else if let Some(index) = self.match_index {
            self.match_index = (!self.document.hits.is_empty())
                .then_some(index.min(self.document.hits.len().saturating_sub(1)));
        }
        if self.displayed.pending_search_jump
            && !self.search.is_empty()
            && !self.document.hits.is_empty()
        {
            self.match_index = None;
            self.jump_match(true, height);
            self.displayed.pending_search_jump = false;
        }
        if result.finished || self.search.is_empty() {
            self.displayed.pending_search_jump = false;
        }
        selection.clone_into(&mut self.displayed.selection);
        self.displayed.query.clone_from(&self.search);
        true
    }
    pub(super) fn scroll_by(&mut self, amount: usize, down: bool, height: usize) {
        self.scroll = if down {
            self.scroll.saturating_add(amount)
        } else {
            self.scroll.saturating_sub(amount)
        }
        .min(self.document.rows.len().saturating_sub(height));
    }

    pub(super) fn jump_match(&mut self, forward: bool, height: usize) {
        let hits = &self.document.hits;
        if hits.is_empty() {
            return;
        }
        let index = match self.match_index {
            Some(i) if forward => (i + 1) % hits.len(),
            Some(i) => (i + hits.len() - 1) % hits.len(),
            None if forward => hits.iter().position(|h| h.row >= self.scroll).unwrap_or(0),
            None => hits
                .iter()
                .rposition(|h| h.row < self.scroll)
                .unwrap_or(hits.len() - 1),
        };
        self.match_index = Some(index);
        let context =
            (height / 3).max(crate::render::FILE_HEADER_ROWS.min(height.saturating_sub(1)));
        self.scroll = hits[index]
            .row
            .saturating_sub(context)
            .min(self.document.rows.len().saturating_sub(height));
    }
}
