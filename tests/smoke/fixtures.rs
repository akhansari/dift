use crate::terminal::{Result, Session, SessionOptions};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

pub struct Fixture {
    _directory: TempDir,
    root: PathBuf,
}

impl Fixture {
    pub fn new() -> Result<Self> {
        let directory = tempfile::Builder::new().prefix("dift-smoke-").tempdir()?;
        let fixture = Self {
            root: directory.path().canonicalize()?,
            _directory: directory,
        };
        fixture.git(&["init", "--initial-branch=main"])?;
        fixture.write("src/main.rs", "fn main() { println!(\"original\"); }\n")?;
        fixture.git(&["add", "."])?;
        fixture.git(&["commit", "-m", "initial"])?;
        fixture.write("src/main.rs", "fn main() { println!(\"staged\"); }\n")?;
        fixture.git(&["add", "."])?;
        fixture.write("src/main.rs", "fn main() { println!(\"working\"); }\n")?;
        fixture.write("notes.txt", "untracked note\n")?;
        fixture.configure("")?;
        Ok(fixture)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self, path: &str) -> PathBuf {
        self.root().join(path)
    }
    pub fn config(&self) -> PathBuf {
        self.path(".git/terminal-config.kdl")
    }

    pub fn write(&self, path: &str, text: impl AsRef<[u8]>) -> Result {
        let path = self.path(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(fs::write(path, text)?)
    }

    pub fn configure(&self, content: &str) -> Result {
        // Native-watch tests cannot pass through the periodic fallback.
        Ok(fs::write(
            self.config(),
            format!("watch {{ reconcile-seconds 3600; }}\n{content}"),
        )?)
    }

    pub fn git(&self, args: &[&str]) -> Result<String> {
        git(self.root(), args)
    }

    pub fn spawn(
        &self,
        cwd: &Path,
        args: &[&str],
        env: &[(&str, Option<&str>)],
    ) -> Result<Session> {
        let variables: Vec<_> = [
            ("GIT_CONFIG_NOSYSTEM", Some("1")),
            ("GIT_CONFIG_GLOBAL", Some("/dev/null")),
        ]
        .into_iter()
        .chain(env.iter().copied())
        .collect();
        Session::start(SessionOptions {
            binary: Path::new(env!("CARGO_BIN_EXE_dift")),
            cwd,
            config: &self.config(),
            args,
            env: &variables,
        })
    }

    pub fn start(&self) -> Result<Session> {
        self.start_with(&self.path("src"), &[], &[])
    }

    pub fn start_with(
        &self,
        cwd: &Path,
        args: &[&str],
        env: &[(&str, Option<&str>)],
    ) -> Result<Session> {
        let mut session = self.spawn(cwd, args, env)?;
        session.wait(if args.is_empty() {
            "Watching"
        } else {
            "Snapshot"
        })?;
        Ok(session)
    }

    pub fn spacing() -> Result<Self> {
        let fixture = Self::new()?;
        fixture.write("spacing.txt", "one two\n")?;
        fixture.git(&["add", "."])?;
        fixture.git(&["commit", "-m", "spacing fixture"])?;
        fixture.write("spacing.txt", "one  two\n")?;
        Ok(fixture)
    }

    pub fn syntax_file(&self, path: &str, lines: usize) -> Result<String> {
        let source: String = (0..lines)
            .map(|i| format!("export const item_{i}: Record<string, number> = {{ count: {i} }};\n"))
            .collect();
        self.write(path, format!("{source}export const last = \"INITIAL\";\n"))?;
        self.git(&["add", path])?;
        self.git(&["commit", "-m", "syntax fixture"])?;
        Ok(source)
    }

    pub fn external_ignore(&self) -> Result<(TempDir, PathBuf)> {
        let directory = tempfile::tempdir()?;
        let ignore = directory.path().join("ignore");
        fs::write(&ignore, "external.txt\n")?;
        self.git(&[
            "config",
            "core.excludesFile",
            ignore.to_str().ok_or("Non-UTF-8 fixture path")?,
        ])?;
        self.write("external.txt", "EXTERNAL_CHANGE\n")?;
        Ok((directory, ignore))
    }
}

pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "user.name=Terminal Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.hooksPath=/nonexistent-dift-hooks",
        ])
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!("git {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
