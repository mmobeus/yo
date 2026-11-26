//! Running a parsed command line against the core.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use yo_core::Ctx;
use yo_core::create::{CreateRequest, plan_create, unplaced};
use yo_core::ledger::Ledger;
use yo_core::ledger::State;
use yo_core::paths::Paths;
use yo_core::place::{self, PlaceTo};
use yo_core::plan::{Note, Plan, Report, execute};
use yo_core::reach::{ResumeOpts, ensure_guest, plan_focus, plan_reach};
use yo_core::registry::Mode;
use yo_core::resolve::{Ambiguous, HomeTarget, Resolved, resolve};
use yo_core::runner::iterm::Iterm;
use yo_core::runner::{DryLaunch, Runner, WindowChoice};
use yo_core::sys::Sys;

use crate::cli::{Cli, Command, ConfigCmd, Up, Window};
use crate::{ledger_cmds, render, teardown};

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
    match &cli.command {
        None => return up(&cli.up, sys, cwd, out),
        Some(Command::Resume(r)) => {
            return ledger_cmds::resume(&mut Ctx::load(sys, cwd)?, r, cwd, out);
        }
        Some(Command::ResumeAll(r)) => {
            return ledger_cmds::resume_all(&mut Ctx::load(sys, cwd)?, r, out);
        }
        Some(Command::Adopt(a)) => return ledger_cmds::adopt(&Ctx::load(sys, cwd)?, a, cwd, out),
        Some(Command::Forget { query }) => {
            return ledger_cmds::forget(&Ctx::load(sys, cwd)?, query, cwd, out);
        }
        Some(Command::Title { title }) => {
            return ledger_cmds::title(&Ctx::load(sys, cwd)?, title.as_deref(), cwd, out);
        }
        Some(Command::Down(d)) => return teardown::down(&Ctx::load(sys, cwd)?, d, cwd, out),
        Some(Command::Sweep(s)) => {
            return teardown::sweep(&Ctx::load(sys, cwd)?, s, cwd, out, err);
        }
        Some(_) => {}
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

/// `yo _session-hook`: a harness's ledger hook, with its payload. Loads no
/// config, so it's fast and a config problem can't break it.
pub fn session_hook(sys: Arc<dyn Sys>, payload: &str) {
    let ledger = Ledger::new(sys.clone(), Paths::new(&*sys));
    for h in yo_core::harness::all() {
        if h.capabilities().ledger_hooks
            && let Some(ev) = h.hook_event(payload)
        {
            ledger.hook(&ev);
            return;
        }
    }
}

/// Whether any flag was given: `yo NAME --flag` always creates. `--dry-run`
/// doesn't count, so a reach can be tried dry too.
fn has_flags(up: &Up) -> bool {
    up.repo.is_some()
        || up.profile.is_some()
        || up.here
        || up.worktree
        || up.window.new_window
        || up.window.tab
        || up.add_to.is_some()
        || up.title.is_some()
        || up.from.is_some()
        || up.model.is_some()
        || up.agent.is_some()
}

/// `yo [flags] [REPO|PROFILE] [INPUT] [EXTRA]`: reach what NAME names, or
/// create a session.
fn up(
    up: &Up,
    sys: Arc<dyn Sys>,
    cwd: &Path,
    out: &mut dyn Write,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut ctx = Ctx::load(sys, cwd)?;
    let live = ctx.live();
    let entries = ctx.ledger().all();
    let message = up.args.get(1).cloned();
    let opts = ResumeOpts {
        message: message.clone(),
        ..ResumeOpts::default()
    };
    let found = resolve(&ctx.registry, &entries, &live, &up.args, has_flags(up))
        .map_err(|a| ambiguous(&ctx, &a))?;
    let (plan, ready) = match found {
        Resolved::New => return create(up, &mut ctx, cwd, out),
        Resolved::Agent(n) => {
            return Err(format!(
                "{n} is an agent; `yo agent` isn't in the Rust yo yet (scripts/yo still has it)"
            )
            .into());
        }
        Resolved::Live(l) => {
            let name = &up.args[0];
            (
                plan_focus(&ctx, l.pid, name, None, message.as_deref()),
                false,
            )
        }
        Resolved::Ledger(e, st) => {
            let label = e.record.name.clone().unwrap_or_else(|| up.args[0].clone());
            say(out, &ensure_guest(&mut ctx, &e)?);
            let running = matches!(st, State::Running { .. });
            (plan_reach(&ctx, &e, st, &label, &opts)?, !running)
        }
        Resolved::Home { repo, target } => {
            let name = &up.args[0];
            let dir = ctx
                .registry
                .repo(&repo)
                .map(|r| r.path.clone())
                .unwrap_or_default();
            if !dir.is_dir() {
                return Err(format!(
                    "{name}'s home, repo '{repo}', has no clone at {}",
                    dir.display()
                )
                .into());
            }
            match target {
                HomeTarget::Live(l) => (
                    plan_focus(
                        &ctx,
                        l.pid,
                        &format!("{name}'s home"),
                        None,
                        message.as_deref(),
                    ),
                    false,
                ),
                HomeTarget::Ledger(e, st) => {
                    let label =
                        format!("{name}'s home ({})", e.record.name.as_deref().unwrap_or(""));
                    say(out, &ensure_guest(&mut ctx, &e)?);
                    let running = matches!(st, State::Running { .. });
                    (plan_reach(&ctx, &e, st, &label, &opts)?, !running)
                }
                HomeTarget::New => {
                    writeln!(
                        out,
                        "==> starting a session in {name}'s home ({})",
                        dir.display()
                    )?;
                    let mut start = Up {
                        repo: Some(repo),
                        here: true,
                        dry_run: up.dry_run,
                        ..Up::default()
                    };
                    start.args = std::iter::once("-".to_string()).chain(message).collect();
                    return create(&start, &mut ctx, cwd, out);
                }
            }
        }
    };
    run_plan(&ctx, &plan, up.dry_run, ready, out)
}

/// Prints notes as progress does.
pub(crate) fn say(out: &mut dyn Write, notes: &[Note]) {
    let mut report = render::Progress::new(out);
    for n in notes {
        report.note(n);
    }
}

/// More than one session matches: list them, and ask for the id.
fn ambiguous(ctx: &Ctx, a: &Ambiguous) -> Box<dyn std::error::Error> {
    let mut s = if a.entries.is_empty() {
        format!(
            "ambiguous: {} live sessions are named '{}'\n",
            a.live, a.query
        )
    } else {
        format!(
            "ambiguous: {} sessions match '{}'\n",
            a.entries.len(),
            a.query
        )
    };
    if !a.entries.is_empty() {
        let table = render::sessions(ctx, &a.entries, &ctx.live());
        // The rows, without the legend under them.
        for l in table.lines().take(a.entries.len() + 1) {
            s.push_str(l);
            s.push('\n');
        }
    }
    s.push_str("yo: be more specific (use the session id)");
    s.into()
}

/// Shows a plan (`--dry-run`) or runs it, on iTerm unless `YO_DRY_LAUNCH`.
fn run_plan(
    ctx: &Ctx,
    plan: &Plan,
    dry_run: bool,
    ready: bool,
    out: &mut dyn Write,
) -> Result<i32, Box<dyn std::error::Error>> {
    if dry_run {
        out.write_all(render::dry_run(plan).as_bytes())?;
        return Ok(0);
    }
    let dry_launch = ctx.sys.env_nonempty("YO_DRY_LAUNCH").is_some();
    let mut report = render::Progress::new(out);
    let iterm = Iterm::new(&ctx.registry.defaults);
    let runner: &dyn Runner = if dry_launch { &DryLaunch } else { &iterm };
    execute(ctx, plan, runner, &mut report)?;
    if ready {
        report.finish()?;
    }
    Ok(0)
}

/// Creates a session.
fn create(
    up: &Up,
    ctx: &mut Ctx,
    cwd: &Path,
    out: &mut dyn Write,
) -> Result<i32, Box<dyn std::error::Error>> {
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
    // A URL's repo nobody registered goes somewhere first: planning the
    // worktree needs its clone.
    if let Some(owner_repo) = unplaced(ctx, &req)? {
        let to = if place::claimed_by(ctx, &owner_repo).is_some() {
            PlaceTo::Guest // the claiming profile wins in `place`
        } else if let Some(a) = &up.add_to {
            PlaceTo::parse(ctx, a)?
        } else if can_ask(ctx) {
            ask_placement(&place::choices(ctx, &owner_repo), &owner_repo)
        } else {
            PlaceTo::Guest
        };
        if up.dry_run {
            let ctx = &*ctx;
            write!(
                out,
                "dry run: {owner_repo} isn't registered; yo would {} {} and register it {}, then plan the session\ndry run: nothing changed.\n",
                if place::clone_path(ctx, &owner_repo).exists() {
                    "use the clone at"
                } else {
                    "clone it to"
                },
                place::clone_path(ctx, &owner_repo).display(),
                match (place::claimed_by(ctx, &owner_repo), &to) {
                    (Some(p), _) => format!("in profile '{p}'"),
                    (None, PlaceTo::Profile(p)) => format!("in profile '{p}'"),
                    (None, PlaceTo::NewProfile(p)) => format!("in a new profile '{p}'"),
                    (None, PlaceTo::Guest) => "as a guest".to_string(),
                }
            )?;
            return Ok(0);
        }
        let (_, notes) = place::place(ctx, &owner_repo, &to)?;
        say(out, &notes);
    }
    let plan = plan_create(ctx, &req)?;
    run_plan(ctx, &plan, up.dry_run, true, out)
}

/// Whether to ask where a repo goes: at a terminal, outside a Claude
/// session (whose terminal isn't the person's), with a config to save to.
fn can_ask(ctx: &Ctx) -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && ctx.sys.env_nonempty("CLAUDECODE").is_none()
        && ctx.paths.config.is_file()
}

/// Asks on the terminal where an unregistered repo goes; anything but a
/// valid choice is a guest.
fn ask_placement(c: &place::Choices, owner_repo: &str) -> PlaceTo {
    use std::io::BufRead;
    let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return PlaceTo::Guest;
    };
    let owner = owner_repo.split('/').next().unwrap_or("");
    let mut menu = format!("{owner_repo} isn't configured. Where does it go?\n");
    for (i, p) in c.profiles.iter().enumerate() {
        menu.push_str(&format!("  [{}] add it to profile {p}\n", i + 1));
    }
    if let Some(n) = &c.new_profile {
        menu.push_str(&format!(
            "  [n] a new profile \"{n}\" that claims every {owner}/* repo\n"
        ));
    }
    menu.push_str(
        "  [g] just this once, as a guest (no shared memory, not pre-trusted)\nchoice [g]: ",
    );
    if tty.write_all(menu.as_bytes()).is_err() {
        return PlaceTo::Guest;
    }
    let mut ans = String::new();
    let _ = std::io::BufReader::new(&mut tty).read_line(&mut ans);
    place_answer(c, ans.trim())
}

/// A menu answer, as bash took it.
fn place_answer(c: &place::Choices, ans: &str) -> PlaceTo {
    match ans {
        "n" | "N" => c
            .new_profile
            .clone()
            .map_or(PlaceTo::Guest, PlaceTo::NewProfile),
        _ => ans
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| c.profiles.get(i))
            .map_or(PlaceTo::Guest, |p| PlaceTo::Profile(p.clone())),
    }
}

pub(crate) fn window(w: &Window) -> WindowChoice {
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

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn menu_answers() {
        let c = place::Choices {
            profiles: vec!["a".into(), "b".into()],
            new_profile: Some("neo".into()),
        };
        assert_eq!(place_answer(&c, ""), PlaceTo::Guest);
        assert_eq!(place_answer(&c, "g"), PlaceTo::Guest);
        assert_eq!(place_answer(&c, "2"), PlaceTo::Profile("b".into()));
        assert_eq!(place_answer(&c, "3"), PlaceTo::Guest);
        assert_eq!(place_answer(&c, "0"), PlaceTo::Guest);
        assert_eq!(place_answer(&c, "N"), PlaceTo::NewProfile("neo".into()));
        let c = place::Choices {
            new_profile: None,
            ..c
        };
        assert_eq!(place_answer(&c, "n"), PlaceTo::Guest);
    }
}
