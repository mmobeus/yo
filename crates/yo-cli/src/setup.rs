//! `yo setup`: the plan, step by step, then what's left to do.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use yo_core::paths::Paths;
use yo_core::plan::Note;
use yo_core::setup::{
    Kept, Removed, SetupOpts, Step, Wrote, execute, hook_approved, hook_offers, plan,
};
use yo_core::sys::Sys;

use crate::cli::Setup;

type Res = Result<i32, Box<dyn std::error::Error>>;

pub fn setup(sys: Arc<dyn Sys>, s: &Setup, cwd: &Path, out: &mut dyn Write) -> Res {
    let paths = Paths::new(&*sys);
    let mut opts = SetupOpts {
        uninstall: s.uninstall,
        merge_settings: s.merge_settings,
        iterm: !s.no_iterm,
        force: s.force,
        link: s.link.as_ref().map(|l| cwd.join(l)),
        hooks: s.hooks,
    };
    let asking = at_terminal(&*sys);
    let offers = if s.uninstall || s.hooks {
        Vec::new()
    } else {
        hook_offers(&*sys)
    };
    if asking && !s.dry_run {
        for o in &offers {
            writeln!(out, "{}", o.what)?;
            opts.hooks |= crate::run::ask_yes(&format!("Add yo's {} hook?", o.harness), false);
        }
    }
    let p = plan(&*sys, &paths, &opts)?;
    let home = sys.home();
    let t = Tilde(&home);

    let what = match (&p.checkout, opts.uninstall) {
        (_, true) => "==> uninstalling yo's skills and settings".to_string(),
        (Some(c), _) => format!("==> setting up yo, linked into {}", t.p(c)),
        (None, _) => "==> setting up yo, from copies built into this yo".to_string(),
    };
    writeln!(out, "{what}")?;
    if s.dry_run {
        writeln!(out, "dry run: nothing will be written")?;
    }
    for n in &p.notes {
        match n {
            Note::Warn(w) => writeln!(out, "yo: {w}")?,
            Note::Info(i) | Note::Detail(i) => writeln!(out, "    {i}")?,
        }
    }
    if s.dry_run {
        for step in &p.steps {
            out.write_all(line(&t, step).as_bytes())?;
        }
        for o in offers.iter().filter(|_| !opts.hooks) {
            writeln!(
                out,
                "    not added: yo's {} hook (setup asks at a terminal; --hooks adds it)",
                o.harness
            )?;
        }
        writeln!(out, "dry run: nothing changed.")?;
        return Ok(0);
    }
    execute(&*sys, &p, &mut |step| {
        let _ = out.write_all(line(&t, step).as_bytes());
    })?;

    for a in &p.approve {
        if asking
            && crate::run::ask_yes(
                &format!("Open {} now to approve yo's hook?", a.harness),
                true,
            )
        {
            writeln!(out, "==> {}", a.how)?;
            let ran = std::process::Command::new(&a.command)
                .current_dir(&home)
                .status();
            if let Err(e) = ran {
                writeln!(out, "yo: couldn't run {}: {e}", a.command)?;
            }
        }
        if hook_approved(&*sys, &a.harness) {
            writeln!(out, "  ok      yo's {} hook is approved", a.harness)?;
        } else {
            writeln!(
                out,
                "yo: yo's {} hook isn't approved yet, so {} stops at its hook review until it is: \
                 run {} and approve it there (or run yo setup again)",
                a.harness, a.harness, a.command
            )?;
        }
    }
    if opts.uninstall {
        writeln!(
            out,
            "done. Backups of anything rewritten end in .bak-<time>; yo itself stays (brew uninstall yo)."
        )?;
    } else if p.checkout.is_some() {
        writeln!(
            out,
            "done. The files are links into the checkout: edits there are live, and moving it breaks them."
        )?;
    } else {
        writeln!(out, "done. Run yo setup again after upgrading yo.")?;
    }
    let config = p.steps.iter().any(|st| {
        matches!(
            st,
            Step::Write {
                what: Wrote::Config { checkout: None },
                ..
            }
        )
    });
    if config {
        writeln!(
            out,
            "next: register a repo with yo config add-repo <clone|owner/repo>"
        )?;
    }
    Ok(0)
}

/// Whether the person can be asked: at a terminal, outside a harness's
/// session (whose terminal isn't the person's).
fn at_terminal(sys: &dyn Sys) -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !yo_core::harness::all().iter().any(|h| h.inside(sys))
}

/// Paths under `HOME` as `~/...`.
struct Tilde<'a>(&'a Path);

impl Tilde<'_> {
    fn p(&self, p: &Path) -> String {
        match p.strip_prefix(self.0) {
            Ok(r) => format!("~/{}", r.display()),
            Err(_) => p.display().to_string(),
        }
    }
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One step, as it's done (or would be).
fn line(t: &Tilde<'_>, step: &Step) -> String {
    let row = |verb: &str, rest: String| format!("  {verb:<7} {rest}\n");
    match step {
        Step::Keep { path, why } => {
            let p = t.p(path);
            match why {
                Kept::Installed => row("ok", format!("{p} (up to date)")),
                Kept::Linked => row("ok", format!("{p} (linked)")),
                Kept::Config => row("ok", format!("{p} (your config, left alone)")),
                Kept::Merged => row("ok", format!("{p} (already merged)")),
                Kept::Hooked => row("ok", format!("{p} (has yo's hook)")),
                Kept::Edited => row("kept", format!("{p} (changed since yo installed it)")),
                Kept::Foreign { to } => row(
                    "kept",
                    format!("{p} (a link to {}; --force replaces it)", t.p(to)),
                ),
                Kept::RunningYo { to } => row(
                    "kept",
                    format!(
                        "{p} (-> {}, the yo running now; once yo is installed another way, run yo setup again to remove it)",
                        t.p(to)
                    ),
                ),
            }
        }
        Step::Backup { path, to, .. } => row("backup", format!("{} -> {}", t.p(path), name(to))),
        Step::Remove { path, why } => {
            let p = t.p(path);
            let why = match why {
                Removed::Link { to } => format!("a link to {}", t.p(to)),
                Removed::Forced { to } => format!("a link to {}, replaced with --force", t.p(to)),
                Removed::Copy => "yo's copy".to_string(),
                Removed::Dangling { to } => format!("a link to {}, which is gone", t.p(to)),
                Removed::OldYo { to } => format!("an old link to {}", t.p(to)),
            };
            row("rm", format!("{p} ({why})"))
        }
        Step::Copy(b) => row("copy", t.p(&b.dest)),
        Step::Link { path, to } => row("link", format!("{} -> {}", t.p(path), t.p(to))),
        Step::Write { path, what, .. } => {
            let p = t.p(path);
            match what {
                Wrote::Config { checkout: Some(c) } => {
                    row("create", format!("{p} (lists {} as yo)", t.p(c)))
                }
                Wrote::Config { checkout: None } => row("create", format!("{p} (no repos yet)")),
                Wrote::Settings => row("create", format!("{p} (yo's Claude settings)")),
                Wrote::Merged { keys } => row(
                    "merge",
                    format!("{p} (yo's keys win: {}; the rest kept)", keys.join(", ")),
                ),
                Wrote::Unmerged { keys } => {
                    row("unmerge", format!("{p} (dropping {})", keys.join(", ")))
                }
                Wrote::Hook { harness } => {
                    row("hook", format!("{p} (yo's {harness} hook; the rest kept)"))
                }
                Wrote::Unhook { harness } => {
                    row("unhook", format!("{p} (dropping yo's {harness} hook)"))
                }
            }
        }
        Step::AddPeer {
            name,
            running,
            backup,
            ..
        } => {
            let mut s = row(
                "peer",
                format!("iTerm2's Claude Code workgroup <- terminal peer '{name}'"),
            );
            s.push_str(&format!(
                "          (the workgroups as they were: {})\n",
                t.p(backup)
            ));
            if *running {
                s.push_str(
                    "          iTerm2 is running: the peer appears after its next restart. Don't\n          edit workgroups in iTerm2's settings until then, or it's overwritten.\n",
                );
            }
            s
        }
        Step::PeerPresent => row(
            "ok",
            "iTerm2's Claude Code workgroup (has a terminal peer)".into(),
        ),
        Step::Manifest { path, contents } => match contents {
            Some(_) => row("record", format!("{} (what setup installed)", t.p(path))),
            None => row("rm", format!("{} (what setup installed)", t.p(path))),
        },
    }
}
