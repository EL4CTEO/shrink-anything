//! shrink-anything: delete the code, files and dependencies your app doesn't use, and prove with
//! your own tests that nothing broke.

mod agent;
mod apply;
mod candidate;
mod check;
mod cut;
mod defs;
mod doctor;
mod find;
mod index;
mod lang;
mod lex;
mod mcp;
mod report;
mod scan;
mod show;
mod state;
mod ui;
mod util;
mod walk;
mod weigh;

use anyhow::{Result, bail};
use candidate::{CandKind, Tier};
use clap::{Args, Parser, Subcommand};
use state::Ctx;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "shrink-anything", version, about, arg_required_else_help = true)]
#[command(
    after_help = "One command:\n  shrink-anything go --agent claude -- npm test\n\nStep by step:\n  shrink-anything scan\n  shrink-anything cut -- npm test\n  shrink-anything task --agent claude\n  shrink-anything report\n  shrink-anything undo"
)]
struct Cli {
    /// Project folder (default: the current folder)
    #[arg(short = 'C', long = "dir", global = true, default_value = ".")]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Mode {
    /// It's a library: keep everything public, its users aren't in this repo
    #[arg(long, conflicts_with = "app")]
    lib: bool,
    /// It's an app: nothing outside this repo calls it
    #[arg(long)]
    app: bool,
}

impl Mode {
    fn library(&self) -> Option<bool> {
        if self.lib {
            Some(true)
        } else if self.app {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Args)]
struct CutArgs {
    /// Also cut what something could reach without naming it: public members, a library's exports
    #[arg(long)]
    aggressive: bool,
    /// Only these kinds: code, file, dep, asset
    #[arg(long, value_delimiter = ',')]
    only: Vec<CandKind>,
    /// Stop after this many runs of the check
    #[arg(long, default_value_t = 40)]
    max_runs: usize,
    /// Cutting can orphan more code: scan and cut again up to this many times
    #[arg(long, default_value_t = 4)]
    rounds: usize,
    /// A build output (file or folder) to weigh before and after, e.g. dist or target/release/app
    #[arg(long)]
    size: Vec<String>,
    /// A shell command to run after each edit, before the check (a formatter, an import fixer)
    #[arg(long)]
    fix: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Everything in one go: scan, cut what the check proves dead, hand the rest to your agent, report
    Go {
        /// Coding agent for duplicates and what needs a look: claude, codex, or any command that reads a prompt on stdin
        #[arg(long)]
        agent: Option<String>,
        /// Agent attempts before its changes are put back
        #[arg(long, default_value_t = 2)]
        attempts: usize,
        #[command(flatten)]
        cut: CutArgs,
        #[command(flatten)]
        mode: Mode,
        /// The command that proves your project works (tests, a build), after --
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Show what could go. Changes nothing
    Scan {
        /// Print everything as JSON
        #[arg(long)]
        json: bool,
        /// List every candidate, with why
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        mode: Mode,
    },
    /// Delete what's dead and run your check: what passes stays, what breaks is put back
    Cut {
        #[command(flatten)]
        cut: CutArgs,
        #[command(flatten)]
        mode: Mode,
        /// The command that proves your project works, after -- (remembered once given)
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Write .shrink/TASK.md for what needs judgment (duplicates, public members), or have an agent do it
    Task {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, default_value_t = 2)]
        attempts: usize,
        #[command(flatten)]
        mode: Mode,
    },
    /// Run your check now: fails if it breaks or fewer tests run than before the first cut
    Verify {
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Take back the last cut
    Undo {
        /// Take back every cut
        #[arg(long)]
        all: bool,
        /// Restore even files that changed since
        #[arg(long)]
        force: bool,
    },
    /// Before and after, as markdown and a shareable SVG card
    Report {
        #[command(flatten)]
        mode: Mode,
    },
    /// Where the weight of a build is: by kind of file, the heaviest files, what's in there twice
    Weigh {
        /// Build outputs to weigh, e.g. dist (default: the usual build folders that exist)
        paths: Vec<String>,
    },
    /// The check command and every run so far
    Status,
    /// What's installed: parsers, package managers, agents
    Doctor,
    /// Run as an MCP server on stdio (for Claude Code, Codex, Cursor, ...)
    Mcp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = util::absolute(&cli.dir);
    match run(cli.cmd, &root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            ui::error(format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn configure(ctx: &mut Ctx, command: Vec<String>, cut: &CutArgs) {
    if !command.is_empty() {
        ctx.state.check = command;
    }
    if !cut.size.is_empty() {
        ctx.state.size = cut.size.clone();
    }
    if cut.fix.is_some() {
        ctx.state.fix = cut.fix.clone();
    }
}

fn options(cut: &CutArgs, mode: &Mode) -> cut::Opts {
    cut::Opts {
        aggressive: cut.aggressive,
        only: cut.only.clone(),
        max_runs: cut.max_runs,
        rounds: cut.rounds,
        library: mode.library(),
    }
}

/// Writes the task for whatever the cut left, and says how to hand it over.
fn leftovers(ctx: &Ctx, library: Option<bool>) -> Result<()> {
    let scanned = scan::scan(&ctx.root, library)?;
    if let Some(path) = agent::write_task(ctx, &scanned)? {
        ui::line(format!(
            "  left for a reviewer: {} duplicates, {} to look at ({})",
            scanned.duplicates.len(),
            scanned.tier(Tier::Risky).count(),
            path.strip_prefix(&ctx.root).unwrap_or(&path).display()
        ));
        show::next("shrink-anything task --agent claude");
    }
    Ok(())
}

fn run(cmd: Cmd, root: &Path) -> Result<()> {
    if matches!(cmd, Cmd::Mcp) {
        return mcp::serve();
    }
    if let Cmd::Scan { json: true, mode, .. } = &cmd {
        println!("{}", serde_json::to_string_pretty(&scan::scan(root, mode.library())?)?);
        return Ok(());
    }
    ui::banner();
    if matches!(cmd, Cmd::Doctor) {
        return doctor::run();
    }
    let mut ctx = Ctx::open(root)?;
    match cmd {
        Cmd::Go { agent, attempts, cut, mode, command } => {
            configure(&mut ctx, command, &cut);
            let library = mode.library();
            let outcome = cut::cut(&mut ctx, &options(&cut, &mode))?;
            show::cut(&outcome);
            match agent {
                Some(agent) => show::agent(&agent::work(&mut ctx, &agent, attempts, library)?),
                None => leftovers(&ctx, library)?,
            }
            if ctx.state.baseline.is_some() {
                show::report(&report::report(&ctx, library)?);
            }
            Ok(())
        }
        Cmd::Scan { all, mode, .. } => {
            show::scan(&scan::scan(root, mode.library())?, all, !ctx.state.check.is_empty());
            Ok(())
        }
        Cmd::Cut { cut, mode, command } => {
            configure(&mut ctx, command, &cut);
            let outcome = cut::cut(&mut ctx, &options(&cut, &mode))?;
            show::cut(&outcome);
            leftovers(&ctx, mode.library())?;
            if outcome.run.is_some() {
                show::next("shrink-anything report");
            }
            Ok(())
        }
        Cmd::Task { agent: Some(agent), attempts, mode } => {
            let outcome = agent::work(&mut ctx, &agent, attempts, mode.library())?;
            show::agent(&outcome);
            if outcome.rejected.is_some() {
                bail!("{agent} couldn't shrink it further without breaking the check");
            }
            Ok(())
        }
        Cmd::Task { agent: None, mode, .. } => {
            let scanned = scan::scan(root, mode.library())?;
            match agent::write_task(&ctx, &scanned)? {
                Some(path) => {
                    ui::ok(format!("wrote {}: hand it to your agent, then run shrink-anything verify", path.display()));
                }
                None => ui::ok("nothing left that needs an agent"),
            }
            Ok(())
        }
        Cmd::Verify { command } => {
            if !command.is_empty() {
                ctx.state.check = command;
            }
            ctx.save()?;
            let verified = cut::verify(&ctx)?;
            show::verify(&verified);
            if !verified.ok {
                bail!("put the last change back, or fix it");
            }
            Ok(())
        }
        Cmd::Undo { all, force } => {
            show::undo(&state::undo(&mut ctx, all, force)?);
            Ok(())
        }
        Cmd::Report { mode } => {
            show::report(&report::report(&ctx, mode.library())?);
            Ok(())
        }
        Cmd::Weigh { paths } => {
            let paths = if paths.is_empty() { ctx.state.size.clone() } else { paths };
            show::weigh(&weigh::weigh(root, &paths)?);
            Ok(())
        }
        Cmd::Status => {
            show::status(&ctx);
            Ok(())
        }
        Cmd::Doctor | Cmd::Mcp => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_check_comes_after_the_dashes() {
        let cli = Cli::try_parse_from(["shrink-anything", "cut", "--only", "code,dep", "--", "npm", "test"]).unwrap();
        let Cmd::Cut { cut, command, .. } = cli.cmd else { panic!("not cut") };
        assert_eq!(command, ["npm", "test"]);
        assert_eq!(cut.only, [CandKind::Symbol, CandKind::Dep]);
        assert!(Cli::try_parse_from(["shrink-anything", "go"]).is_err(), "go needs a check");
        assert!(Cli::try_parse_from(["shrink-anything", "scan", "--lib", "--app"]).is_err());
        assert!(matches!(
            Cli::try_parse_from(["shrink-anything", "undo", "--all"]).unwrap().cmd,
            Cmd::Undo { all: true, force: false }
        ));
    }
}
