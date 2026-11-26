//! Running a parsed command line against the core.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use yo_core::Ctx;
use yo_core::create::{CreateRequest, plan_create};
use yo_core::plan::execute;
use yo_core::registry::Mode;
use yo_core::runner::{DryLaunch, WindowChoice};
use yo_core::sys::Sys;

use crate::cli::{Cli, Command, ConfigCmd, Up, Window};
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
    if cli.command.is_none() {
        return create(&cli.up, sys, cwd, out);
    }
    let Some(name) = read_only(cli.command.as_ref()) else {
        if let Some(c) = &cli.command {
            writeln!(
                err,
                "yo: `yo {}` isn't in the Rust yo yet; scripts/yo still has it",
                subcommand_name(c)
            )?;
        }
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

/// `yo [flags] [REPO|PROFILE] [INPUT] [EXTRA]`. Reaching a session by
/// name (`yo NAME`) isn't ported yet, so this always creates.
fn create(
    up: &Up,
    sys: Arc<dyn Sys>,
    cwd: &Path,
    out: &mut dyn Write,
) -> Result<i32, Box<dyn std::error::Error>> {
    // Until the iTerm runner is ported, only a dry launch can run: refuse
    // before anything is created, not after.
    let dry_launch = sys.env_nonempty("YO_DRY_LAUNCH").is_some();
    if !up.dry_run && !dry_launch {
        return Err("launching in iTerm isn't in the Rust yo yet; use --dry-run, or YO_DRY_LAUNCH=1 to do everything but open the tab".into());
    }
    let ctx = Ctx::load(sys, cwd)?;
    let req = CreateRequest {
        cwd: cwd.to_path_buf(),
        repo: up.repo.clone(),
        profile: up.profile.clone(),
        mode: if up.here {
            Some(Mode::Here)
        } else if up.worktree {
            Some(Mode::Worktree)
        } else {
            None
        },
        window: window(&up.window),
        add_to: up.add_to.clone(),
        title: up.title.clone(),
        from: up.from.clone(),
        model: up.model.clone(),
        agent: up.agent.clone(),
        args: up.args.clone(),
    };
    let plan = plan_create(&ctx, &req)?;
    if up.dry_run {
        out.write_all(render::dry_run(&plan).as_bytes())?;
        return Ok(0);
    }
    let mut report = render::Progress::new(out);
    execute(&ctx, &plan, &DryLaunch, &mut report)?;
    report.finish()?;
    Ok(0)
}

fn window(w: &Window) -> WindowChoice {
    if w.new_window {
        WindowChoice::Window
    } else if w.tab {
        WindowChoice::Tab
    } else {
        WindowChoice::Config
    }
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
