# Startup performance

dift publishes the initial Git snapshot before walking the directories that need
filesystem watches. Worktree watches are nonrecursive: ignored directories are
pruned unless they contain tracked paths or Git configuration resources.
Directory symlinks are not followed. Submodule coverage remains conservative.
Git metadata and external configuration parents are watched explicitly.

When coverage expands, another collection catches edits made before the watches
were installed. Directory changes, ignore and index changes, queue overflow, and
periodic reconciliation update coverage. Watch errors retain polling recovery.

The renderer publishes a plain preview first, followed by a complete searchable
preview. As individual files finish syntax highlighting, their colored documents
replace the plain versions. Intermediate updates include every file and retain
source anchors for scrolling. The first colored file is published immediately;
subsequent intermediate publications are limited to one per 33 ms. The final
publication is always delivered unless the request has been superseded.

## Measuring the actual terminal

Run the opt-in Rust terminal benchmark on Unix (Cargo builds the release executable):

```sh
cargo test --release --locked --test perf -- --repo /path/to/repository --runs 5
```

Use `--binary /path/to/dift` to compare another build. The benchmark opens the
repository read-only using a 120×35 pseudo-terminal, built-in settings, and
enabled colors, including when its parent environment sets `NO_COLOR`.
Each run is a fresh process; filesystem caches are not cleared. Setup of the
temporary configuration is excluded, while process startup, watcher setup,
collection, rendering, terminal output, and UI polling are included.

JSON output reports individual runs and medians:

- `first_frame_ms`: the first visible application frame.
- `readable_diff_ms`: the first visible source row with a diff line number.
- `highlighted_ms`: the first visible source row with distinct foreground colors.
- `complete_ms`: the first visible `Watching` or `Polling` activity label. Activity
  changes must persist for 500 ms before appearing, so this measurement includes
  the label delay after rendering completes.
- `native_watches`: Linux inotify registrations at completion; null elsewhere.

Color detection applies to the fixed default layout and measures visible syntax,
not an internal render-worker timestamp. It reports null if the initial viewport
contains no multicolored source row, such as a plain-text diff. An empty or
summary-only view can also have a null readable-diff measurement. No timing
thresholds are enforced in unit tests.

## Local comparison

Five-run medians on Linux in `yeswerise/monorepo`, with 38 changed paths:

| Measurement | Before | After |
| --- | ---: | ---: |
| First frame | 9 ms | 9 ms |
| Readable diff | 1,136 ms | 76 ms |
| Visible syntax colors | 3,520 ms | 144 ms |
| Complete highlighting | 3,521 ms | 2,500 ms |
| Native watches | 9,328 | 118 |

These measurements use the same color-enabled benchmark and warmed filesystem
caches. They describe this checkout and machine, not a universal latency bound.
The optimization makes the opening view useful sooner; highlighting all files
still consumes CPU in the background.
