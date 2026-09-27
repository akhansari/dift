# Dift

A persistent window into your working tree.

![Dift Screenshot](./docs/assets/snapshot.png)

## Highlights

- Track worktree changes live as you work.
- See staged, unstaged, untracked, and conflicted changes in one view.
- Review commit-to-commit and merge-base differences.
- Browse current-branch history, inspect commits and complete historical file trees, and compare selected commits.
- Read diffs in unified or side-by-side views with syntax highlighting.
- Search diffs and quickly move between matches and files.
- Navigate changes through a hierarchical file tree with Git status indicators.
- Stage, unstage, and safely discard changes.
- Jump directly from a diff to the changed location in your editor.
- Stay responsive when working with large diffs and files.

### Non-goals

Dift is intentionally focused on keeping current Git changes visible and easy to
review. It is not intended to become a full-featured Git client or replace tools
like [LazyGit](https://github.com/jesseduffield/lazygit), which already does that job pretty well.

## Install

Install from [crates.io](https://crates.io/crates/dift):

```sh
cargo install --locked dift
```

Install a prebuilt release for Linux, Windows, or macOS:

```sh
curl -fsSL https://gitlab.com/akhansari/dift/-/raw/main/install.sh | sh
```

Dift is compatible with Git 2.47 and newer.

## Usage

Run without a subcommand to watch worktree changes live.

Press `t` to open the current branch's history from any view. History loads on demand and remains pinned to the branch tip until you press `r` to refresh. `Enter` opens a commit; `v` switches between its first-parent patch and its complete file tree. On the history list, `b` marks a base and `m` compares it with the highlighted target. The comparison is a direct **base → target** snapshot diff; `x` swaps the endpoints. `/` searches the entire pinned history by hash, message or author. `Esc` returns to the list and `t` returns to your original worktree or CLI comparison view. Press `i` to read the full commit message and metadata.

Historical files opened with `o` or `O` are **temporary copies**. Edits to them are discarded when the editor exits and never modify the working tree. Large files must be explicitly loaded with `Enter`; binary files, symlinks and submodules display summaries rather than following their contents.

All history shortcuts can be remapped in the `keybindings` section of `--default-config`: `history` (`t`) is under `global`; `mark-base` (`b`), `compare-commits` (`m`), `toggle-commit-files` (`v`), `swap-commits` (`x`), and `commit-info` (`i`) are under `history`. The `?` help panel and history footers show the active bindings.

For names used across the UI and documentation, see the [ubiquitous language](docs/ubiquitous-language.md).

```sh
dift [--config <CONFIG>] [--default-config] [SUBCOMMAND]

Commands:
  diff    Compare two commits or branch tips directly.
  review  Compare the merge base of BASE and HEAD with HEAD.
  help    Print this message or the help of the given subcommand(s)

Flags:
      --config <CONFIG>  Read a specific KDL configuration file.
      --default-config   Print the complete default KDL configuration and exit.
  -h, --help             Print help
  -V, --version          Print version
```

Configuration path:

- `$XDG_CONFIG_HOME`, when set.
- `$HOME/.config` on Unix-like systems.
- `%APPDATA%` or `%USERPROFILE%\AppData\Roaming` on Windows.
