//! `yo down`: tearing down the session yo runs in. In a yo worktree, the
//! worktree is removed (its branch kept), its record dropped and its tab
//! closed; anywhere else, nothing on disk is touched. bash's `cmd_down`.
//!
//! Its session is ended too, if it's still running: closing a tab doesn't
//! always end it (see "Closing a tab" in [`crate::runner::iterm`]). Run
//! from a shell (the tab's CLI, a split), `down` ends it before closing the
//! tab, but only when it's in this tab (its recorded tab is the one yo runs
//! in): a session in another tab is left running, and said so. Run from
//! inside it, a detached helper ends it once the tab has had time to close.
//!
//! What it refuses, it refuses while planning, before anything changes:
//! an agent's home, a repo it can't identify, and with `--yes` but not
//! `--discard`, uncommitted work.

use std::path::{Path, PathBuf};

use crate::ledger::{Entry, Key, Record};
use crate::plan::{Action, Note, Plan};
use crate::registry::Mode;
use crate::runner::Runner;
use crate::{Ctx, Error, Result, git};

#[derive(Clone, Copy, Debug, Default)]
pub struct DownOpts {
    /// Don't ask (from inside the session, where nobody can answer).
    pub yes: bool,
    /// With `yes`: lose uncommitted work.
    pub discard: bool,
}

/// What `down` found, for the CLI to show and ask about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// Not a yo worktree: nothing on disk is ours.
    InPlace {
        dir: PathBuf,
        /// The one ledger record for this session here, if there is one.
        key: Option<Key>,
    },
    Worktree {
        repo: String,
        worktree: PathBuf,
        branch: String,
        /// `git status --porcelain` lines: lost if it goes ahead.
        dirty: Vec<String>,
        /// Commits not on the base: kept on the branch.
        unpushed: Vec<String>,
        /// `<remote>/<base>`.
        base: String,
    },
}

/// The tab to close: iTerm's id for the session yo runs in
/// (`ITERM_SESSION_ID`), if it's in one.
fn own_tab(ctx: &Ctx) -> Option<String> {
    ctx.sys.env_nonempty("ITERM_SESSION_ID")
}

pub fn plan_down(ctx: &Ctx, runner: &dyn Runner, cwd: &Path, o: DownOpts) -> Result<(Found, Plan)> {
    let sys = &*ctx.sys;
    let top = git::git(sys, cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let is_worktree = top
        .as_ref()
        .and_then(|t| t.parent())
        .and_then(|p| p.file_name())
        .is_some_and(|n| n.to_string_lossy().ends_with("-worktrees"));
    let mut plan = Plan::default();

    let Some(top) = top.filter(|_| is_worktree) else {
        let dir = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        let key = here_key(ctx, &dir);
        let rec = key.as_ref().and_then(|k| record(ctx, k));
        if let Some(k) = &key {
            plan.actions.push(Action::DropRecord(k.clone()));
        }
        end_and_close(ctx, runner, rec.as_ref(), &mut plan);
        return Ok((Found::InPlace { dir, key }, plan));
    };

    let ledger = ctx.ledger();
    if let Ok(Some(e)) = ledger.get(&top.to_string_lossy())
        && e.record.home
    {
        let agent = e.record.agent.as_deref().unwrap_or("an agent");
        return Err(Error::Refused(format!(
            "this is {agent}'s home checkout, which yo keeps; quit the session with /exit instead"
        )));
    }
    let repo = ctx
        .registry
        .detect_repo(sys, &top)
        .ok_or_else(|| Error::Refused("couldn't identify the repo from its remotes".into()))?;
    let branch =
        git::git(sys, &top, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "?".into());
    let dirty = git::git_lines(sys, &top, &["status", "--porcelain"]).unwrap_or_default();
    let upstream = git::remote_for(
        sys,
        &repo.path,
        ctx.registry.github_for(sys, repo).as_deref(),
    );
    let base = format!("{upstream}/{}", ctx.registry.base_ref(sys, repo));
    let unpushed = git::git_lines(sys, &top, &["log", "--oneline", &format!("{base}..HEAD")])
        .unwrap_or_default();
    if o.yes && !dirty.is_empty() && !o.discard {
        let files: String = dirty.iter().map(|l| format!("\n        {l}")).collect();
        return Err(Error::Refused(format!(
            "refusing: these uncommitted changes would be lost:{files}\n    Commit or stash them, or pass --discard."
        )));
    }
    plan.actions.push(Action::RemoveWorktree {
        repo: repo.path.clone(),
        path: top.clone(),
        // Only when it was asked for (--discard, or yes at the prompt after
        // being shown the changes): git refuses a dirty worktree otherwise.
        force: !dirty.is_empty(),
    });
    plan.actions.push(Action::PruneWorktrees {
        repo: repo.path.clone(),
    });
    let key = Key::for_worktree(&sys.home(), &top);
    let rec = record(ctx, &key);
    plan.actions.push(Action::DropRecord(key));
    end_and_close(ctx, runner, rec.as_ref(), &mut plan);
    Ok((
        Found::Worktree {
            repo: repo.key.clone(),
            worktree: top,
            branch,
            dirty,
            unpushed,
            base,
        },
        plan,
    ))
}

fn record(ctx: &Ctx, key: &Key) -> Option<Record> {
    ctx.ledger()
        .all()
        .into_iter()
        .find(|e| &e.key == key)
        .map(|e| e.record)
}

/// The last steps: ending the session if it's running (the record's, else
/// the one yo runs inside), then closing the tab.
fn end_and_close(ctx: &Ctx, runner: &dyn Runner, rec: Option<&Record>, plan: &mut Plan) {
    let inside = ctx.own_session();
    let id = rec
        .and_then(|r| r.id().map(str::to_string))
        .or(inside.clone());
    let live = id.and_then(|id| ctx.live().into_iter().find(|l| l.id == id));
    // Inside it, yo may be in a tab that shows another member (a workgroup
    // showing CLI, with the chat reached by Remote Control), so the tab is
    // found by what was recorded too.
    let in_it = live
        .as_ref()
        .is_some_and(|l| inside.as_deref() == Some(l.id.as_str()));
    let tab = own_tab(ctx);
    // From outside it, only a session in this very tab is ours to end: its
    // recorded tab is the one yo runs in. A `down` typed in some other shell
    // under the directory mustn't end a session in another tab.
    let here = || {
        let recorded = rec.and_then(Record::tab)?;
        let mine = runner.own_tab(&*ctx.sys, tab.as_deref()?)?;
        Some(recorded == mine)
    };
    // Run inside it with no tab to close (another terminal), it stays: the
    // user ends it, as they would have before.
    let here = if in_it { None } else { here() };
    let ours = if in_it {
        tab.is_some()
    } else {
        here == Some(true)
    };
    match live {
        Some(l) if ours => plan.actions.push(Action::EndSession {
            harness: l.harness,
            id: l.id,
            pid: l.pid,
            after_close: in_it,
        }),
        Some(l) if !in_it => plan.notes.push(Note::Info(match here {
            Some(_) => format!(
                "its {} session (pid {}) isn't in this tab, so it's left running",
                l.harness, l.pid
            ),
            // No recorded tab (started before yo recorded them), or no
            // runner to ask: it can't be shown to be this tab's.
            None => format!(
                "its {} session (pid {}) is left running: yo can't tell it's in this tab",
                l.harness, l.pid
            ),
        })),
        _ => {}
    }
    plan.actions.push(Action::CloseOwnTab {
        session: tab,
        tab: rec
            .filter(|_| in_it)
            .and_then(|r| r.tab().map(str::to_string)),
    });
}

/// The record for the session running in place in `dir`: the one
/// `YO_SESSION_KEY` names, else the one holding this session's id, else
/// the only in-place record holding `dir`. bash's `_yo_here_session_key`.
fn here_key(ctx: &Ctx, dir: &Path) -> Option<Key> {
    if let Some(k) = ctx.sys.env_nonempty("YO_SESSION_KEY") {
        return Some(Key(k));
    }
    let all = ctx.ledger().all();
    let id = ctx.own_session();
    if let Some(e) = all
        .iter()
        .rev()
        .find(|e| id.is_some() && e.record.session_id == id)
    {
        return Some(e.key.clone());
    }
    let here: Vec<&Entry> = all
        .iter()
        .filter(|e| e.record.mode_or_default() == Mode::Here && dir.starts_with(&e.record.worktree))
        .collect();
    match here.as_slice() {
        [one] => Some(one.key.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
