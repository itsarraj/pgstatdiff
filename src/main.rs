use std::fs;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use pgstatdiff::model::Snapshot;
use pgstatdiff::{db, diff, render};

#[derive(Copy, Clone, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Parser)]
#[command(
    name = "pgstatdiff",
    version,
    about = "Snapshots pg_stat_database/pg_stat_user_tables and reports the delta across an interval — a no-extension activity profiler"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Take one snapshot right now and print it as JSON (to stdout, or a
    /// file with --out) — save it, do something to the database, then
    /// `diff` it against a second snapshot taken later.
    Snapshot {
        conninfo: String,
        #[arg(short, long)]
        out: Option<String>,
    },
    /// Diff two previously saved snapshot files.
    Diff {
        before: String,
        after: String,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Convenience wrapper: snapshot now, sleep, snapshot again, diff —
    /// one command instead of three.
    Watch {
        conninfo: String,
        /// Seconds to wait between the two snapshots.
        #[arg(short, long, default_value_t = 5.0)]
        interval: f64,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

fn load_snapshot(path: &str) -> Result<Snapshot> {
    let text = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse snapshot from {path}"))
}

fn print_report(report: &diff::DeltaReport, format: Format) -> Result<()> {
    match format {
        Format::Text => print!("{}", render::render_text(report)),
        Format::Json => println!("{}", render::render_json(report)?),
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Snapshot { conninfo, out } => {
            let mut client = db::connect(&conninfo)?;
            let snap = db::snapshot(&mut client)?;
            let json = serde_json::to_string_pretty(&snap)?;
            match out {
                Some(path) => {
                    fs::write(&path, json).with_context(|| format!("failed to write {path}"))?
                }
                None => println!("{json}"),
            }
        }
        Command::Diff {
            before,
            after,
            format,
        } => {
            let before = load_snapshot(&before)?;
            let after = load_snapshot(&after)?;
            let report = diff::compute(&before, &after)?;
            print_report(&report, format)?;
        }
        Command::Watch {
            conninfo,
            interval,
            format,
        } => {
            let mut client = db::connect(&conninfo)?;
            let before = db::snapshot(&mut client)?;
            sleep(Duration::from_secs_f64(interval.max(0.0)));
            let after = db::snapshot(&mut client)?;
            let report = diff::compute(&before, &after)?;
            print_report(&report, format)?;
        }
    }

    Ok(())
}
