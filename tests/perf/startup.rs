use crate::terminal::{Result, Session, SessionOptions};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Options {
    repo: PathBuf,
    binary: PathBuf,
    runs: usize,
    timeout: Duration,
}

impl Options {
    fn parse() -> Result<Option<Self>> {
        let mut args = env::args_os().skip(1);
        let mut repo = None;
        let mut binary = PathBuf::from(env!("CARGO_BIN_EXE_dift"));
        let mut runs = 5;
        let mut timeout = Duration::from_secs(30);
        while let Some(flag) = args.next() {
            if flag == "--help" || flag == "-h" {
                println!(
                    "Measure warmed-filesystem TUI startup, serially.\n\nUsage: cargo test --release --test perf -- --repo PATH [--binary PATH] [--runs N] [--timeout SECONDS]\n\nDefaults: Cargo-built dift, 5 runs, 30 seconds per run.\nReports JSON per run and medians; no timing pass/fail thresholds."
                );
                return Ok(None);
            }
            if !matches!(
                flag.to_str(),
                Some("--repo" | "--binary" | "--runs" | "--timeout")
            ) {
                return Err(format!("Unknown option: {}", flag.to_string_lossy()).into());
            }
            let value = args
                .next()
                .ok_or_else(|| format!("Missing value for {}", flag.to_string_lossy()))?;
            match flag.to_str() {
                Some("--repo") => repo = Some(PathBuf::from(value)),
                Some("--binary") => binary = PathBuf::from(value),
                Some("--runs") => runs = positive(value.to_str(), "--runs")?,
                Some("--timeout") => {
                    timeout = Duration::from_secs(positive(value.to_str(), "--timeout")? as u64)
                }
                _ => return Err(format!("Unknown option: {}", flag.to_string_lossy()).into()),
            }
        }
        let repo = repo.ok_or("--repo PATH is required")?.canonicalize()?;
        if !repo.is_dir() {
            return Err("--repo must name a directory".into());
        }
        let binary = binary.canonicalize()?;
        if !binary.is_file() {
            return Err("--binary must name an executable file".into());
        }
        Ok(Some(Self {
            repo,
            binary,
            runs,
            timeout,
        }))
    }
}

fn positive(value: Option<&str>, flag: &str) -> Result<usize> {
    value
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} must be a positive integer").into())
}

#[derive(Default)]
struct Measurement {
    first_frame: Option<f64>,
    readable: Option<f64>,
    highlighted: Option<f64>,
    complete: Option<f64>,
    watches: Option<usize>,
    polling: bool,
}

impl Measurement {
    fn json(&self) -> Value {
        json!({"first_frame_ms": self.first_frame, "readable_diff_ms": self.readable,
            "highlighted_ms": self.highlighted, "complete_ms": self.complete,
            "native_watches": self.watches, "polling": self.polling})
    }
}

fn measure(options: &Options, config: &Path) -> Result<Measurement> {
    let start = Instant::now();
    let mut session = Session::start(SessionOptions {
        binary: &options.binary,
        cwd: &options.repo,
        config,
        args: &[],
        env: &[],
    })?;
    let mut result = Measurement::default();
    while start.elapsed() < options.timeout {
        session.pump(
            (options.timeout - start.elapsed().min(options.timeout)).min(Duration::from_millis(10)),
        )?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if session.screen.contains("All changes") {
            result.first_frame.get_or_insert(elapsed);
        }
        let (readable, highlighted) = session.screen.visible_source();
        if readable {
            result.readable.get_or_insert(elapsed);
        }
        if highlighted {
            result.highlighted.get_or_insert(elapsed);
        }
        let footer = session.screen.row(session.screen.rows() - 2);
        let fields: Vec<_> = footer.trim().split(" · ").map(str::trim).collect();
        if fields.iter().any(|field| field.starts_with("Error:")) {
            return Err(session.failure(&footer));
        }
        let watching = footer.trim_start().starts_with("Watching");
        let polling = footer.trim_start().starts_with("Polling");
        if watching || polling {
            result.complete = Some(elapsed);
            result.watches = watch_count(session.pid());
            result.polling = polling;
            session.quit()?;
            return Ok(result);
        }
        session.ensure_running()?;
    }
    Err(session.failure(&format!(
        "Startup exceeded {} seconds",
        options.timeout.as_secs()
    )))
}

fn watch_count(pid: u32) -> Option<usize> {
    let entries = fs::read_dir(format!("/proc/{pid}/fdinfo")).ok()?;
    Some(
        entries
            .filter_map(|entry| fs::read_to_string(entry.ok()?.path()).ok())
            .map(|text| text.matches("inotify wd:").count())
            .sum(),
    )
}

fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    values.get(middle).map(|&value| {
        if values.len().is_multiple_of(2) {
            (values[middle - 1] + value) / 2.0
        } else {
            value
        }
    })
}

pub fn run() -> Result {
    let Some(options) = Options::parse()? else {
        return Ok(());
    };
    let directory = tempfile::Builder::new().prefix("dift-startup-").tempdir()?;
    let config = directory.path().join("config.kdl");
    fs::write(&config, "")?;
    let mut results = Vec::with_capacity(options.runs);
    for run in 1..=options.runs {
        let measurement = measure(&options, &config)?;
        let mut row = measurement.json();
        row["run"] = json!(run);
        println!("{row}");
        results.push(measurement);
    }
    println!(
        "{}",
        json!({"median": {
            "first_frame_ms": median(results.iter().filter_map(|m| m.first_frame)),
            "readable_diff_ms": median(results.iter().filter_map(|m| m.readable)),
            "highlighted_ms": median(results.iter().filter_map(|m| m.highlighted)),
            "complete_ms": median(results.iter().filter_map(|m| m.complete)),
            "native_watches": median(results.iter().filter_map(|m| m.watches.map(|count| count as f64))),
        }})
    );
    Ok(())
}
