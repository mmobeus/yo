//! Plans: what a command will do, before it does it. A command that changes
//! anything builds a [`Plan`] first; `--dry-run` shows it, tests assert on
//! it, and [`execute`] carries it out.

use std::fs;
use std::path::{Path, PathBuf};

use crate::harness::Prep;
use crate::ledger::{Entry, Key};
use crate::runner::{Focused, LaunchSpec, Launched, Runner};
use crate::{Ctx, Error, Result, git};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// What the caller should know, found while planning.
    pub notes: Vec<Note>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// A step, or something found.
    Info(String),
    /// A detail of the step before.
    Detail(String),
    Warn(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// `git fetch <remote> <refspec>` in the clone; `what` says it in words.
    Fetch {
        repo: PathBuf,
        remote: String,
        refspec: String,
        what: String,
    },
    AddWorktree {
        repo: PathBuf,
        path: PathBuf,
        branch: BranchSpec,
    },
    /// `branch.<b>.remote` and `branch.<b>.merge`, so `git pull` follows it.
    SetUpstream {
        repo: PathBuf,
        branch: String,
        remote: String,
        merge: String,
    },
    /// Gitignored files from the main clone that a worktree needs (`mirror`):
    /// each copied only if it's there, not in the worktree yet, and ignored
    /// by the worktree's git, so it can't be committed.
    MirrorFiles {
        from: PathBuf,
        to: PathBuf,
        files: Vec<String>,
    },
    /// A directory a session uses (its memory). `what` says which.
    MakeDir {
        path: PathBuf,
        what: String,
    },
    /// A file yo composes for a session (its settings layer).
    WriteFile {
        path: PathBuf,
        contents: String,
    },
    /// A harness's own setup.
    Prepare(Prep),
    PutRecord(Box<Entry>),
    DropRecord(Key),
    /// `git worktree remove`; `force` only when losing uncommitted work was
    /// asked for.
    RemoveWorktree {
        repo: PathBuf,
        path: PathBuf,
        force: bool,
    },
    PruneWorktrees {
        repo: PathBuf,
    },
    /// Sweep's removal of one worktree: never forced, so git keeps one that
    /// turned dirty since it was classified; then its record goes, and its
    /// branch with `delete_branch`.
    SweepWorktree {
        repo: PathBuf,
        path: PathBuf,
        key: Key,
        delete_branch: Option<String>,
    },
    /// Moves an agent's home to the newest `<remote>/<base>`, unless it has
    /// uncommitted changes (then it stays where it is).
    RefreshHome {
        path: PathBuf,
        remote: String,
        base: String,
    },
    /// Closes the tab yo runs in (iTerm's `session` id), which ends the
    /// session in it. `None`: not in one.
    CloseOwnTab {
        session: Option<String>,
    },
    Launch(LaunchSpec),
    /// A running session (process `pid`, on terminal `tty` if it has one):
    /// its tab brought forward, and `message` typed in.
    Focus {
        pid: u32,
        tty: Option<String>,
        /// What the progress calls it.
        label: String,
        /// Its Remote Control name, for when there's no tab to reach.
        remote_control: Option<String>,
        message: Option<String>,
    },
}

/// The branch a new worktree checks out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchSpec {
    /// A local branch, as it is.
    Existing(String),
    /// A new local branch tracking `start` (a remote branch).
    Track { branch: String, start: String },
    /// A new branch from `start`.
    New { branch: String, start: String },
    /// No branch: a detached checkout of `start` (an agent's home).
    Detached(String),
}

impl BranchSpec {
    pub fn name(&self) -> &str {
        match self {
            BranchSpec::Existing(b)
            | BranchSpec::Track { branch: b, .. }
            | BranchSpec::New { branch: b, .. } => b,
            BranchSpec::Detached(_) => "",
        }
    }
}

/// Where a command's progress goes. The CLI prints it; the core never does.
pub trait Report {
    fn note(&mut self, note: &Note);
    /// An action is about to run.
    fn doing(&mut self, action: &Action);
    fn launched(&mut self, spec: &LaunchSpec, launched: &Launched);
}

/// Runs `plan`'s actions in order, stopping at the first that fails. A
/// ledger record that can't be written is only a warning: the session still
/// runs, and only `resume-all` loses it.
pub fn execute(ctx: &Ctx, plan: &Plan, runner: &dyn Runner, out: &mut dyn Report) -> Result<()> {
    let sys = &*ctx.sys;
    for n in &plan.notes {
        out.note(n);
    }
    for a in &plan.actions {
        out.doing(a);
        match a {
            Action::Fetch {
                repo,
                remote,
                refspec,
                ..
            } => git::git_do(sys, repo, &["fetch", remote, refspec])?,
            Action::AddWorktree { repo, path, branch } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }
                let p = path.to_string_lossy();
                match branch {
                    BranchSpec::Existing(b) => git::git_do(sys, repo, &["worktree", "add", &p, b])?,
                    BranchSpec::Track { branch, start } => git::git_do(
                        sys,
                        repo,
                        &["worktree", "add", "--track", "-b", branch, &p, start],
                    )?,
                    BranchSpec::New { branch, start } => {
                        git::git_do(sys, repo, &["worktree", "add", "-b", branch, &p, start])?;
                    }
                    BranchSpec::Detached(start) => {
                        git::git_do(sys, repo, &["worktree", "add", "--quiet", "--detach", &p, start])?;
                    }
                }
            }
            Action::SetUpstream {
                repo,
                branch,
                remote,
                merge,
            } => {
                git::git_do(
                    sys,
                    repo,
                    &["config", &format!("branch.{branch}.remote"), remote],
                )?;
                git::git_do(
                    sys,
                    repo,
                    &["config", &format!("branch.{branch}.merge"), merge],
                )?;
            }
            Action::MirrorFiles { from, to, files } => {
                for n in mirror(sys, from, to, files) {
                    out.note(&n);
                }
            }
            Action::MakeDir { path, .. } => {
                fs::create_dir_all(path).map_err(|e| Error::io(path, e))?;
            }
            Action::WriteFile { path, contents } => {
                crate::write_atomic(path, contents.as_bytes())?;
            }
            Action::Prepare(p) => {
                if let Err(err) = p.run(sys) {
                    out.note(&Note::Warn(format!("{} ({err})", p.failed())));
                }
            }
            Action::PutRecord(e) => {
                if let Err(err) = ctx.ledger().write(&mut e.clone()) {
                    out.note(&Note::Warn(format!(
                        "couldn't record the session ({err}); resume-all won't know about it"
                    )));
                }
            }
            Action::DropRecord(k) => {
                let _ = ctx.ledger().drop(k);
            }
            Action::RemoveWorktree { repo, path, force } => {
                let p = path.to_string_lossy();
                let mut args = vec!["worktree", "remove"];
                if *force {
                    args.push("--force");
                }
                args.push(&p);
                git::git_do(sys, repo, &args)?;
            }
            Action::PruneWorktrees { repo } => {
                let _ = git::git(sys, repo, &["worktree", "prune"]);
            }
            Action::SweepWorktree {
                repo,
                path,
                key,
                delete_branch,
            } => {
                let name = path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let p = path.to_string_lossy();
                if let Err(e) = git::git_do(sys, repo, &["worktree", "remove", &p])
                    && path.is_dir()
                {
                    out.note(&Note::Warn(format!("kept {name}: {e}")));
                    continue;
                }
                let _ = ctx.ledger().drop(key);
                out.note(&Note::Info(format!("removed {name}")));
                if let Some(b) = delete_branch {
                    // -D, not -d: a squash-merged branch isn't an ancestor of
                    // base, so -d refuses it. The reflog still has its tip.
                    if git::git(sys, repo, &["branch", "-D", b]).is_some() {
                        out.note(&Note::Detail(format!("deleted branch {b}")));
                    }
                }
            }
            Action::RefreshHome { path, remote, base } => {
                let at = format!("{remote}/{base}");
                let dirty = git::git_lines(sys, path, &["status", "--porcelain"])
                    .is_some_and(|l| !l.is_empty());
                if dirty {
                    out.note(&Note::Info(format!(
                        "note: {} has uncommitted changes; left at its current commit",
                        path.display()
                    )));
                } else {
                    let _ = git::git(sys, path, &["fetch", "--quiet", remote, base]);
                    if git::git(sys, path, &["checkout", "--quiet", "--detach", &at]).is_some() {
                        let sha = git::git(sys, path, &["rev-parse", "--short", "HEAD"])
                            .unwrap_or_default();
                        out.note(&Note::Info(format!("home at {at} ({sha})")));
                    } else {
                        out.note(&Note::Info(format!(
                            "note: couldn't move {} to {at}; left at its current commit",
                            path.display()
                        )));
                    }
                }
            }
            Action::CloseOwnTab { session } => match session {
                None => out.note(&Note::Warn(
                    "not running inside an iTerm2 session; leaving windows alone (exit claude yourself)"
                        .into(),
                )),
                Some(s) => {
                    if !runner.close_own_tab(sys, s) {
                        out.note(&Note::Info(format!("dry close: the tab of {s}")));
                    }
                }
            },
            Action::Launch(spec) => {
                let l = runner.launch(sys, spec)?;
                out.launched(spec, &l);
            }
            Action::Focus {
                pid,
                tty,
                label,
                remote_control,
                message,
            } => {
                let got = match tty {
                    Some(t) => runner.focus(sys, t, message.as_deref())?,
                    None => Focused::NotFound,
                };
                let typed = if message.is_some() {
                    " and typed your message"
                } else {
                    ""
                };
                match got {
                    Focused::Yes => out.note(&Note::Info(format!(
                        "{label} is running (pid {pid}): brought its tab forward{typed}"
                    ))),
                    Focused::Dry { tty } => out.note(&Note::Info(format!(
                        "dry focus: {tty}{}",
                        message
                            .as_deref()
                            .map(|m| format!(", then type: {m}"))
                            .unwrap_or_default()
                    ))),
                    Focused::NotFound => {
                        let rc = remote_control
                            .as_deref()
                            .map(|r| format!("by Remote Control as {r}, or "))
                            .unwrap_or_default();
                        let lost = if message.is_some() {
                            " (your message was not delivered)"
                        } else {
                            ""
                        };
                        out.note(&Note::Info(format!(
                            "{label} is running (pid {pid}), but not in an iTerm tab yo can find;"
                        )));
                        out.note(&Note::Detail(format!(
                            "reach it {rc}with SendMessage from another session{lost}"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Copies each of `files` from the main clone into the worktree, saying
/// what it did with each.
fn mirror(sys: &dyn crate::sys::Sys, from: &Path, to: &Path, files: &[String]) -> Vec<Note> {
    let mut notes = Vec::new();
    for rel in files {
        let src = from.join(rel);
        let dst = to.join(rel);
        if !src.is_file() {
            notes.push(Note::Warn(format!(
                "no {rel} in {} -- skipped (servers that read it won't start)",
                from.display()
            )));
        } else if dst.exists() {
            notes.push(Note::Info(format!(
                "{rel} already in the worktree; left alone"
            )));
        } else if git::git(sys, to, &["check-ignore", "-q", rel]).is_none() {
            // These hold credentials: never where `git add -A` could stage one.
            notes.push(Note::Warn(format!(
                "{rel} is NOT gitignored in the worktree -- not copying (it could be committed)"
            )));
        } else if dst.parent().is_some_and(|d| fs::create_dir_all(d).is_err())
            || fs::copy(&src, &dst).is_err()
        {
            notes.push(Note::Warn(format!("couldn't copy {rel} into the worktree")));
        } else {
            // fs::copy keeps the mode, as `cp -p` did.
            notes.push(Note::Info(format!(
                "{rel} copied from the main clone (gitignored; goes away with the worktree)"
            )));
        }
    }
    notes
}
