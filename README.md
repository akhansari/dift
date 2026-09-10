# Dift

A persistent window into your working tree.

![Dift Screenshot](./docs/assets/snapshot.png)

## Highlights

- Track worktree changes live as you work.
- See staged, unstaged, untracked, and conflicted changes in one view.
- Review commit-to-commit and merge-base differences.
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
