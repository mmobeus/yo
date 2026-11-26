//! Plans: what a command will do, before it does it. A command that changes
//! anything builds a [`Plan`] first; `--dry-run` shows it, tests assert on
//! it, and [`execute`] carries it out.

use std::fs;
use std::path::PathBuf;

use crate::ledger::Entry;
use crate::runner::{LaunchSpec, Launched, Runner};
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
    PutRecord(Box<Entry>),
    Launch(LaunchSpec),
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
}

impl BranchSpec {
    pub fn name(&self) -> &str {
        match self {
            BranchSpec::Existing(b)
            | BranchSpec::Track { branch: b, .. }
            | BranchSpec::New { branch: b, .. } => b,
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
            Action::PutRecord(e) => {
                if let Err(err) = ctx.ledger().write(&mut e.clone()) {
                    out.note(&Note::Warn(format!(
                        "couldn't record the session ({err}); resume-all won't know about it"
                    )));
                }
            }
            Action::Launch(spec) => {
                let l = runner.launch(sys, spec)?;
                out.launched(spec, &l);
            }
        }
    }
    Ok(())
}
