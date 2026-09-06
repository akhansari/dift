mod cache;
mod control;
mod layout;
mod prepare;
mod syntax;
pub mod worker;

use crate::{
    config::Config,
    model::{Category, Change, LineKind, Snapshot, SourceLine, display_path},
};
use cache::{Cache, fingerprint};
pub use control::{Cancellation, Cancelled};
pub use layout::search_ranges;
use prepare::{Prepared, WorkBudget};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::{ops::Range, path::PathBuf, sync::Arc, time::Duration};
use syntax::{SyntaxCache, SyntaxColors};
use syntect::{highlighting::ThemeSet, parsing::SyntaxSet};

/// A source position used to preserve scroll location across renders.
#[derive(Debug, Clone)]
pub struct Anchor {
    /// File path relative to the repository root.
    pub path: Arc<PathBuf>,
    /// Diff category containing the source line.
    pub category: Category,
    /// Whether the position belongs to the old side of the diff.
    pub old: bool,
    /// One-based source line number.
    pub number: usize,
    /// Byte offset where this wrapped row begins in the source line.
    pub byte: usize,
    /// Full source line text used to locate matching content.
    pub text: Arc<str>,
}
/// A styled terminal row with its associated source positions.
#[derive(Debug, Clone)]
pub struct Row {
    /// Styled text ready for terminal display.
    pub line: Line<'static>,
    /// Source positions represented by this row.
    pub anchors: Vec<Anchor>,
}
/// A search match located in a rendered document.
#[derive(Debug, Clone)]
pub struct Hit {
    /// Zero-based document row containing the match.
    pub row: usize,
}
/// Rendered diff rows and metadata used by the UI.
#[derive(Debug, Default, Clone)]
pub struct Document {
    /// Terminal rows in display order.
    pub rows: Vec<Arc<Row>>,
    /// Search matches in document order.
    pub hits: Vec<Hit>,
    /// Number of oversized changes whose contents were omitted.
    pub excluded: usize,
    /// Whether the document contains only a preview of the selected changes.
    pub partial: bool,
}
impl Document {
    pub fn reanchored_scroll(&self, previous: &Self, row: usize) -> usize {
        if row == 0 {
            return 0;
        }
        previous
            .rows
            .iter()
            .enumerate()
            .skip(row)
            .find_map(|(index, line)| line.anchors.first().map(|anchor| (index - row, anchor)))
            .and_then(|(offset, anchor)| {
                self.locate(anchor)
                    .map(|index| index.saturating_sub(offset))
            })
            .unwrap_or(row.min(self.rows.len().saturating_sub(1)))
    }

    pub fn anchor(&self, row: usize) -> Option<Anchor> {
        self.rows
            .iter()
            .skip(row)
            .find_map(|r| r.anchors.first())
            .cloned()
    }
    pub fn locate(&self, anchor: &Anchor) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .flat_map(|(i, row)| row.anchors.iter().map(move |a| (i, a)))
            .filter(|(_, a)| {
                a.path == anchor.path
                    && a.old == anchor.old
                    && (a.category == anchor.category
                        || (a.text == anchor.text
                            && a.category != Category::Comparison
                            && anchor.category != Category::Comparison))
            })
            .min_by_key(|(_, a)| {
                (
                    a.text != anchor.text,
                    a.category != anchor.category,
                    a.number.abs_diff(anchor.number),
                    a.byte.abs_diff(anchor.byte),
                )
            })
            .map(|(i, _)| i)
    }
}

/// UI colors for text, panels, and diff highlighting.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Default background color.
    pub background: Color,
    /// Default text color.
    pub foreground: Color,
    /// Text color for secondary labels and line numbers.
    pub muted: Color,
    /// Background color for panels and headers.
    pub panel: Color,
    /// Color for focused elements and highlighted labels.
    pub accent: Color,
    /// Background color for added lines.
    pub added: Color,
    /// Background color for removed lines.
    pub removed: Color,
    /// Background color for changed text within added lines.
    pub added_emphasis: Color,
    /// Background color for changed text within removed lines.
    pub removed_emphasis: Color,
}
impl Palette {
    pub fn new(light: bool) -> Self {
        if light {
            Self {
                background: Color::Reset,
                foreground: Color::Rgb(35, 40, 52),
                muted: Color::Rgb(113, 119, 132),
                panel: Color::Rgb(236, 239, 244),
                accent: Color::Rgb(44, 91, 181),
                added: Color::Rgb(222, 243, 224),
                removed: Color::Rgb(253, 229, 232),
                added_emphasis: Color::Rgb(169, 225, 180),
                removed_emphasis: Color::Rgb(247, 181, 189),
            }
        } else {
            Self {
                background: Color::Reset,
                foreground: Color::Rgb(211, 217, 228),
                muted: Color::Rgb(114, 127, 151),
                panel: Color::Rgb(28, 33, 44),
                accent: Color::Rgb(131, 180, 255),
                added: Color::Rgb(24, 48, 39),
                removed: Color::Rgb(57, 30, 39),
                added_emphasis: Color::Rgb(33, 89, 58),
                removed_emphasis: Color::Rgb(110, 43, 58),
            }
        }
    }
    pub fn base(self) -> Style {
        Style::default().fg(self.foreground).bg(self.background)
    }
    fn line(self, kind: LineKind) -> Style {
        self.base().bg(match kind {
            LineKind::Added => self.added,
            LineKind::Removed => self.removed,
            LineKind::Conflict => self.panel,
            LineKind::Context => self.background,
        })
    }
}

#[derive(Debug)]
pub enum RenderError {
    UnknownTheme {
        name: String,
        available: Vec<String>,
    },
}
impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTheme { name, available } => write!(
                f,
                "Unknown syntax theme {name:?}. Available: {}",
                available.join(", ")
            ),
        }
    }
}
impl std::error::Error for RenderError {}

/// Cumulative renderer counters and current cache usage.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderStats {
    /// Total source lines parsed for syntax highlighting.
    pub syntax_lines: u64,
    /// Total syntax-highlighted lines reused from cache.
    pub reused_syntax_lines: u64,
    /// Number of prepared diff cache hits.
    pub prepared_hits: u64,
    /// Number of rendered layout cache hits.
    pub layout_hits: u64,
    /// Number of budgeted alignment and inline diff comparisons.
    pub detail_comparisons: u64,
    /// Estimated bytes currently retained by renderer caches.
    pub cache_bytes: usize,
}

/// Borrowed inputs and presentation options for rendering a diff.
#[derive(Clone, Copy)]
pub struct RenderSpec<'a> {
    /// Repository changes to render from.
    pub snapshot: &'a Snapshot,
    /// Indices of changes to display, in render order.
    pub selected: &'a [usize],
    /// Available terminal width in columns.
    pub width: usize,
    /// Whether to lay out old and new lines side by side.
    pub split: bool,
    /// Search query to highlight in the rendered text.
    pub query: &'a str,
    /// Initial preview row limit; zero requests a full preview.
    pub preview_rows: usize,
}

/// A preview or final document published during progressive rendering.
pub struct RenderUpdate {
    /// Latest rendered document available for display.
    pub document: Arc<Document>,
    /// Whether this is the final update for the render operation.
    pub finished: bool,
}

#[derive(Default)]
struct Visuals {
    old: SyntaxColors,
    new: SyntaxColors,
}
struct CachedLayout {
    prepared: Arc<Prepared>,
    width: usize,
    split: bool,
    query: String,
    enriched: bool,
    document: Arc<Document>,
}
impl CachedLayout {
    fn bytes(&self) -> usize {
        self.prepared.bytes()
            + self
                .document
                .rows
                .iter()
                .map(|row| {
                    size_of::<Row>()
                        + row.anchors.len() * size_of::<Anchor>()
                        + row
                            .line
                            .spans
                            .iter()
                            .map(|span| size_of::<Span<'static>>() + span.content.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// Renders diffs while caching syntax, prepared changes, and layouts.
pub struct Renderer {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
    config: Config,
    syntax: SyntaxCache,
    prepared: Cache<Arc<Prepared>>,
    layouts: Cache<CachedLayout>,
    stats: RenderStats,
}
impl Renderer {
    pub fn validate_theme(config: &Config) -> Result<(), RenderError> {
        let themes = ThemeSet::load_defaults();
        if themes.themes.contains_key(&config.theme) {
            Ok(())
        } else {
            Err(RenderError::UnknownTheme {
                name: config.theme.clone(),
                available: themes.themes.into_keys().collect(),
            })
        }
    }
    pub fn new(config: Config) -> Result<Self, RenderError> {
        Self::validate_theme(&config)?;
        Ok(Self {
            syntaxes: two_face::syntax::extra_newlines(),
            themes: ThemeSet::load_defaults(),
            config,
            syntax: SyntaxCache::new(),
            prepared: Cache::new(32 * 1024 * 1024),
            layouts: Cache::new(64 * 1024 * 1024),
            stats: RenderStats::default(),
        })
    }
    pub fn stats(&self) -> RenderStats {
        RenderStats {
            syntax_lines: self.syntax.parsed_lines,
            reused_syntax_lines: self.syntax.reused_lines,
            cache_bytes: self.syntax.bytes() + self.prepared.bytes() + self.layouts.bytes(),
            ..self.stats
        }
    }
    /// Blocking convenience API for exports, tests and benchmarks. The TUI uses
    /// render_progressive so its first readable frame precedes syntax processing.
    pub fn document(
        &mut self,
        snapshot: &Snapshot,
        selected: &[usize],
        width: usize,
        split: bool,
        query: &str,
    ) -> Result<Arc<Document>, Cancelled> {
        let mut latest = Arc::new(Document::default());
        self.render_progressive(
            RenderSpec {
                snapshot,
                selected,
                width,
                split,
                query,
                preview_rows: 0,
            },
            &Cancellation::default(),
            |update| {
                latest = update.document;
                true
            },
        )?;
        Ok(latest)
    }
    pub fn render_progressive(
        &mut self,
        spec: RenderSpec<'_>,
        cancel: &Cancellation,
        mut publish: impl FnMut(RenderUpdate) -> bool,
    ) -> Result<(), Cancelled> {
        cancel.check()?;
        let versions: Vec<_> = spec
            .selected
            .iter()
            .filter_map(|i| spec.snapshot.changes.get(*i))
            .map(|change| (fingerprint(change), change))
            .collect();
        // Revisits with unchanged presentation need no preview or syntax pass.
        if let Some(documents) = versions
            .iter()
            .map(|(hash, change)| self.cached_layout(*hash, change, spec, true))
            .collect::<Option<Vec<_>>>()
        {
            let document = compose(&versions, &documents, self.config.light, None, cancel)?;
            cancel.check()?;
            return if publish(RenderUpdate {
                document: Arc::new(document),
                finished: true,
            }) {
                cancel.check()
            } else {
                Err(Cancelled)
            };
        }
        let mut budget = WorkBudget::new(Duration::from_millis(20));
        let mut prepared = Vec::new();
        for (hash, change) in &versions {
            prepared.push(self.prepare(*hash, change, false, &mut budget, cancel)?);
        }
        if spec.preview_rows > 0 {
            let document =
                self.preview(&versions, &prepared, spec, Some(spec.preview_rows), cancel)?;
            let partial = document.partial;
            cancel.check()?;
            if !publish(RenderUpdate {
                document: Arc::new(document),
                finished: false,
            }) {
                return Err(Cancelled);
            }
            cancel.check()?;
            if partial {
                let document = self.preview(&versions, &prepared, spec, None, cancel)?;
                cancel.check()?;
                if !publish(RenderUpdate {
                    document: Arc::new(document),
                    finished: false,
                }) {
                    return Err(Cancelled);
                }
            }
        } else {
            let document = self.preview(&versions, &prepared, spec, None, cancel)?;
            cancel.check()?;
            if !publish(RenderUpdate {
                document: Arc::new(document),
                finished: false,
            }) {
                return Err(Cancelled);
            }
        }
        let mut refined = Vec::new();
        for (hash, change) in &versions {
            refined.push(self.prepare(*hash, change, true, &mut budget, cancel)?);
        }
        self.stats.detail_comparisons += budget.comparisons;
        let mut documents = Vec::new();
        for item in refined {
            cancel.check()?;
            if let Some(cached) = self.cached_layout(item.hash, &item.change, spec, true) {
                documents.push(cached);
                continue;
            }
            let change = &item.change;
            let theme = &self.themes.themes[&self.config.theme];
            let old_end = change
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter_map(|line| line.old)
                .max()
                .unwrap_or(0);
            let new_end = change
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter_map(|line| line.new)
                .max()
                .unwrap_or(0);
            let old = self.syntax.highlight(
                &self.syntaxes,
                theme,
                change.old_path.as_deref().unwrap_or(&change.path),
                change.old_source.as_ref(),
                old_end,
                cancel,
            )?;
            let new = self.syntax.highlight(
                &self.syntaxes,
                theme,
                &change.path,
                change.new_source.as_ref(),
                new_end,
                cancel,
            )?;
            let document = Arc::new(layout::file(
                &item,
                &Visuals { old, new },
                &self.config,
                spec,
                None,
                cancel,
            )?);
            self.remember_layout(item, spec, true, Arc::clone(&document));
            documents.push(document);
        }
        let document = compose(&versions, &documents, self.config.light, None, cancel)?;
        cancel.check()?;
        if publish(RenderUpdate {
            document: Arc::new(document),
            finished: true,
        }) {
            cancel.check()
        } else {
            Err(Cancelled)
        }
    }
    fn prepare(
        &mut self,
        hash: u64,
        change: &Change,
        refined: bool,
        budget: &mut WorkBudget,
        cancel: &Cancellation,
    ) -> Result<Arc<Prepared>, Cancelled> {
        cancel.check()?;
        if let Some(entry) = self.prepared.take(|entry| {
            entry.hash == hash && entry.change.as_ref() == change && (!refined || entry.refined)
        }) {
            self.stats.prepared_hits += 1;
            self.prepared.insert(Arc::clone(&entry), entry.bytes());
            return Ok(entry);
        }
        let shared = self
            .prepared
            .iter()
            .find(|entry| entry.hash == hash && entry.change.as_ref() == change)
            .map(|entry| Arc::clone(&entry.change))
            .unwrap_or_else(|| Arc::new(change.clone()));
        let prepared = Arc::new(prepare::prepare(hash, shared, refined, budget, cancel)?);
        self.prepared
            .insert(Arc::clone(&prepared), prepared.bytes());
        Ok(prepared)
    }
    fn cached_layout(
        &mut self,
        hash: u64,
        change: &Change,
        spec: RenderSpec<'_>,
        enriched: bool,
    ) -> Option<Arc<Document>> {
        let entry = self.layouts.take(|entry| {
            entry.prepared.hash == hash
                && entry.prepared.change.as_ref() == change
                && entry.width == spec.width.max(4)
                && entry.split == spec.split
                && entry.query == spec.query
                && entry.enriched == enriched
        })?;
        self.stats.layout_hits += 1;
        let document = Arc::clone(&entry.document);
        let bytes = entry.bytes();
        self.layouts.insert(entry, bytes);
        Some(document)
    }
    fn remember_layout(
        &mut self,
        prepared: Arc<Prepared>,
        spec: RenderSpec<'_>,
        enriched: bool,
        document: Arc<Document>,
    ) {
        let entry = CachedLayout {
            prepared,
            width: spec.width.max(4),
            split: spec.split,
            query: spec.query.into(),
            enriched,
            document,
        };
        let bytes = entry.bytes();
        self.layouts.insert(entry, bytes);
    }
    fn preview(
        &mut self,
        versions: &[(u64, &Change)],
        prepared: &[Arc<Prepared>],
        spec: RenderSpec<'_>,
        limit: Option<usize>,
        cancel: &Cancellation,
    ) -> Result<Document, Cancelled> {
        let mut documents = Vec::new();
        let mut rows = 0;
        for item in prepared {
            cancel.check()?;
            if limit.is_some_and(|limit| rows >= limit) {
                break;
            }
            let cached = self
                .cached_layout(item.hash, &item.change, spec, true)
                .or_else(|| self.cached_layout(item.hash, &item.change, spec, false));
            let document = if let Some(cached) = cached {
                cached
            } else {
                let document = Arc::new(layout::file(
                    item,
                    &Visuals::default(),
                    &self.config,
                    spec,
                    limit.map(|limit| limit.saturating_sub(rows)),
                    cancel,
                )?);
                if !document.partial {
                    self.remember_layout(Arc::clone(item), spec, false, Arc::clone(&document));
                }
                document
            };
            rows += document.rows.len();
            documents.push(document);
        }
        compose(versions, &documents, self.config.light, limit, cancel)
    }
}

fn compose(
    versions: &[(u64, &Change)],
    files: &[Arc<Document>],
    light: bool,
    limit: Option<usize>,
    cancel: &Cancellation,
) -> Result<Document, Cancelled> {
    let palette = Palette::new(light);
    let mut result = Document::default();
    let mut category = None;
    for ((_, change), file) in versions.iter().zip(files) {
        cancel.check()?;
        if category != Some(change.category) {
            if !result.rows.is_empty() {
                layout::push_header(&mut result, "", palette.base());
            }
            layout::push_header(
                &mut result,
                &format!(" {} ", change.category.label().to_uppercase()),
                palette
                    .base()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
            category = Some(change.category);
        }
        let offset = result.rows.len();
        let count = limit.map_or(file.rows.len(), |limit| {
            limit.saturating_sub(offset).min(file.rows.len())
        });
        result.rows.extend(file.rows.iter().take(count).cloned());
        result.hits.extend(
            file.hits
                .iter()
                .filter(|hit| hit.row < count)
                .map(|hit| Hit {
                    row: offset + hit.row,
                }),
        );
        result.excluded += file.excluded;
        result.partial |= file.partial || count < file.rows.len();
        if limit.is_some_and(|limit| result.rows.len() >= limit) {
            result.partial = true;
            break;
        }
    }
    result.partial |= files.len() < versions.len();
    if result.rows.is_empty() {
        layout::push_header(
            &mut result,
            " No changes in this view",
            palette.base().fg(palette.muted),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
