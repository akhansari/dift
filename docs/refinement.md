# Diff alignment and inline emphasis

Git generates patches and determines which lines changed. The renderer uses `similar` 3.2.0 to align those lines for split display and emphasize changes within lines in both layouts. Source text, line numbers, search anchors, and unified patch ordering are preserved.

## Whitespace comparison

`diff { whitespace "exact"; }` is the startup default. `ignore-changes` selects Git's `--ignore-space-change`; `ignore-all` selects `--ignore-all-space`. Filtering happens during collection, before preview or refinement, and applies to unstaged, staged, diff, and review comparisons. Remaining lines use the same alignment and emphasis engine described below. Similar's `WhitespaceMode` is not involved in Git patch generation.

The `toggle-whitespace` action (`w`) switches between exact comparison and the configured ignore mode. When startup configuration is exact, the alternate mode is ignore-changes. Configuration files are never written by this action. Collection requests carry the session mode, explicitly loaded paths, and a generation identifier. Superseded requests are coalesced, stale results are rejected, and collection failures retain the last successful snapshot. There is no time-based fallback to a different whitespace policy.

Ignore modes add one exact names-only Git query per tracked category. Hidden counts are the distinct exact paths absent from all remaining categories. Untracked files and conflicts retain their existing collection paths; metadata and special-file patches follow Git's handling. File loading limits apply after Git has generated the filtered patches, so whitespace-only files can be hidden without reading their full sources into dift.

The status bar always identifies the applied mode and shows visible and whitespace-hidden distinct-path counts, independently of interactive filtering and tree collapse. Pending mode changes are labelled separately. In split view, context lines use the original source for each side when available, since whitespace-equivalent lines may contain different bytes. Unified context retains Git's text. Exact inline refinement remains enabled for lines with substantive edits; whitespace filtering is not language-aware.

Local Linux x86-64 release measurements on 2026-09-06, using `collect_bench` with 300 changed files, 5,000 ignored build files, and ten warm collections per mode:

| Corpus | Exact | Ignore changes | Ignore all |
| --- | ---: | ---: | ---: |
| Content edits, 300 visible files | 15.326 ms | 27.935 ms | 24.447 ms |
| Spacing edits, 300 files hidden in ignore modes | 11.840 ms | 15.323 ms | 14.975 ms |

Values are median collection times, excluding fixture setup and terminal rendering. They include both Git's selected comparison and the extra names-only queries, so they do not isolate query overhead. Runs used separate fixtures and sequential mode order; these are illustrative local measurements, not latency guarantees. Exact mode makes no extra counting query.

## Alignment

Each contiguous changed block is processed in two stages:

1. Compare the removed and added line sequences using trimmed text as keys. These matches anchor unchanged or reindented lines throughout the block. Trimming is only for matching; the original whitespace remains visible and participates in inline emphasis.
2. Between anchors, find the order-preserving pairing with the greatest total character similarity. Only pairs with similarity at least 0.65 qualify. Ties prefer pairing, then removal, then insertion. Unmatched lines have an empty opposite cell.

For example, inserting two comments before an edited assignment keeps that assignment aligned with its old version. The comments appear as additions above it. Matching is local to a changed block and never crosses a context line or hunk boundary.

## Emphasis configuration

The startup setting `appearance { emphasis "word"; }` accepts three values:

| Value | Behavior |
| --- | --- |
| `word` (default) | `InlineChangeMode::UnicodeWords`; changed identifiers remain whole words. |
| `grapheme` | `InlineChangeMode::Graphemes`; highlight changed user-perceived characters, keeping combining marks and emoji sequences together. |
| `off` | Skip inline refinement; keep ordinary added/removed line colors, alignment, syntax, and search highlighting. |

Both enabled modes use minimum similarity 0.25 and semantic cleanup. Cleanup shifts highlight boundaries and extracts overlaps for readable regions, so grapheme mode does not guarantee the smallest possible highlighted substring. Highlight ranges reference the original UTF-8 bytes; adjacent ranges are merged, and empty ranges are not retained.

For example, changing `1000` to `1001` emphasizes the whole number in word mode and the final digit in grapheme mode. A change from `e` plus a combining acute accent to `e` plus a combining grave accent emphasizes the complete character in grapheme mode. This setting controls emphasis boundaries; terminal wrapping and character-width handling are separate concerns.

Omitting the setting preserves existing behavior. Unknown values and duplicate settings are rejected, and `--default-config` lists the choices. Configuration is immutable for each renderer instance, whose caches are private to that instance; restarting applies a different mode. Algorithms and thresholds remain internal.

## Work limits

A basic preview appears before refinement. All selected files share a 20 ms refinement allowance:

- Alignment gets at most 3 ms per block and starts only with time remaining above a 10 ms reserve for inline emphasis. This rule also applies with emphasis disabled.
- Block matching is limited to 64 KiB of combined line text, counting one terminator byte per line so empty lines also consume the limit.
- Fuzzy alignment is limited to 4,096 candidate pairs per replacement region and 2 KiB per candidate line. Each character comparison gets at most 1 ms within the block deadline.
- Both enabled emphasis modes get at most 3 ms per pair and are skipped above 4 KiB per line. Diff construction and inline refinement use the same deadline, with the entire operation charged to the shared allowance. `off` skips these operations entirely.

Deadlines are cooperative. Tokenization, cleanup, and mapping can finish after a deadline, so these are work allowances rather than strict end-to-end latency guarantees. Size limits bound those operations, and cancellation is checked between operations and during mapping.

When an alignment limit is reached, completed anchors remain and unresolved regions use positional pairing. Incomplete fuzzy score matrices are discarded. Exhausted or oversized emphasis falls back to ordinary added/removed styling. Both sides retain all their source lines.

## Algorithm comparison

Run:

```sh
cargo run --release --locked --example refine_bench
```

The example compiles the internal refinement engine directly, without exposing algorithm selection in the library's public API. It checks expected pairings for insertions, deletions, repeated lines, indentation, ties, unrelated lines, and empty inputs. Algorithm selection uses word-mode fixtures covering identifiers, semantic boundaries, Unicode, whitespace, punctuation, and disjoint or empty inputs.

Timing includes those fixtures plus sparse and dense 32-line source blocks and a repetitive block. For each fixture, the benchmark warms every candidate and records five batches of 20 operations. Candidate order rotates each round. The reported score is the sum of per-fixture median times; it is not the latency of one user operation. Block timings measure key-based diff capture, similarity timings compare candidate line pairs, and inline timings use the production emphasis function. Syntax and layout are excluded.

Only candidates passing the behavioral fixtures qualify. The lowest score wins; within 10% of it, the preference is Histogram for block matching, Myers for similarity, and Patience for inline refinement. Algorithms are fixed internal choices, not selected at runtime.

Local Linux x86-64 release measurements with Rust 1.98.1, on 2026-09-06 (microseconds):

| Algorithm | Block matching | Character similarity | Inline refinement |
| --- | ---: | ---: | ---: |
| Myers | 13.554 | 2345.857 | 8.354 |
| RawMyers | 9.333 | 2170.161 | 8.046 |
| Patience | 40.243 | 9498.720 | 13.830 |
| Histogram | 10.322 | 6717.789 | 8.941 |
| Hunt | 13.113 | 6252.350 | 9.117 |

All five candidates passed. The selected algorithms are **RawMyers for block matching, Myers for character similarity, and RawMyers for inline refinement**. Myers similarity was within 10% of the fastest result. RawMyers benefits from the same application deadlines and input limits; its unrestricted worst-case cost is not accepted as an unbounded renderer operation.

Complete alignment of the small insertion/deletion fixtures took about 7 µs; the dense 32-line fixture took about 1.22 ms. Results describe this corpus and machine. Near-threshold choices can vary between runs, and should be reevaluated with new representative fixtures when workloads change.

## Emphasis mode comparison

The same benchmark also checks expected grapheme highlights for digit edits, identifier suffixes, combining marks, emoji sequences, semantic phrases, and whitespace. It checks that `off` produces no emphasized ranges. It then compares all three modes on identical inputs with the fixed production RawMyers algorithm, five warmed batches of 20 operations, and rotating mode order. These measurements compare modes without choosing a new algorithm or default.

Selected per-operation medians from the local Linux x86-64 release run on 2026-09-06 with Rust 1.98.1, in microseconds:

| Input | Word | Grapheme | Off |
| --- | ---: | ---: | ---: |
| Identifier replacement | 1.710 | 3.269 | 0.039 |
| Digit edit | 1.541 | 1.900 | 0.038 |
| Identifier suffix | 1.658 | 1.958 | 0.039 |
| Combining mark | 1.455 | 1.648 | 0.038 |
| Emoji sequence | 1.669 | 1.769 | 0.038 |

All mode fixtures passed. Grapheme mode usually took slightly longer on these small inputs; both enabled modes were well within the 3 ms allowance. This does not establish worst-case performance or which presentation users prefer. `off` measures only the engine's early return; the renderer skips that call entirely. The original algorithm selection and renderer comparison used word mode only.
