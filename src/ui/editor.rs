use super::terminal::{self, TerminalError};
use crate::{git::Repository, state::EditorTarget};
use ratatui::DefaultTerminal;
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum EditorError {
    #[error("Select a file to open in the editor")]
    NoActiveFile,
    #[error("Set VISUAL or EDITOR to open files")]
    NotConfigured,
    #[error("Cannot open {}: {source}", crate::model::display_path(path))]
    Inspect { path: PathBuf, source: io::Error },
    #[error("Cannot open {}: not a file", crate::model::display_path(path))]
    NotFile { path: PathBuf },
    #[error("Cannot start ${variable}: {source}")]
    Spawn {
        variable: &'static str,
        source: io::Error,
    },
    #[error("Cannot wait for ${variable}: {source}")]
    Wait {
        variable: &'static str,
        source: io::Error,
    },
    #[error("${variable} exited unsuccessfully ({status})")]
    Exit {
        variable: &'static str,
        status: ExitStatus,
    },
    #[error("Cannot prepare historical editor copy: {0}")]
    Temporary(#[from] io::Error),
    #[error("Cannot read historical file: {0}")]
    Historical(#[from] crate::git::GitError),
}

/// A validated target and editor command, prepared before handing over the terminal.
pub(super) struct EditorCommand {
    command: Command,
    variable: &'static str,
}

impl EditorCommand {
    pub(super) fn prepare(
        root: &Path,
        target: Option<EditorTarget<'_>>,
        visual: Option<OsString>,
        editor: Option<OsString>,
    ) -> Result<Self, EditorError> {
        Self::prepare_with_wait(root, target, visual, editor, false)
    }

    fn prepare_with_wait(
        root: &Path,
        target: Option<EditorTarget<'_>>,
        visual: Option<OsString>,
        editor: Option<OsString>,
        wait: bool,
    ) -> Result<Self, EditorError> {
        let target = target.ok_or(EditorError::NoActiveFile)?;
        let path = root.join(target.path);
        let metadata = fs::metadata(&path).map_err(|source| EditorError::Inspect {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(EditorError::NotFile { path });
        }
        let (variable, value) = [("VISUAL", visual), ("EDITOR", editor)]
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| (name, value)))
            .find(|(_, value)| !value.as_encoded_bytes().trim_ascii().is_empty())
            .ok_or(EditorError::NotConfigured)?;
        let prepared =
            literal_editor_command(&value, &path, target.line, wait).unwrap_or_else(|| {
                gix::command::prepare(value)
                    .command_may_be_shell_script()
                    .arg(path)
            });
        let mut command = Command::from(
            prepared
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit()),
        );
        command.current_dir(root);
        // Git's command helper suppresses consoles by default; interactive editors need ours.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0);
        }
        Ok(Self { command, variable })
    }

    fn run(mut self) -> Result<(), EditorError> {
        let variable = self.variable;
        let mut child = self
            .command
            .spawn()
            .map_err(|source| EditorError::Spawn { variable, source })?;
        let status = child
            .wait()
            .map_err(|source| EditorError::Wait { variable, source })?;
        if status.success() {
            Ok(())
        } else {
            Err(EditorError::Exit { variable, status })
        }
    }

    /// Always resume the terminal before reporting a child-process error.
    pub(super) fn open(
        self,
        terminal: &mut DefaultTerminal,
    ) -> Result<Result<(), EditorError>, TerminalError> {
        terminal::with_suspended(terminal, || Ok(self.run()))
    }
}

/// Only rewrite literal invocations of known editors; preserve shell commands verbatim.
fn literal_editor_command(
    value: &OsStr,
    path: &Path,
    line: Option<usize>,
    wait: bool,
) -> Option<gix::command::Prepare> {
    let bytes = gix::path::os_str_into_bstr(value).ok()?;
    if !literal_command(bytes.as_ref()) {
        return None;
    }
    let mut parsed = gix::command::parse::command_line(bytes).ok()?;
    let name = Path::new(&parsed.command)
        .file_name()?
        .to_str()?
        .to_ascii_lowercase();
    let name = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".cmd"))
        .or_else(|| name.strip_suffix(".bat"))
        .unwrap_or(&name);
    let vscode = matches!(name, "code" | "code-insiders");
    let position = match name {
        "vi" | "vim" | "nvim" | "hx" | "helix" => line.map(|line| format!("+{}", line.max(1))),
        "code" | "code-insiders" => line.map(|_| "--goto".into()),
        _ => return None,
    };
    if position.is_none() && !(wait && vscode) {
        return None;
    }
    let mut file = path.as_os_str().to_os_string();
    if vscode && let Some(line) = line {
        file.push(format!(":{}:1", line.max(1)));
    }
    let index = parsed
        .args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(parsed.args.len());
    let options = position.into_iter().chain(
        (wait && vscode && !parsed.args[..index].iter().any(|arg| arg == "--wait"))
            .then(|| "--wait".into()),
    );
    parsed.args.splice(index..index, options.map(Into::into));
    parsed.args.push(file);
    Some(parsed.env.into_iter().fold(
        gix::command::prepare(parsed.command).args(parsed.args),
        |command, (key, value)| command.env(key, value),
    ))
}

/// The temporary copy is owned until the editor exits; it is never written to the worktree.
pub(super) fn open_historical(
    terminal: &mut DefaultTerminal,
    repo: &Repository,
    revision: &str,
    target: EditorTarget<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let copy = HistoricalCopy::new(repo, revision, target.path)?;
    let command = EditorCommand::prepare_with_wait(
        copy.directory.path(),
        Some(EditorTarget {
            path: Path::new(&copy.filename),
            line: target.line,
        }),
        std::env::var_os("VISUAL"),
        std::env::var_os("EDITOR"),
        true,
    )?;
    let result = command.open(terminal)?;
    // Dropping the copy removes any edits when the child has finished.
    result?;
    Ok(())
}

struct HistoricalCopy {
    directory: tempfile::TempDir,
    filename: OsString,
}

impl HistoricalCopy {
    fn new(repo: &Repository, revision: &str, path: &Path) -> Result<Self, EditorError> {
        let bytes = repo
            .historical_blob(revision, path)?
            .ok_or(EditorError::NoActiveFile)?;
        let directory = tempfile::Builder::new()
            .prefix(&format!("dift-{}-", &revision[..revision.len().min(12)]))
            .tempdir()?;
        let filename = path
            .file_name()
            .ok_or(EditorError::NoActiveFile)?
            .to_os_string();
        fs::write(directory.path().join(&filename), bytes)?;
        Ok(Self {
            directory,
            filename,
        })
    }
}

/// Reject expansion and control syntax outside literal quoting, before shell-word parsing.
fn literal_command(bytes: &[u8]) -> bool {
    let mut quote = None;
    let mut bytes = bytes.iter().copied();
    while let Some(byte) = bytes.next() {
        match (quote, byte) {
            (Some(b'\''), b'\'') => quote = None,
            (Some(b'\''), _) => {}
            (_, b'\\') => {
                if bytes.next().is_none_or(|next| next == b'\n') {
                    return false;
                }
            }
            (Some(b'"'), b'"') => quote = None,
            (Some(b'"'), b'$' | b'`') => return false,
            (Some(b'"'), _) => {}
            (None, b'\'' | b'"') => quote = Some(byte),
            (
                None,
                b'|' | b'&' | b';' | b'<' | b'>' | b'(' | b')' | b'$' | b'`' | b'\n' | b'*' | b'?'
                | b'[' | b']' | b'{' | b'}' | b'~' | b'#',
            ) => return false,
            _ => {}
        }
    }
    quote.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppResult, test_support::Fixture};

    #[cfg(unix)]
    #[path = "command_tests.rs"]
    mod command_tests;

    #[test]
    fn no_active_file_returns_a_selection_hint() {
        // Given
        let root = Path::new("");
        // When
        let result = EditorCommand::prepare(root, None, None, None);
        // Then
        assert!(matches!(result, Err(EditorError::NoActiveFile)));
    }

    #[test]
    fn blank_editor_variables_return_a_setup_hint() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        // When
        let result = EditorCommand::prepare(
            &fixture.root,
            Some(EditorTarget {
                path: Path::new("tracked.txt"),
                line: None,
            }),
            Some(" \t".into()),
            Some("".into()),
        );
        // Then
        assert!(matches!(result, Err(EditorError::NotConfigured)));
        Ok(())
    }

    #[test]
    fn missing_working_tree_files_cannot_be_opened() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        // When
        let result = EditorCommand::prepare(
            &fixture.root,
            Some(EditorTarget {
                path: Path::new("missing.txt"),
                line: None,
            }),
            Some("editor".into()),
            None,
        );
        // Then
        assert!(
            matches!(result, Err(EditorError::Inspect { source, .. }) if source.kind() == io::ErrorKind::NotFound)
        );
        Ok(())
    }

    #[test]
    fn a_directory_at_a_changed_path_cannot_be_opened() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        // When
        let result = EditorCommand::prepare(
            &fixture.root,
            Some(EditorTarget {
                path: Path::new("target"),
                line: None,
            }),
            Some("editor".into()),
            None,
        );
        // Then
        assert!(matches!(result, Err(EditorError::NotFile { .. })));
        Ok(())
    }

    #[test]
    fn file_error_notices_escape_control_characters() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        // When
        let result = EditorCommand::prepare(
            &fixture.root,
            Some(EditorTarget {
                path: Path::new("missing\nfile"),
                line: None,
            }),
            Some("editor".into()),
            None,
        );
        // Then
        let error = result.err().ok_or("Missing file should report an error")?;
        assert!(error.to_string().contains("missing\\nfile"));
        assert!(!error.to_string().contains('\n'));
        Ok(())
    }

    #[test]
    fn historical_editor_copy_is_disposable_and_does_not_touch_the_worktree() -> AppResult<()> {
        // Given: a committed file and a modified working-tree version.
        let fixture = Fixture::new("")?;
        let repo = fixture.repo()?;
        let commit = repo.resolve("HEAD")?;
        fixture.write("tracked.txt", "working tree\n")?;
        let copy = HistoricalCopy::new(&repo, &commit, Path::new("tracked.txt"))?;
        let path = copy.directory.path().join("tracked.txt");
        assert_eq!(fs::read_to_string(&path)?, "original\n");
        // When: editing and dropping the temporary copy.
        fs::write(&path, "edited\n")?;
        drop(copy);
        // Then: only the temporary path is removed.
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(fixture.root.join("tracked.txt"))?,
            "working tree\n"
        );
        Ok(())
    }
}
