//! `yo down` and `yo sweep`: what they found, the question, and the run.

use std::io::{BufRead, Write};
use std::path::Path;

use yo_core::Ctx;
use yo_core::down::{
    Activity, Children, DownOpts, Found, Other, OtherTab, Skip, Skipped, plan_down,
    plan_down_children, plan_down_named,
};
use yo_core::plan::{Action, Note, Plan, Report, execute};
use yo_core::runner::{LaunchSpec, Launched};
use yo_core::sweep::{SweepOpts, plan_sweep};

use crate::cli::{Down, Sweep};
use crate::ledger_cmds::{ambiguous, as_dir};
use crate::render;
use crate::run::runner;

type Res = Result<i32, Box<dyn std::error::Error>>;

/// What planning found, under the summary (so a dry run shows it too).
fn say_notes(out: &mut dyn Write, notes: &[Note]) -> std::io::Result<()> {
    for n in notes {
        let (Note::Info(t) | Note::Detail(t) | Note::Warn(t)) = n;
        writeln!(out, "  {t}")?;
    }
    Ok(())
}

/// A yes from stdin.
fn confirm(out: &mut dyn Write, q: &str) -> bool {
    let _ = write!(out, "{q} [y/N] ");
    let _ = out.flush();
    let mut ans = String::new();
    let _ = std::io::stdin().lock().read_line(&mut ans);
    matches!(ans.trim(), "y" | "Y" | "yes" | "YES")
}

/// A worktree `down` would remove: where it is, and what's lost or kept.
fn say_worktree(
    out: &mut dyn Write,
    repo: &str,
    worktree: &Path,
    branch: &str,
    dirty: &[String],
    unpushed: &[String],
    base: &str,
) -> std::io::Result<()> {
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
    Ok(())
}

pub fn down(ctx: &Ctx, d: &Down, cwd: &Path, out: &mut dyn Write) -> Res {
    if let Some(name) = &d.name {
        return down_named(ctx, d, name, cwd, out);
    }
    if d.children {
        return down_children(ctx, d, out);
    }
    let runner = runner(ctx);
    let (found, plan) = plan_down(
        ctx,
        &*runner,
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
        (None, true) => "  and close this iTerm2 tab, which ends the session in it.".to_string(),
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
            say_notes(out, &plan.notes)?;
            let q = if ending.is_some() {
                "End its session and close this tab?"
            } else {
                "Close this tab?"
            };
            (q, false, None)
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
            say_worktree(out, repo, worktree, branch, dirty, unpushed, base)?;
            writeln!(out, "\n{tab}")?;
            say_notes(out, &plan.notes)?;
            (
                if ending.is_some() {
                    "Remove the worktree, end its session and close this tab?"
                } else {
                    "Remove the worktree and close this tab?"
                },
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
    let mut report = render::Progress::new(out);
    execute(
        ctx,
        &Plan {
            notes: vec![],
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

/// What happens to another session, as `down NAME` and `--children` say
/// it, each line indented.
fn say_other(out: &mut dyn Write, o: &Other, plan: &Plan) -> std::io::Result<()> {
    match &o.found {
        Found::Worktree {
            repo,
            worktree,
            branch,
            dirty,
            unpushed,
            base,
        } => {
            say_worktree(out, repo, worktree, branch, dirty, unpushed, base)?;
            writeln!(out)?;
        }
        Found::InPlace { dir, .. } => writeln!(
            out,
            "  leave {} exactly as it is (not a yo worktree to remove)",
            dir.display()
        )?,
    }
    if let Some(k) = plan.actions.iter().find_map(|a| match a {
        Action::DropRecord(k) => Some(k),
        _ => None,
    }) {
        writeln!(out, "  forget it in yo's session ledger ({})", k.0)?;
    }
    if let Some((harness, pid)) = plan.actions.iter().find_map(|a| match a {
        Action::EndSession { harness, pid, .. } => Some((harness, pid)),
        _ => None,
    }) {
        let how = match o.activity {
            Activity::Idle => "idle",
            Activity::Busy => "STILL WORKING",
            _ => "running; yo can't tell whether it's still working",
        };
        writeln!(out, "  end its {harness} session (pid {pid}), {how}")?;
    } else if o.activity == Activity::Unknown {
        writeln!(
            out,
            "  (it may still be running: no session id recorded yet, so yo can't tell)"
        )?;
    }
    match &o.tab {
        OtherTab::Close(t) => writeln!(out, "  close its iTerm2 tab ({t})")?,
        OtherTab::NoneRecorded => writeln!(out, "  (no tab recorded for it: none to close)")?,
        OtherTab::NotOpen => writeln!(out, "  (its tab is already closed)")?,
        OtherTab::Unconfirmed(t) => writeln!(
            out,
            "  leave tab {t} open: it no longer shows this session's title or terminal (close it by hand if it's this one's)"
        )?,
        OtherTab::CantTell => writeln!(
            out,
            "  leave its tab as it is: yo can't see iTerm2's tabs (close it by hand if it's open)"
        )?,
    }
    say_notes(out, &plan.notes)
}

/// `yo down NAME`: tears down another session, from outside it.
fn down_named(ctx: &Ctx, d: &Down, q: &str, cwd: &Path, out: &mut dyn Write) -> Res {
    let q = as_dir(cwd, q);
    let e = match ctx.ledger().get(&q) {
        Ok(Some(e)) => e,
        Ok(None) => {
            return Err(format!("no session matches '{q}' (yo sessions lists them)").into());
        }
        Err(hits) => return Err(ambiguous(ctx, &q, &hits)),
    };
    let runner = runner(ctx);
    let (other, plan) = plan_down_named(
        ctx,
        &*runner,
        &e,
        DownOpts {
            yes: d.yes,
            discard: d.discard,
        },
    )?;
    let name = &other.name;
    if d.dry_run {
        writeln!(out, "Tearing down {name} would:")?;
    } else {
        writeln!(out, "About to tear down {name}:")?;
    }
    say_other(out, &other, &plan)?;
    let dirty = matches!(&other.found, Found::Worktree { dirty, .. } if !dirty.is_empty());
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
    let question = if other.activity.working() {
        format!("{name} may still be working. Tear it down anyway?")
    } else {
        format!("Tear down {name}?")
    };
    if !d.yes && !confirm(out, &question) {
        writeln!(out, "aborted.")?;
        return Ok(0);
    }
    let mut report = render::Progress::new(out);
    execute(ctx, &plan, &*runner, &mut report)?;
    if let Found::Worktree { branch, .. } = &other.found {
        writeln!(out, "worktree removed; branch '{branch}' kept.")?;
    }
    writeln!(out, "{name} torn down.")?;
    Ok(0)
}

/// `yo down --children`: tears down the sessions this one started that
/// are done, skipping (and listing) those that may still be working or
/// would lose uncommitted work.
fn down_children(ctx: &Ctx, d: &Down, out: &mut dyn Write) -> Res {
    let runner = runner(ctx);
    let Children { go, skipped } = plan_down_children(ctx, &*runner, d.discard)?;
    if go.is_empty() && skipped.is_empty() {
        writeln!(out, "no sessions in the ledger were started from this one.")?;
        return Ok(0);
    }
    for (other, plan) in &go {
        if d.dry_run {
            writeln!(out, "Tearing down {} would:", other.name)?;
        } else {
            writeln!(out, "To tear down {}:", other.name)?;
        }
        say_other(out, other, plan)?;
        writeln!(out)?;
    }
    if !skipped.is_empty() {
        writeln!(out, "Skipped:")?;
        for Skipped { name, why } in &skipped {
            match why {
                Skip::Working => writeln!(out, "  {name}: may still be working")?,
                Skip::Uncommitted(files) => {
                    writeln!(
                        out,
                        "  {name}: uncommitted changes (--discard to lose them):"
                    )?;
                    for f in files {
                        writeln!(out, "        {f}")?;
                    }
                }
                Skip::Refused(m) => writeln!(out, "  {name}: {}", m.replace('\n', "\n  "))?,
            }
        }
    }
    if go.is_empty() {
        writeln!(out, "nothing to tear down.")?;
        return Ok(0);
    }
    if d.dry_run {
        writeln!(out, "dry run: nothing changed.")?;
        return Ok(0);
    }
    let names: Vec<&str> = go.iter().map(|(o, _)| o.name.as_str()).collect();
    if !d.yes && !confirm(out, &format!("Tear down {}?", names.join(", "))) {
        writeln!(out, "aborted.")?;
        return Ok(0);
    }
    // One that fails doesn't stop the rest.
    let mut failed = Vec::new();
    for (other, plan) in &go {
        writeln!(out, "==> {}", other.name)?;
        let mut report = render::Progress::new(out);
        if let Err(e) = execute(ctx, plan, &*runner, &mut report) {
            writeln!(out, "  ! {}: {e}", other.name)?;
            failed.push(other.name.as_str());
        }
    }
    let done: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| !failed.contains(n))
        .collect();
    if !done.is_empty() {
        writeln!(out, "torn down: {}.", done.join(", "))?;
    }
    if failed.is_empty() {
        Ok(0)
    } else {
        writeln!(out, "not torn down: {}.", failed.join(", "))?;
        Ok(1)
    }
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
