use dift::{
    AppResult,
    config::Config,
    git::{Mode, Repository},
    ui,
};
use std::{ffi::OsStr, path::PathBuf, process::ExitCode};
use usage_rs as usage;

#[derive(usage::Cli)]
#[usage(bin = "dift", version)]
struct Cli {
    /// Read a specific KDL configuration file.
    #[usage(long, global)]
    config: Option<PathBuf>,
    /// Print the complete default KDL configuration and exit.
    #[usage(long)]
    default_config: bool,
    #[usage(subcommand)]
    command: Option<Commands>,
}

#[derive(usage::Subcommands)]
enum Commands {
    /// Compare two commits or branch tips directly.
    Diff(CompareArgs),
    /// Compare the merge base of BASE and HEAD with HEAD.
    Review(CompareArgs),
}

#[derive(usage::Args)]
struct CompareArgs {
    /// Base commit or branch.
    base: String,
    /// Target commit or branch.
    head: String,
}

fn main() -> ExitCode {
    let owned: Vec<_> = std::env::args_os().skip(1).collect();
    let argv: Vec<&OsStr> = owned.iter().map(|s| s.as_os_str()).collect();
    let cli = match Cli::parse_from(&argv) {
        Ok(cli) => cli,
        Err(usage::Error::Help { cmd, long }) => {
            print!(
                "{}",
                usage::help::render_styled(Cli::spec(), cmd, long, usage::help::Style::PLAIN)
                    .unwrap_or_default()
            );
            return ExitCode::SUCCESS;
        }
        Err(usage::Error::Version { .. }) => {
            println!("dift {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!(
                "{}",
                usage::render_failure_plain(Cli::spec(), &argv, &error)
            );
            return ExitCode::from(2);
        }
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dift: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> AppResult<()> {
    if cli.default_config {
        print!("{}", Config::template());
        return Ok(());
    }
    let config = Config::load(cli.config)?;
    let repo = Repository::discover(&std::env::current_dir()?)?;
    let mode = match cli.command {
        None => Mode::Watch,
        Some(Commands::Diff(args)) => repo.comparison(&args.base, &args.head, false)?,
        Some(Commands::Review(args)) => repo.comparison(&args.base, &args.head, true)?,
    };
    ui::run(repo, mode, config)
}
