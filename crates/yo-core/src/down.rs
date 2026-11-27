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
//!
//! `yo down NAME` ([`plan_down_named`]) tears down another session, from
//! outside it, by its record: it ends the session if it's running, removes
//! its worktree as above, drops its record, and closes its recorded tab
//! (every member of it, a workgroup's CLI too). A session still working
//! is asked about, and refused with `--yes`. A recorded tab is closed only
//! once yo confirms it's still that session's: tab ids are a window's id
//! and a small number, so after iTerm restarts, an old record's tab may be
//! someone else's. [`children`] finds the sessions this one started, for
//! `yo down --children`.

use std::path::{Path, PathBuf};

use crate::harness::LiveSession;
use crate::ledger::{Entry, Key, Record};
use crate::plan::{Action, Note, Plan};
use crate::registry::Mode;
use crate::runner::{Runner, tty_of};
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
    let is_worktree = top.as_deref().is_some_and(is_yo_worktree);
    let mut plan = Plan::default();

    let Some(top) = top.filter(|_| is_worktree) else {
        let dir = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        let key = here_key(ctx, runner, &dir)?;
        let rec = key.as_ref().and_then(|k| record(ctx, k));
        if let Some(k) = &key {
            plan.actions.push(Action::DropRecord(k.clone()));
        }
        end_and_close(ctx, runner, rec.as_ref(), &mut plan);
        return Ok((Found::InPlace { dir, key }, plan));
    };

    let found = remove_worktree(ctx, &top, o, &mut plan)?;
    let key = Key::for_worktree(&sys.home(), &top);
    let rec = record(ctx, &key);
    plan.actions.push(Action::DropRecord(key));
    end_and_close(ctx, runner, rec.as_ref(), &mut plan);
    Ok((found, plan))
}

/// Whether `top` is a yo worktree: under a `<repo>-worktrees` directory.
fn is_yo_worktree(top: &Path) -> bool {
    top.parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n.to_string_lossy().ends_with("-worktrees"))
}

/// Plans removing yo worktree `top` (its branch kept), after its refusals.
fn remove_worktree(ctx: &Ctx, top: &Path, o: DownOpts, plan: &mut Plan) -> Result<Found> {
    let sys = &*ctx.sys;
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
        .detect_repo(sys, top)
        .ok_or_else(|| Error::Refused("couldn't identify the repo from its remotes".into()))?;
    let branch =
        git::git(sys, top, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "?".into());
    let dirty = git::git_lines(sys, top, &["status", "--porcelain"]).unwrap_or_default();
    let upstream = git::remote_for(
        sys,
        &repo.path,
        ctx.registry.github_for(sys, repo).as_deref(),
    );
    let base = format!("{upstream}/{}", ctx.registry.base_ref(sys, repo));
    let unpushed = git::git_lines(sys, top, &["log", "--oneline", &format!("{base}..HEAD")])
        .unwrap_or_default();
    if o.yes && !dirty.is_empty() && !o.discard {
        let files: String = dirty.iter().map(|l| format!("\n        {l}")).collect();
        return Err(Error::Refused(format!(
            "refusing: these uncommitted changes would be lost:{files}\n    Commit or stash them, or pass --discard."
        )));
    }
    plan.actions.push(Action::RemoveWorktree {
        repo: repo.path.clone(),
        path: top.to_path_buf(),
        // Only when it was asked for (--discard, or yes at the prompt after
        // being shown the changes): git refuses a dirty worktree otherwise.
        force: !dirty.is_empty(),
    });
    plan.actions.push(Action::PruneWorktrees {
        repo: repo.path.clone(),
    });
    Ok(Found::Worktree {
        repo: repo.key.clone(),
        worktree: top.to_path_buf(),
        branch,
        dirty,
        unpushed,
        base,
    })
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
/// the in-place record holding `dir` whose tab is the one yo runs in (the
/// tab's CLI, which has neither), else the only in-place record holding
/// `dir`. bash's `_yo_here_session_key`, plus the tab.
///
/// Several there and none of them picked is refused: closing the tab would
/// leave its session running with no tab and its record kept (#52).
fn here_key(ctx: &Ctx, runner: &dyn Runner, dir: &Path) -> Result<Option<Key>> {
    if let Some(k) = ctx.sys.env_nonempty("YO_SESSION_KEY") {
        return Ok(Some(Key(k)));
    }
    let all = ctx.ledger().all();
    let id = ctx.own_session();
    if let Some(e) = all
        .iter()
        .rev()
        .find(|e| id.is_some() && e.record.session_id == id)
    {
        return Ok(Some(e.key.clone()));
    }
    let here: Vec<&Entry> = all
        .iter()
        .filter(|e| e.record.mode_or_default() == Mode::Here && dir.starts_with(&e.record.worktree))
        .collect();
    if let [one] = here.as_slice() {
        return Ok(Some(one.key.clone()));
    }
    if here.is_empty() {
        return Ok(None);
    }
    // Only asked with more than one to pick from: it's a round trip to iTerm.
    let mine = own_tab(ctx).and_then(|s| runner.own_tab(&*ctx.sys, &s));
    if let Some(e) = mine
        .as_deref()
        .and_then(|t| here.iter().find(|e| e.record.tab() == Some(t)))
    {
        return Ok(Some(e.key.clone()));
    }
    let names: String = here
        .iter()
        .map(|e| {
            let name = e.record.name.as_deref().unwrap_or(&e.key.0);
            format!("\n        {name}")
        })
        .collect();
    Err(Error::Refused(format!(
        "refusing: yo can't tell which of the sessions in place here is this one:{names}\n    Run yo down from the session's own tab, or yo forget the one to drop."
    )))
}

/// What a session is doing, for `yo down NAME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    NotRunning,
    /// Running, waiting for a prompt.
    Idle,
    /// Running, and its harness says it's working.
    Busy,
    /// Running, and its harness doesn't say whether it's working (Codex).
    Unknown,
}

impl Activity {
    /// Whether it may still be working: asked about, and refused with
    /// `--yes`.
    pub fn working(self) -> bool {
        matches!(self, Activity::Busy | Activity::Unknown)
    }
}

/// What happens to another session's tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OtherTab {
    /// Closed: yo confirmed it's this session's.
    Close(String),
    /// No tab recorded (started before yo recorded them, or no runner).
    NoneRecorded,
    /// Its recorded tab isn't open.
    NotOpen,
    /// A tab with its recorded id is open, but nothing in it shows this
    /// session's title or terminal: left open.
    Unconfirmed(String),
    /// The runner couldn't say what it shows: left open.
    CantTell,
}

/// What `down NAME` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Other {
    pub name: String,
    pub found: Found,
    pub activity: Activity,
    pub tab: OtherTab,
}

/// What `down NAME` would do to session `e`, from outside it: end it if
/// it's running, remove its worktree (if it's a yo worktree still there)
/// and drop its record, then close its tab, in that order, so the tab
/// closes last even if it's the one yo runs in. With `yes`, a session that
/// may still be working is refused, like uncommitted work.
pub fn plan_down_named(
    ctx: &Ctx,
    runner: &dyn Runner,
    e: &Entry,
    o: DownOpts,
) -> Result<(Other, Plan)> {
    let sys = &*ctx.sys;
    let rec = &e.record;
    let name = rec.name.clone().unwrap_or_else(|| e.key.0.clone());
    if rec.home {
        let agent = rec.agent.as_deref().unwrap_or("an agent");
        return Err(Error::Refused(format!(
            "{name} is {agent}'s home, which yo keeps; quit its session with /exit instead"
        )));
    }
    if rec.id().is_some() && rec.id() == ctx.own_session().as_deref() {
        return Err(Error::Refused(format!(
            "{name} is this session: run yo down without a name"
        )));
    }
    let Some((live, activity)) = running(ctx, rec) else {
        return Err(Error::Refused(format!(
            "{name} is this session (it runs in that worktree): run yo down without a name"
        )));
    };
    if o.yes && activity.working() {
        return Err(Error::Refused(match activity {
            Activity::Busy => format!("refusing: {name} is still working"),
            _ => format!(
                "refusing: yo can't tell whether {name} is still working; without --yes, yo asks"
            ),
        }));
    }
    // The tab is looked at before anything changes: once its session has
    // ended, a hidden chat can't be told from a stale tab.
    let tab = confirm_tab(ctx, runner, rec, live.as_ref());

    let mut plan = Plan::default();
    if let Some(l) = live {
        plan.actions.push(Action::EndSession {
            harness: l.harness,
            id: l.id,
            pid: l.pid,
            after_close: false,
        });
    }
    // Only a yo worktree whose checkout is still the recorded directory is
    // removed; anything else on disk is left as it is.
    let wt = &rec.worktree;
    let top = (rec.mode_or_default() == Mode::Worktree && is_yo_worktree(wt) && wt.is_dir())
        .then(|| git::git(sys, wt, &["rev-parse", "--show-toplevel"]).map(PathBuf::from))
        .flatten()
        .filter(|t| t == wt);
    let found = match top {
        Some(top) => remove_worktree(ctx, &top, o, &mut plan)?,
        None => Found::InPlace {
            dir: wt.clone(),
            key: Some(e.key.clone()),
        },
    };
    plan.actions.push(Action::DropRecord(e.key.clone()));
    if let OtherTab::Close(t) = &tab {
        plan.actions.push(Action::CloseTab { tab: t.clone() });
    }
    Ok((
        Other {
            name,
            found,
            activity,
            tab,
        },
        plan,
    ))
}

/// The session running for `rec`, and what it's doing. By its id; else,
/// for a worktree, any session running in it (as sweep keeps a worktree a
/// live session holds), whose id yo can't match, so it may be working;
/// else, with no id yet from a harness that reports its own later (Codex,
/// before its first message), yo can't tell. A session in place is never
/// found by directory: others run in the same checkout. Nor is the one yo
/// runs in: `None` when it's the only session in the worktree.
fn running(ctx: &Ctx, rec: &Record) -> Option<(Option<LiveSession>, Activity)> {
    let live = ctx.live();
    let own = ctx.own_session();
    if let Some(l) = rec.id().and_then(|id| live.iter().find(|l| l.id == id)) {
        let a = match l.idle {
            Some(true) => Activity::Idle,
            Some(false) => Activity::Busy,
            None => Activity::Unknown,
        };
        return Some((Some(l.clone()), a));
    }
    if rec.mode_or_default() == Mode::Worktree {
        let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let wt = real(&rec.worktree);
        let (mine, others): (Vec<&LiveSession>, Vec<&LiveSession>) = live
            .iter()
            .filter(|l| !l.cwd.as_os_str().is_empty() && real(&l.cwd).starts_with(&wt))
            .partition(|l| own.as_deref() == Some(l.id.as_str()));
        if let Some(l) = others.first() {
            return Some((Some((*l).clone()), Activity::Unknown));
        }
        if !mine.is_empty() {
            return None;
        }
    }
    let reports = crate::harness::all().iter().any(|h| {
        h.name() == rec.harness.as_deref().unwrap_or(crate::harness::DEFAULT)
            && h.ids() == crate::harness::IdPolicy::Reported
    });
    if rec.id().is_none() && reports {
        return Some((None, Activity::Unknown));
    }
    Some((None, Activity::NotRunning))
}

/// Whether `rec`'s recorded tab is open and still its own: something in it
/// shows its title or its terminal, or it's running with its terminal shown
/// nowhere (hidden in that tab: a workgroup showing CLI).
fn confirm_tab(
    ctx: &Ctx,
    runner: &dyn Runner,
    rec: &Record,
    live: Option<&LiveSession>,
) -> OtherTab {
    let Some(tab) = rec.tab() else {
        return OtherTab::NoneRecorded;
    };
    let Some(shown) = runner.tabs(&*ctx.sys) else {
        return OtherTab::CantTell;
    };
    if !shown.tabs.iter().any(|t| t == tab) {
        return OtherTab::NotOpen;
    }
    let tty = live.and_then(|l| tty_of(&*ctx.sys, l.pid));
    let title = rec.title.as_deref().filter(|t| !t.is_empty());
    let shows_it = shown.panes.iter().any(|p| {
        p.tab == tab && (title == Some(p.title.as_str()) || tty.as_deref() == Some(p.tty.as_str()))
    });
    // Hidden, only where it could be: its terminal is shown nowhere, and its
    // tab has a pane that could be its peer (no yo title, as a workgroup's
    // CLI has none, or its own). Running outside iTerm, its old tab id may
    // be another session's, showing another title.
    let peer = shown
        .panes
        .iter()
        .any(|p| p.tab == tab && (p.title.is_empty() || title == Some(p.title.as_str())));
    let hidden = peer && tty.as_ref().is_some_and(|t| !shown.ttys.contains(t));
    if shows_it || hidden {
        OtherTab::Close(tab.to_string())
    } else {
        OtherTab::Unconfirmed(tab.to_string())
    }
}

/// Why `--children` leaves a child alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    /// It may still be working (busy, or yo can't tell).
    Working,
    /// Uncommitted work, and no `--discard`.
    Uncommitted(Vec<String>),
    /// `down` refuses it (an agent's home, an unknown repo).
    Refused(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub name: String,
    pub why: Skip,
}

/// What `down --children` found.
#[derive(Clone, Debug, Default)]
pub struct Children {
    /// Each child to tear down, with its plan.
    pub go: Vec<(Other, Plan)>,
    pub skipped: Vec<Skipped>,
}

/// `down --children`: the sessions this one started ([`children`]), each
/// planned as `down NAME` would, but those that may still be working, or
/// would lose uncommitted work without `discard`, or that `down` refuses,
/// are skipped rather than asked about one by one. Any other error (yo
/// couldn't read its own state) stops it, before anything has changed:
/// planning changes nothing, and a broken ledger isn't a child to skip.
pub fn plan_down_children(ctx: &Ctx, runner: &dyn Runner, discard: bool) -> Result<Children> {
    let mut c = Children::default();
    for e in children(ctx)? {
        let name = e.record.name.clone().unwrap_or_else(|| e.key.0.clone());
        let why = match plan_down_named(ctx, runner, &e, DownOpts::default()) {
            Ok((o, _)) if o.activity.working() => Skip::Working,
            Ok((
                Other {
                    found: Found::Worktree { dirty, .. },
                    ..
                },
                _,
            )) if !dirty.is_empty() && !discard => Skip::Uncommitted(dirty),
            Ok(p) => {
                c.go.push(p);
                continue;
            }
            Err(Error::Refused(m)) => Skip::Refused(m),
            Err(err) => return Err(err),
        };
        c.skipped.push(Skipped { name, why });
    }
    Ok(c)
}

/// The sessions this one started ([`crate::session::started_from`]).
/// Refused outside a session.
pub fn children(ctx: &Ctx) -> Result<Vec<Entry>> {
    let Some(id) = ctx.own_session() else {
        return Err(Error::Refused(
            "not inside a session: --children finds the sessions this one started".into(),
        ));
    };
    Ok(crate::session::started_from(ctx, &id))
}

#[cfg(test)]
mod tests;
