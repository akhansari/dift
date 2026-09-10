#[cfg(unix)]
mod terminal;

#[cfg(unix)]
#[path = "perf/startup.rs"]
mod startup;

#[cfg(unix)]
fn main() -> terminal::Result {
    startup::run()
}

#[cfg(not(unix))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Terminal performance measurements currently require Unix".into())
}
