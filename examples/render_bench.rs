//! Repeatable, in-memory renderer measurements. No repository files are read or changed.
use dift::{
    AppResult,
    config::Config,
    model::{Category, Change, Hunk, LineKind, Snapshot, SourceLine},
    render::{Cancellation, RenderSpec, Renderer},
};
use std::{sync::Arc, time::Instant};

fn fixture(lines: usize, last: &str, dense: bool) -> Snapshot {
    let before: Vec<String> = (0..lines)
        .map(|i| format!("export const item_{i}: Record<string, number> = {{ count: {i} }};"))
        .collect();
    let after: Vec<String> = (0..lines)
        .map(|i| {
            if i + 1 == lines {
                format!("export const item_{i}: Record<string, number> = {{ count: {last} }};")
            } else if dense {
                format!(
                    "export const item_{i}: Record<string, number> = {{ count: {} }};",
                    i + 1
                )
            } else {
                before[i].clone()
            }
        })
        .collect();
    let start = if dense { 0 } else { lines - 1 };
    let changes = before
        .iter()
        .enumerate()
        .skip(start)
        .map(|(i, text)| SourceLine {
            kind: LineKind::Removed,
            old: Some(i + 1),
            new: None,
            text: text.clone(),
            no_newline: false,
        })
        .chain(
            after
                .iter()
                .enumerate()
                .skip(start)
                .map(|(i, text)| SourceLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(i + 1),
                    text: text.clone(),
                    no_newline: false,
                }),
        )
        .collect();
    Snapshot {
        branch: "benchmark".into(),
        changes: vec![Change {
            path: "large.ts".into(),
            old_path: None,
            category: Category::Unstaged,
            status: 'M',
            hunks: vec![Hunk {
                header: "@@ benchmark @@".into(),
                lines: changes,
            }],
            summary: None,
            oversized: false,
            old_source: Some(Arc::from(before.join("\n") + "\n")),
            new_source: Some(Arc::from(after.join("\n") + "\n")),
        }],
    }
}

fn measure(
    renderer: &mut Renderer,
    snapshot: &Snapshot,
    label: &str,
    width: usize,
    split: bool,
    query: &str,
) -> AppResult<()> {
    let before = renderer.stats();
    let start = Instant::now();
    let mut first = None;
    let mut total_rows = 0;
    renderer.render_progressive(
        RenderSpec {
            snapshot,
            selected: &[0],
            width,
            split,
            query,
            preview_rows: 64,
        },
        &Cancellation::default(),
        |update| {
            first.get_or_insert_with(|| start.elapsed());
            total_rows = update.document.rows.len();
            true
        },
    )?;
    let complete = start.elapsed();
    let stats = renderer.stats();
    println!(
        "{label}: first={:.2}ms complete={:.2}ms parsed={} reused={} rows={total_rows} cache={:.2}MiB",
        first.unwrap_or_default().as_secs_f64() * 1000.0,
        complete.as_secs_f64() * 1000.0,
        stats.syntax_lines - before.syntax_lines,
        stats.reused_syntax_lines - before.reused_syntax_lines,
        stats.cache_bytes as f64 / (1024.0 * 1024.0),
    );
    Ok(())
}

fn main() -> AppResult<()> {
    let lines = std::env::args()
        .nth(1)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(10000);
    if lines == 0 {
        return Err("Line count must be positive".into());
    }
    let dense = std::env::args().nth(2).as_deref() == Some("dense");
    let original = fixture(lines, "12345", dense);
    let edited = fixture(lines, "67890", dense);
    let mut renderer = Renderer::new(Config::default())?;
    measure(&mut renderer, &original, "cold", 100, false, "")?;
    measure(&mut renderer, &original, "revisit", 100, false, "")?;
    let mut other = fixture(lines, "22222", dense);
    other.changes[0].path = "other.ts".into();
    measure(&mut renderer, &other, "second file", 100, false, "")?;
    measure(&mut renderer, &original, "return to first", 100, false, "")?;
    measure(
        &mut renderer,
        &original,
        "resize and search",
        80,
        true,
        "count",
    )?;
    measure(&mut renderer, &edited, "tail edit", 100, false, "")?;
    Ok(())
}
