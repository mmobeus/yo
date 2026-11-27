//! `yo down` and `yo sweep`: what they found, the question, and the run.

use std::io::{BufRead, Write};
use std::path::Path;

use yo_core::Ctx;
use yo_core::down::{DownOpts, Found, plan_down};
use yo_core::plan::{Action, Note, Plan, Report, execute};
use yo_core::runner::{LaunchSpec, Launched};
use yo_core::sweep::{SweepOpts, plan_sweep};

use crate::cli::{Down, Sweep};
use crate::render;
use crate::run::runner;

type Res = Result<i32, Box<dyn std::error::Error>>;

/// A yes from stdin.
fn confirm(out: &mut dyn Write, q: &str) -> bool {
    let _ = write!(out, "{q} [y/N] ");
    let _ = out.flush();
    let mut ans = String::new();
    let _ = std::io::stdin().lock().read_line(&mut ans);
    matches!(ans.trim(), "y" | "Y" | "yes" | "YES")
}

pub fn down(ctx: &Ctx, d: &Down, cwd: &Path, out: &mut dyn Write) -> Res {
    let (found, plan) = plan_down(
        ctx,
        cwd,
        DownOpts {
            yes: d.yes,
            discard: d.discard,
        },
    )?;
    let in_tab = ctx.sys.env_nonempty("ITERM_SESSION_ID").is_some();
    // A session still running that yo will end itself, from outside it.
    let ending = plan.actions.iter().find_map(|a| match a {
        Action::EndSession {
            harness,
            pid,
            after_close: false,
            ..
        } => Some(format!("{harness} (pid {pid})")),
        _ => None,
    });
    let tab = match (&ending, in_tab) {
        (Some(s), true) => format!("  end its {s}, still running, and close this iTerm2 tab."),
        (Some(s), false) => {
            format!("  end its {s}, still running (not inside iTerm2: no tab to close)")
        }
        (None, true) => {
            "  and close this iTerm2 tab, which ends the claude session in it.".to_string()
        }
        (None, false) => "  (not inside iTerm2: no tab to close)".to_string(),
    };
    let lead = |what: &str| {
        if d.dry_run {
            format!("Tearing down this {what} would:")
        } else {
            format!("About to tear down this {what}:")
        }
    };
    let (question, dirty, done) = match &found {
        Found::InPlace { dir, key } => {
            writeln!(out, "{}", lead("session"))?;
            writeln!(
                out,
                "  leave {} exactly as it is (no yo worktree here)",
                dir.display()
            )?;
            match key {
                Some(k) => writeln!(out, "  forget it in yo's session ledger ({})", k.0)?,
                None => writeln!(
                    out,
                    "  (no single ledger record for it here — yo forget <name> if one lingers)"
                )?,
            }
            writeln!(out, "{tab}")?;
            ("Close this tab?", false, None)
        }
        Found::Worktree {
            repo,
            worktree,
            branch,
            dirty,
            unpushed,
            base,
        } => {
            writeln!(out, "{}", lead("work session"))?;
            writeln!(out, "  repo:     {repo}")?;
            writeln!(out, "  worktree: {}", worktree.display())?;
            writeln!(out, "  branch:   {branch}   (KEPT — not deleted)")?;
            if !dirty.is_empty() {
                writeln!(
                    out,
                    "\n  !!  UNCOMMITTED / untracked changes — these will be LOST:"
                )?;
                for l in dirty {
                    writeln!(out, "        {l}")?;
                }
            }
            if !unpushed.is_empty() {
                writeln!(
                    out,
                    "\n  *   commits not on {base} (preserved on branch '{branch}'):"
                )?;
                for l in unpushed {
                    writeln!(out, "        {l}")?;
                }
            }
            writeln!(out, "\n{tab}")?;
            (
                "Remove the worktree and close this tab?",
                !dirty.is_empty(),
                Some(format!("worktree removed; branch '{branch}' kept.")),
            )
        }
    };
    if d.dry_run {
        if dirty {
            writeln!(
                out,
                "dry run: nothing changed. Uncommitted work means --yes alone will refuse; add --discard to lose it."
            )?;
        } else {
            writeln!(out, "dry run: nothing changed.")?;
        }
        return Ok(0);
    }
    if !d.yes && !confirm(out, question) {
        writeln!(out, "aborted.")?;
        return Ok(0);
    }
    // Everything but ending the session and closing the tab (either may end
    // yo), then say so.
    let (close, rest): (Vec<Action>, Vec<Action>) = plan
        .actions
        .into_iter()
        .partition(|a| matches!(a, Action::CloseOwnTab { .. } | Action::EndSession { .. }));
    let runner = runner(ctx);
    let mut report = render::Progress::new(out);
    execute(
        ctx,
        &Plan {
            notes: plan.notes,
            actions: rest,
        },
        &*runner,
        &mut report,
    )?;
    if let Some(done) = done {
        writeln!(out, "{done}")?;
    }
    let mut report = render::Progress::new(out);
    execute(
        ctx,
        &Plan {
            notes: vec![],
            actions: close,
        },
        &*runner,
        &mut report,
    )?;
    Ok(0)
}

/// Sweep's progress, in bash's words.
struct Swept<'a>(&'a mut dyn Write);

impl Report for Swept<'_> {
    fn note(&mut self, n: &Note) {
        let _ = match n {
            Note::Info(s) => writeln!(self.0, "  {s}"),
            Note::Detail(s) => writeln!(self.0, "    {s}"),
            Note::Warn(s) => writeln!(self.0, "  ! {s}"),
        };
    }
    fn doing(&mut self, _: &Action) {}
    fn launched(&mut self, _: &LaunchSpec, _: &Launched) {}
}

pub fn sweep(ctx: &Ctx, s: &Sweep, cwd: &Path, out: &mut dyn Write, err: &mut dyn Write) -> Res {
    // Adopt live sessions first, so one yo didn't start holds its worktree
    // through the ledger too.
    let _ = ctx.ledger().sync(&ctx.sync_clones(), &ctx.live());
    let (reports, notes, plan) = plan_sweep(
        ctx,
        cwd,
        &SweepOpts {
            delete_branches: s.delete_branches,
            include_stale: s.include_stale,
            include_suspended: s.include_suspended,
            repos: s.repos.clone(),
        },
    )?;
    for n in &notes {
        let (Note::Info(t) | Note::Detail(t) | Note::Warn(t)) = n;
        writeln!(err, "yo: {t}")?;
    }
    let mut total = 0;
    for r in &reports {
        if r.missing {
            let path = ctx
                .registry
                .repo(&r.key)
                .map(|x| x.path.display().to_string())
                .unwrap_or_default();
            writeln!(out, "\n{}: no clone at {path} — skipped", r.key)?;
            continue;
        }
        total += r.rows.len();
        writeln!(
            out,
            "\n{}  ({} worktrees, base {})",
            r.key,
            r.rows.len(),
            r.base
        )?;
        writeln!(out, "{}", "-".repeat(96))?;
        for row in &r.rows {
            let name = row
                .worktree
                .path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            writeln!(
                out,
                "  {:<5} {:<32} {:<40} {}",
                row.verdict.as_str(),
                name,
                row.worktree.branch.as_deref().unwrap_or("(detached)"),
                row.why
            )?;
        }
    }
    let removable = plan
        .actions
        .iter()
        .filter(|a| matches!(a, Action::SweepWorktree { .. }))
        .count();
    writeln!(out, "\n{removable} of {total} worktrees removable.")?;
    if removable == 0 {
        return Ok(0);
    }
    if !s.apply {
        writeln!(
            out,
            "Dry run. Re-run with --apply to remove{}",
            if s.delete_branches {
                "."
            } else {
                ", --delete-branches to drop their branches too."
            }
        )?;
        return Ok(0);
    }
    execute(ctx, &plan, &*runner(ctx), &mut Swept(out))?;
    Ok(0)
}
