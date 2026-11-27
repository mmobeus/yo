//! An agent's home: one long-lived session per agent, in a detached
//! checkout of its repo's base branch at `<clone>-worktrees/agent-<name>`,
//! run as the agent and reachable by Remote Control as `<name>-home`. Each
//! start moves a clean home to the newest base, so it reads current docs.
//! bash's `cmd_agent`.

use std::path::{Path, PathBuf};

use crate::harness::{IdPolicy, PrepareAsk, SessionSpec};
use crate::ledger::{Entry, Key, Record, State, state_of};
use crate::plan::{Action, BranchSpec, Note, Plan};
use crate::reach::{ResumeOpts, plan_reach};
use crate::registry::Mode;
use crate::runner::{LaunchSpec, RunnerKind};
use crate::{Ctx, Error, Result, git, naming};

/// A registered agent, for `yo agent` with no name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub repo: String,
    /// Its home, or `None` when its repo isn't registered.
    pub home: Option<PathBuf>,
}

pub fn list(ctx: &Ctx) -> Vec<Listed> {
    ctx.registry
        .agents()
        .iter()
        .map(|a| Listed {
            name: a.name.clone(),
            repo: a.repo.clone(),
            home: ctx
                .registry
                .repo(&a.repo)
                .map(|r| home_dir(&r.path, &a.name)),
        })
        .collect()
}

fn home_dir(clone: &Path, name: &str) -> PathBuf {
    let mut root = clone.as_os_str().to_owned();
    root.push("-worktrees");
    PathBuf::from(root).join(format!("agent-{name}"))
}

/// Starts or reaches agent `name`'s home, with `o.message` as its next
/// prompt.
pub fn plan_home(ctx: &Ctx, name: &str, o: &ResumeOpts) -> Result<Plan> {
    let reg = &ctx.registry;
    let sys = &*ctx.sys;
    let agent = reg.agent(name).ok_or_else(|| {
        Error::Refused(format!(
            "unknown agent '{name}': register it under \"agents\" in {}",
            ctx.paths.config.display()
        ))
    })?;
    let repo = reg.repo(&agent.repo).ok_or_else(|| {
        let known: Vec<&str> = reg.repos().iter().map(|r| r.key.as_str()).collect();
        Error::Refused(format!(
            "agent '{name}' names unknown repo '{}' (known: {})",
            agent.repo,
            known.join(" | ")
        ))
    })?;
    if !repo.path.join(".git").exists() {
        return Err(Error::Refused(format!(
            "no clone at: {}",
            repo.path.display()
        )));
    }
    let base = reg.base_ref(sys, repo);
    let remote = git::remote_for(sys, &repo.path, reg.github_for(sys, repo).as_deref());
    let start = format!("{remote}/{base}");
    let wt = home_dir(&repo.path, name);
    let display = naming::agent_display(name, agent.title.as_deref());
    let title = format!("{display} (home)");
    let rc = format!("{name}-home");
    let label = format!("{display}'s home");
    let refresh = Action::RefreshHome {
        path: wt.clone(),
        remote: remote.clone(),
        base: base.clone(),
    };

    // Already recorded: bring it back rather than starting a second home.
    let mut plan = Plan::default();
    if let Ok(Some(e)) = ctx.ledger().get(&wt.to_string_lossy()) {
        match state_of(&e.record, &ctx.live()) {
            st @ State::Running { .. } => return plan_reach(ctx, &e, st, &label, o),
            st @ (State::Ready | State::Ended) => {
                let mut p = plan_reach(ctx, &e, st, &label, o)?;
                p.actions.insert(0, refresh);
                return Ok(p);
            }
            // Gone, or no id: a fresh home.
            State::Gone | State::NoId => plan.actions.push(Action::DropRecord(e.key.clone())),
        }
    }

    // An agent's home runs on its repo's harness: the persona needs one
    // that has agents.
    let harness = ctx.harness(&reg.harness(None, Some(repo), None))?;
    if !harness.capabilities().personas || harness.persona_dir().is_none() {
        return Err(Error::Refused(format!(
            "agent '{name}': the {} harness has no agents",
            harness.name()
        )));
    }
    crate::create::check_flags(harness, o.model.is_some(), None)?;

    // Merged agents only: checked against the base, freshly fetched.
    plan.notes.push(Note::Info(format!("fetching {start}")));
    let _ = git::git(sys, &repo.path, &["fetch", "--quiet", &remote, &base]);
    if !crate::create::has_agent_at(ctx, harness, &repo.path, &start, name) {
        return Err(crate::create::no_agent(
            harness,
            name,
            &format!("{start}:"),
            &repo.path,
        ));
    }
    if wt.exists() {
        if git::git(sys, &wt, &["rev-parse", "--is-inside-work-tree"]).is_none() {
            return Err(Error::Refused(format!(
                "{} exists and isn't a git worktree",
                wt.display()
            )));
        }
        plan.actions.push(refresh);
    } else {
        plan.actions.push(Action::AddWorktree {
            repo: repo.path.clone(),
            path: wt.clone(),
            branch: BranchSpec::Detached(start.clone()),
        });
    }

    let trusted = crate::create::trusted(ctx, repo);
    if !trusted {
        plan.notes.push(Note::Info(format!(
            "not pre-trusting the worktree ({} isn't a trusted repo) — {} will ask",
            repo.key,
            harness.name()
        )));
    }
    let profile = reg.profile_of(Some(repo), None).map(|p| p.name.clone());
    let prepared = harness.prepare(
        ctx,
        &PrepareAsk {
            repo: Some(repo),
            picked: None,
            label: &repo.key,
            dir: &wt,
            mode: Mode::Worktree,
            trusted_clone: trusted.then_some(repo.path.as_path()),
        },
    )?;
    plan.notes.extend(prepared.notes);
    plan.actions.extend(prepared.actions);
    if trusted && !repo.mirror.is_empty() {
        plan.actions.push(Action::MirrorFiles {
            from: repo.path.clone(),
            to: wt.clone(),
            files: repo.mirror.clone(),
        });
    }

    let id = sys.new_session_id();
    let key = Key::for_worktree(&sys.home(), &wt);
    // Only a harness with personas gets here, and so far that's one that
    // lets yo pick the id.
    let id = (harness.ids() == IdPolicy::Picked).then_some(id);
    let brief = o.message.clone().filter(|m| !m.is_empty());
    plan.actions.push(Action::PutRecord(Box::new(Entry {
        key: key.clone(),
        record: Record {
            worktree: wt.clone(),
            session_id: id.clone(),
            name: Some(format!("agent-{name}")),
            title: Some(title.clone()),
            repo: Some(repo.key.clone()),
            profile: profile.clone(),
            mode: Some(Mode::Worktree),
            agent: Some(name.to_string()),
            home: true,
            remote_control: Some(rc.clone()),
            // A home's codename is its title: one per agent.
            codename: Some(title.clone()),
            brief: brief.clone(),
            harness: Some(harness.name().to_string()),
            runner: Some(RunnerKind::Iterm),
            created_at: Some(sys.unix_now()),
            ..Record::default()
        },
    })));
    let spec = SessionSpec {
        command: harness.command(reg, Some(repo), None),
        dir: wt.clone(),
        name: format!("agent-{name}"),
        id,
        model: o.model.clone(),
        persona: Some(name.to_string()),
        codename: title.clone(),
        first_message: brief,
        settings: prepared.settings,
        remote_control: Some(rc.clone()),
    };
    let mut argv = harness.tui_command(&spec);
    argv.env.insert(0, ("YO_SESSION_KEY".into(), key.0.clone()));
    plan.actions.push(Action::Launch(LaunchSpec {
        dir: wt,
        title,
        argv,
        window: o.window,
        group: reg
            .window_group(Some(repo), None)
            .replace("{repo}", &repo.key),
        record: Some(key),
    }));
    Ok(plan)
}

#[cfg(test)]
mod tests;
