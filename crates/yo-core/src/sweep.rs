//! `yo sweep`: worktrees whose work has landed, minus any a session still
//! needs. bash's `_yo_state sweep`.
//!
//! What's never swept: the primary clone (not listed at all), the current
//! directory, an agent's home, a worktree a live session runs in, and one
//! with uncommitted changes. Removal never uses `--force`, so a worktree
//! that turned dirty after it was classified is kept by git itself.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::harness::LiveSession;
use crate::ledger::{Entry, Key};
use crate::plan::{Action, Note, Plan};
use crate::registry::Repo;
use crate::sys::CommandSpec;
use crate::{Ctx, Error, Result, git};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Landed (a merged PR, no commits beyond base) or a clean PR-review
    /// checkout: removable.
    Dead,
    /// Unmerged work: removable only with `--include-stale`.
    Stale,
    /// A session `resume-all` would bring back: only with
    /// `--include-suspended`.
    Suspended,
    Keep,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Dead => "DEAD",
            Verdict::Stale => "STALE",
            Verdict::Suspended => "SUSP",
            Verdict::Keep => "KEEP",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SweepOpts {
    pub delete_branches: bool,
    pub include_stale: bool,
    pub include_suspended: bool,
    /// Only these repos (empty: all).
    pub repos: Vec<String>,
}

/// One worktree, as `git worktree list --porcelain` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub detached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub worktree: Worktree,
    pub verdict: Verdict,
    pub why: String,
}

/// A repo's worktrees, classified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoReport {
    pub key: String,
    /// `<remote>/<base>`.
    pub base: String,
    pub rows: Vec<Row>,
    /// The clone isn't there.
    pub missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pr {
    number: u64,
    state: String,
}

/// Every worktree but the primary clone.
fn worktrees(ctx: &Ctx, clone: &Path) -> Vec<Worktree> {
    let out = git::git(&*ctx.sys, clone, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    let mut all: Vec<Worktree> = Vec::new();
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            all.push(Worktree {
                path: PathBuf::from(p),
                branch: None,
                detached: false,
            });
        } else if let (Some(b), Some(w)) = (line.strip_prefix("branch "), all.last_mut()) {
            w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        } else if let ("detached", Some(w)) = (line, all.last_mut()) {
            w.detached = true;
        }
    }
    let primary = real(clone);
    all.into_iter()
        .filter(|w| real(&w.path) != primary)
        .collect()
}

fn real(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Branch name -> its most decisive PR, from one listing per repo. A
/// squash merge leaves the merged PR as the only witness the work landed;
/// an open PR on a reused branch outranks anything earlier.
fn pr_index(ctx: &Ctx, owner_repo: &str) -> Option<BTreeMap<String, Pr>> {
    let spec = CommandSpec::new("gh").args([
        "pr",
        "list",
        "--repo",
        owner_repo,
        "--state",
        "all",
        "--limit",
        "800",
        "--json",
        "number,headRefName,state",
    ]);
    let out = ctx.sys.cmd().run(&spec).ok().filter(|o| o.success())?;
    let list: Vec<Value> = serde_json::from_slice(&out.stdout).ok()?;
    let rank = |s: &str| match s {
        "MERGED" => 3,
        "CLOSED" => 2,
        "OPEN" => 1,
        _ => 0,
    };
    let mut idx: BTreeMap<String, Pr> = BTreeMap::new();
    for pr in list {
        let (Some(head), Some(number), Some(state)) = (
            pr.get("headRefName").and_then(Value::as_str),
            pr.get("number").and_then(Value::as_u64),
            pr.get("state").and_then(Value::as_str),
        ) else {
            continue;
        };
        let replace = match idx.get(head) {
            None => true,
            Some(prev) => prev.state != "OPEN" && rank(state) > rank(&prev.state),
        };
        if replace {
            idx.insert(
                head.to_string(),
                Pr {
                    number,
                    state: state.to_string(),
                },
            );
        }
    }
    Some(idx)
}

struct Ask<'a> {
    ctx: &'a Ctx,
    clone: &'a Path,
    base: &'a str,
    here: PathBuf,
    entries: &'a [Entry],
    live: &'a [LiveSession],
    prs: Option<&'a BTreeMap<String, Pr>>,
}

/// A session that still needs the worktree: an agent's home, one running
/// in it, or one `resume-all` would bring back. One ended with `/exit`
/// holds nothing.
fn held(a: &Ask<'_>, path: &Path) -> Option<(Verdict, String)> {
    let rp = real(path);
    let at = |e: &&Entry| real(&e.record.worktree) == rp;
    if let Some(e) = a.entries.iter().filter(at).find(|e| e.record.home) {
        let ag = e.record.agent.as_deref().unwrap_or("");
        return Some((Verdict::Keep, format!("{ag}'s home (yo agent {ag})")));
    }
    if let Some(l) = a.live.iter().find(|l| real(&l.cwd).starts_with(&rp)) {
        return Some((Verdict::Keep, format!("live session, pid {}", l.pid)));
    }
    if let Some(e) = a.entries.iter().filter(at).find(|e| {
        e.record
            .session_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
            && !e.record.ended_at.is_some_and(|t| t != 0)
    }) {
        return Some((
            Verdict::Suspended,
            format!(
                "suspended session (yo resume {})",
                e.record.name.as_deref().unwrap_or("")
            ),
        ));
    }
    None
}

fn classify(a: &Ask<'_>, w: &Worktree) -> (Verdict, String) {
    let sys = &*a.ctx.sys;
    let rp = real(&w.path);
    if a.here == rp || a.here.starts_with(&rp) {
        return (Verdict::Keep, "current directory".into());
    }
    if !w.path.is_dir() {
        return (Verdict::Dead, "directory is gone (prunable)".into());
    }
    if let Some(h) = held(a, &w.path) {
        return h;
    }
    // Unreadable status is treated as dirty: keep it.
    let Some(status) = git::git_lines(sys, &w.path, &["status", "--porcelain"]) else {
        return (Verdict::Keep, "status unreadable".into());
    };
    let n_dirty = status.len();
    let Some(branch) = w.branch.as_deref().filter(|_| !w.detached) else {
        // A PR-review checkout at the PR's head with no branch: nothing is
        // lost by removing it.
        return if n_dirty > 0 {
            (Verdict::Keep, format!("detached, {n_dirty} uncommitted"))
        } else {
            (Verdict::Dead, "detached PR-review checkout".into())
        };
    };
    let Some(ahead) = git::git(
        sys,
        a.clone,
        &["rev-list", "--count", &format!("{}..{branch}", a.base)],
    )
    .and_then(|n| n.parse::<u64>().ok()) else {
        return (Verdict::Keep, "branch unreadable".into());
    };
    let pr = a.prs.and_then(|p| p.get(branch));
    let prtxt = pr.map_or("no PR".to_string(), |p| {
        format!("PR #{} {}", p.number, p.state.to_lowercase())
    });
    if n_dirty > 0 {
        return (Verdict::Keep, format!("{n_dirty} uncommitted ({prtxt})"));
    }
    match pr.map(|p| p.state.as_str()) {
        Some("OPEN") => (Verdict::Keep, format!("{prtxt}, {ahead} commit(s)")),
        Some("MERGED") => (Verdict::Dead, format!("{prtxt} (squash-merged)")),
        _ if ahead == 0 => (
            Verdict::Dead,
            format!("no commits beyond {} ({prtxt})", a.base),
        ),
        Some("CLOSED") => (
            Verdict::Stale,
            format!("{prtxt} unmerged, {ahead} commit(s) would be orphaned"),
        ),
        _ => (
            Verdict::Stale,
            format!("{ahead} commit(s), no PR — unreviewed work"),
        ),
    }
}

/// Classifies every repo's worktrees, and plans removing the removable
/// ones. Fetches each repo's base remote first, as bash did, so "merged"
/// means merged now; that's the one thing it changes when it only lists.
pub fn plan_sweep(
    ctx: &Ctx,
    cwd: &Path,
    o: &SweepOpts,
) -> Result<(Vec<RepoReport>, Vec<Note>, Plan)> {
    let reg = &ctx.registry;
    let sys = &*ctx.sys;
    let unknown: Vec<&str> = o
        .repos
        .iter()
        .filter(|k| reg.repo(k).is_none())
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(Error::Refused(format!(
            "unknown repo(s): {}",
            unknown.join(", ")
        )));
    }
    let repos: Vec<&Repo> = reg
        .repos()
        .iter()
        .filter(|r| o.repos.is_empty() || o.repos.contains(&r.key))
        .collect();
    let entries = ctx.ledger().all();
    let live = ctx.live();
    let here = real(cwd);
    let mut notes = Vec::new();
    let mut reports = Vec::new();
    let mut plan = Plan::default();
    for r in repos {
        let gh = reg.github_for(sys, r);
        if gh.is_none() {
            notes.push(Note::Warn(format!(
                "no owner/repo for '{}' — its PRs can't be checked, so merged branches read as unmerged",
                r.key
            )));
        }
        let remote = git::remote_for(sys, &r.path, gh.as_deref());
        let base = format!("{remote}/{}", reg.base_ref(sys, r));
        if !r.path.is_dir() {
            reports.push(RepoReport {
                key: r.key.clone(),
                base,
                rows: Vec::new(),
                missing: true,
            });
            continue;
        }
        let wts = worktrees(ctx, &r.path);
        if wts.is_empty() {
            continue;
        }
        let _ = git::git(sys, &r.path, &["fetch", "--quiet", &remote]);
        let prs = gh.as_deref().and_then(|g| {
            let idx = pr_index(ctx, g);
            if idx.is_none() {
                notes.push(Note::Warn(format!("could not list PRs for {g}")));
            }
            idx
        });
        let ask = Ask {
            ctx,
            clone: &r.path,
            base: &base,
            here: here.clone(),
            entries: &entries,
            live: &live,
            prs: prs.as_ref(),
        };
        let mut rows: Vec<Row> = wts
            .into_iter()
            .map(|w| {
                let (verdict, why) = classify(&ask, &w);
                Row {
                    worktree: w,
                    verdict,
                    why,
                }
            })
            .collect();
        let mut any = false;
        for row in &rows {
            let take = match row.verdict {
                Verdict::Dead => true,
                Verdict::Stale => o.include_stale,
                Verdict::Suspended => o.include_suspended,
                Verdict::Keep => false,
            };
            if take {
                any = true;
                plan.actions.push(Action::SweepWorktree {
                    repo: r.path.clone(),
                    path: row.worktree.path.clone(),
                    key: Key::for_worktree(&sys.home(), &row.worktree.path),
                    delete_branch: row.worktree.branch.clone().filter(|_| o.delete_branches),
                });
            }
        }
        // Shown by verdict; removed in git's order, as listed.
        rows.sort_by_key(|row| row.verdict);
        if any {
            plan.actions.push(Action::PruneWorktrees {
                repo: r.path.clone(),
            });
        }
        reports.push(RepoReport {
            key: r.key.clone(),
            base,
            rows,
            missing: false,
        });
    }
    Ok((reports, notes, plan))
}

#[cfg(test)]
mod tests;
