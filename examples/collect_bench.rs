//! Reproducible collection benchmark: cargo run --release --example collect_bench -- 300 10
use dift::{
    AppResult,
    git::{Collector, Limits, Mode, Repository},
};
use std::{collections::BTreeSet, fs, path::PathBuf, process::Command, time::Instant};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    fn git(&self, args: &[&str]) -> AppResult<()> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args([
                "-c",
                "user.name=Benchmark",
                "-c",
                "user.email=bench@example.invalid",
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
}
fn main() -> AppResult<()> {
    let mut args = std::env::args().skip(1);
    let files: usize = args.next().as_deref().unwrap_or("300").parse()?;
    let repeats: usize = args.next().as_deref().unwrap_or("10").parse()?;
    if files == 0 || repeats == 0 {
        return Err("File and repeat counts must be positive".into());
    }
    let fixture =
        Fixture(std::env::temp_dir().join(format!("dift-collect-bench-{}", std::process::id())));
    fs::create_dir(&fixture.0)?;
    fixture.git(&["init", "-q"])?;
    fs::write(fixture.0.join(".gitignore"), "target/\n")?;
    for i in 0..files {
        fs::write(
            fixture.0.join(format!("file-{i}.rs")),
            format!("// file {i}\nfn main() {{\n    let value = 1;\n}}\n"),
        )?;
    }
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-qm", "initial"])?;
    fs::create_dir(fixture.0.join("target"))?;
    for i in 0..5000 {
        fs::write(fixture.0.join(format!("target/{i}")), "ignored\n")?;
    }
    for i in 0..files {
        fs::write(
            fixture.0.join(format!("file-{i}.rs")),
            format!("// file {i}\nfn main() {{\n    let value = 2;\n}}\n"),
        )?;
    }
    let mut collector = Collector::new(Repository::discover(&fixture.0)?)?;
    let loaded = BTreeSet::new();
    println!("scenario,files,milliseconds");
    for iteration in 0..=repeats {
        let start = Instant::now();
        let snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &loaded)?;
        if snapshot.changes.len() != files {
            return Err("Unexpected change count".into());
        }
        println!(
            "{},{files},{:.3}",
            if iteration == 0 { "cold" } else { "warm" },
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    fs::write(
        fixture.0.join("file-0.rs"),
        "fn main() { /* edited again */ }\n",
    )?;
    let start = Instant::now();
    let _snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &loaded)?;
    println!(
        "one-edit,{files},{:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}
