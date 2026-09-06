//! In-memory tree measurements, excluding Git collection and terminal output.
use dift::{
    AppResult,
    input::Action,
    model::{Category, Change, Snapshot},
    state::State,
};
use std::{hint::black_box, time::Instant};

fn fixture(files: usize, nested: bool) -> Snapshot {
    Snapshot {
        branch: "benchmark".into(),
        changes: (0..files)
            .map(|i| Change {
                path: if nested {
                    format!(
                        "group-{:05}/src/module-{:05}/file-{i:05}.rs",
                        i / 100,
                        i / 10
                    )
                } else {
                    format!("file-{i:05}.rs")
                }
                .into(),
                old_path: None,
                category: Category::Unstaged,
                status: 'M',
                hunks: Vec::new(),
                summary: None,
                oversized: false,
                old_source: None,
                new_source: None,
            })
            .collect(),
    }
}

fn measure(files: usize, nested: bool, repeats: usize) {
    let snapshot = fixture(files, nested);
    let mut state = State::new(false);
    let start = Instant::now();
    state.update_snapshot(snapshot);
    black_box(state.tree(false));
    let build = start.elapsed();

    let start = Instant::now();
    for _ in 0..repeats {
        black_box(state.action(Action::NextFile, 30, Instant::now()));
        black_box(state.tree(false));
        black_box(state.matching_paths().len());
    }
    let navigation = start.elapsed();

    state.action(Action::Filter, 30, Instant::now());
    let start = Instant::now();
    black_box(state.paste("file-000", start));
    // Apply immediately so this measures filtering rather than the debounce wait.
    black_box(state.action(Action::Accept, 30, start));
    black_box(state.tree(false));
    let filter = start.elapsed();
    println!(
        "{},{files},{repeats},{:.3},{:.3},{:.3}",
        if nested { "nested" } else { "wide" },
        build.as_secs_f64() * 1000.0,
        navigation.as_secs_f64() * 1000.0 / repeats as f64,
        filter.as_secs_f64() * 1000.0,
    );
}

fn main() -> AppResult<()> {
    let mut args = std::env::args().skip(1);
    let files = args.next().map(|s| s.parse()).transpose()?.unwrap_or(1000);
    let repeats = args.next().map(|s| s.parse()).transpose()?.unwrap_or(10);
    if files == 0 || repeats == 0 {
        return Err("File and repeat counts must be positive".into());
    }
    println!("shape,files,repeats,build_ms,navigation_and_redraw_ms,filter_ms");
    measure(files, false, repeats);
    measure(files, true, repeats);
    Ok(())
}
