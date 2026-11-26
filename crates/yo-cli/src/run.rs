//! Running a parsed command line against the core.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use yo_core::Ctx;
use yo_core::sys::Sys;

use crate::cli::{Cli, Command, ConfigCmd};
use crate::render;

/// Runs `cli`, writing to `out` and `err`, and returns the exit status.
pub fn run(
    cli: &Cli,
    sys: Arc<dyn Sys>,
    cwd: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match go(cli, sys, cwd, out, err) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "yo: {e}");
            1
        }
    }
}

fn go(
    cli: &Cli,
    sys: Arc<dyn Sys>,
    cwd: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(name) = read_only(cli.command.as_ref()) else {
        let what = match &cli.command {
            None => "starting or reaching a session".to_string(),
            Some(c) => format!("`yo {}`", subcommand_name(c)),
        };
        writeln!(
            err,
            "yo: {what} isn't in the Rust yo yet; scripts/yo still has it"
        )?;
        return Ok(1);
    };
    let ctx = Ctx::load(sys, cwd)?;
    let text = match name {
        ReadOnly::Help => render::help(&ctx),
        ReadOnly::ConfigShow => render::config_show(&ctx),
        ReadOnly::ConfigPath => format!("{}\n", ctx.paths.config.display()),
        ReadOnly::Sessions => {
            let live = ctx.live();
            // Adopt live sessions first, as bash does; a ledger problem
            // doesn't stop the listing.
            if let Ok(adopted) = ctx.ledger().sync(&ctx.sync_clones(), &live) {
                for a in adopted {
                    writeln!(err, "adopted {} ({}) from a live session", a.name, a.id)?;
                }
            }
            render::sessions(&ctx, &ctx.ledger().all(), &live)
        }
    };
    out.write_all(text.as_bytes())?;
    Ok(0)
}

enum ReadOnly {
    Help,
    ConfigShow,
    ConfigPath,
    Sessions,
}

/// The commands ported so far: the ones that only read (and `sessions`'
/// ledger sync).
fn read_only(c: Option<&Command>) -> Option<ReadOnly> {
    match c? {
        Command::Help => Some(ReadOnly::Help),
        Command::Config {
            command: None | Some(ConfigCmd::Show),
        } => Some(ReadOnly::ConfigShow),
        Command::Config {
            command: Some(ConfigCmd::Path),
        } => Some(ReadOnly::ConfigPath),
        Command::Sessions => Some(ReadOnly::Sessions),
        _ => None,
    }
}

fn subcommand_name(c: &Command) -> &'static str {
    match c {
        Command::Help => "help",
        Command::Config {
            command: Some(ConfigCmd::Edit),
        } => "config edit",
        Command::Config {
            command: Some(ConfigCmd::AddRepo { .. }),
        } => "config add-repo",
        Command::Config { .. } => "config",
        Command::Sessions => "sessions",
        Command::Resume(_) => "resume",
        Command::ResumeAll(_) => "resume-all",
        Command::Down(_) => "down",
        Command::Sweep(_) => "sweep",
        Command::Adopt(_) => "adopt",
        Command::Title { .. } => "title",
        Command::Forget { .. } => "forget",
        Command::Agent(_) => "agent",
        Command::SessionHook => "_session-hook",
    }
}
