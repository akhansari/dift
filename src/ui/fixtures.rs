use crate::{
    AppResult,
    git::{Collector, Limits, Mode, Repository},
};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Fixture {
    pub root: PathBuf,
}
impl Fixture {
    pub fn new(ignore: &str) -> AppResult<Self> {
        let root = std::env::temp_dir().join(format!(
            "dift-watch-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        let fixture = Self {
            root: gix::path::realpath(root)?,
        };
        fixture.git(&["init", "--initial-branch=main"])?;
        fixture.write(".gitignore", ignore)?;
        fixture.write("tracked.txt", "original\n")?;
        fixture.git(&["add", "-f", ".gitignore", "tracked.txt"])?;
        fixture.git(&["commit", "-m", "initial"])?;
        fs::create_dir(fixture.root.join("target"))?;
        Ok(fixture)
    }

    pub fn write(&self, path: &str, text: &str) -> AppResult<()> {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
        Ok(())
    }

    pub fn git(&self, args: &[&str]) -> AppResult<()> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args([
                "-c",
                "user.name=Watch Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/nonexistent-dift-hooks",
            ])
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        Ok(())
    }

    pub fn repo(&self) -> AppResult<Repository> {
        Ok(Repository::discover(&self.root)?)
    }

    pub fn collector(&self) -> AppResult<Collector> {
        let mut collector = Collector::new(self.repo()?)?;
        collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
        Ok(collector)
    }

    pub fn build_burst(&self) -> AppResult<()> {
        for i in 0..100 {
            self.write(&format!("target/{i}.o"), "build output\n")?;
        }
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
