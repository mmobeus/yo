//! Reaching a session `yo NAME` found: a running one has its tab brought
//! forward and the message typed in; a stopped one is resumed with the
//! message as its next prompt. bash's `_yo_reach_row`, `_yo_reach_live` and
//! `_yo_resume_row`.

use std::path::Path;

use crate::harness::{Harness, PrepareAsk, SessionSpec};
use crate::ledger::{Entry, State};
use crate::place::{self, PlaceTo};
use crate::plan::{Action, Note, Plan};
use crate::registry::Repo;
use crate::runner::{LaunchSpec, WindowChoice};
use crate::{Ctx, Error, Result};

/// How to reopen a session.
#[derive(Clone, Debug, Default)]
pub struct ResumeOpts {
    /// Its next prompt.
    pub message: Option<String>,
    /// A model for this run; else the one it last ran on.
    pub model: Option<String>,
    pub window: WindowChoice,
}

/// A running session's tab: brought forward, with the message typed in.
pub fn plan_focus(
    ctx: &Ctx,
    pid: u32,
    label: &str,
    rc: Option<&str>,
    message: Option<&str>,
) -> Plan {
    let tty = ctx.sys.procs().tty(pid).map(|t| {
        if t.starts_with("/dev/") {
            t
        } else {
            format!("/dev/{t}")
        }
    });
    Plan {
        notes: Vec::new(),
        actions: vec![Action::Focus {
            pid,
            tty,
            label: label.to_string(),
            remote_control: rc.map(str::to_string),
            message: message.filter(|m| !m.is_empty()).map(str::to_string),
        }],
    }
}

/// A ledger session, as `yo NAME` reaches it: focused if it's running,
/// else resumed. `label` is what the progress calls it.
pub fn plan_reach(ctx: &Ctx, e: &Entry, st: State, label: &str, o: &ResumeOpts) -> Result<Plan> {
    match st {
        State::Running { pid } => Ok(plan_focus(
            ctx,
            pid,
            label,
            e.record.remote_control.as_deref(),
            o.message.as_deref(),
        )),
        _ => plan_resume(ctx, e, o),
    }
}

/// A guest repo a session was created in is registered only for that run:
/// registers it again, from its clone, so the session resumes with it.
pub fn ensure_guest(ctx: &mut Ctx, e: &Entry) -> Result<Vec<Note>> {
    let r = &e.record;
    let (Some(key), Some(guest)) = (&r.repo, &r.guest) else {
        return Ok(Vec::new());
    };
    let known = ctx.registry.repo(key).is_some()
        || ctx
            .registry
            .repo_for_url(&*ctx.sys, &format!("https://github.com/{guest}"))
            .is_some();
    if known {
        return Ok(Vec::new());
    }
    place::place(ctx, guest, &PlaceTo::Guest).map(|(_, n)| n)
}

/// The repo a record was created in, as it's registered now.
fn repo_of<'a>(ctx: &'a Ctx, e: &Entry) -> Option<&'a Repo> {
    let r = &e.record;
    let key = r.repo.as_deref()?;
    ctx.registry.repo(key).or_else(|| {
        let g = r.guest.as_deref()?;
        ctx.registry
            .repo_for_url(&*ctx.sys, &format!("https://github.com/{g}"))
    })
}

/// Relaunches a ledger session under the repo and profile it was created
/// with, so it gets the same settings layer, memory and window.
pub fn plan_resume(ctx: &Ctx, e: &Entry, o: &ResumeOpts) -> Result<Plan> {
    let reg = &ctx.registry;
    let r = &e.record;
    let harness: &dyn Harness = ctx
        .harnesses
        .iter()
        .find(|h| h.name() == r.harness.as_deref().unwrap_or("claude"))
        .map(|h| &**h)
        .ok_or_else(|| {
            Error::Refused(format!(
                "no harness '{}' to resume it with",
                r.harness.as_deref().unwrap_or("")
            ))
        })?;
    let name = r.name.clone().unwrap_or_else(|| e.key.0.clone());
    let id = r
        .id()
        .ok_or_else(|| Error::Refused(format!("{name} has no session id to resume")))?;
    if o.model.is_some() && !harness.capabilities().models {
        return Err(Error::Refused(format!(
            "--model: the {} harness can't pick a model",
            harness.name()
        )));
    }
    let mut plan = Plan::default();
    let repo = repo_of(ctx, e);
    if let (Some(k), None) = (&r.repo, repo) {
        plan.notes.push(Note::Warn(format!(
            "repo '{k}' is no longer configured; resuming {name} without its settings"
        )));
    }
    let picked = match r.profile.as_deref() {
        Some(p) if reg.profile(p).is_some() => Some(p),
        Some(p) => {
            plan.notes.push(Note::Warn(format!(
                "profile '{p}' is no longer defined; resuming {name} without it"
            )));
            None
        }
        None => None,
    };
    let agent = r
        .agent
        .as_deref()
        .map(|a| format!(" as {a}"))
        .unwrap_or_default();
    plan.notes.push(Note::Info(format!(
        "resuming {name} ({id}) in {}{agent}",
        r.worktree.display()
    )));
    let label = repo
        .map(|x| x.key.clone())
        .or_else(|| reg.profile_of(repo, picked).map(|p| p.name.clone()))
        .unwrap_or_else(|| basename(&r.worktree));
    let prepared = harness.prepare(
        ctx,
        &PrepareAsk {
            repo,
            picked,
            label: &label,
            dir: &r.worktree,
            mode: r.mode_or_default(),
            trusted_clone: None,
        },
    )?;
    plan.notes.extend(prepared.notes);
    plan.actions.extend(prepared.actions);
    let spec = SessionSpec {
        command: reg.claude_cmd(repo, picked),
        dir: r.worktree.clone(),
        name: name.clone(),
        id: Some(id.to_string()),
        model: o.model.clone(),
        persona: r.agent.clone(),
        codename: r.codename.clone().unwrap_or_default(),
        first_message: o.message.clone().filter(|m| !m.is_empty()),
        settings: prepared.settings,
        remote_control: r.remote_control.clone(),
    };
    let mut argv = harness.resume_command(id, &spec);
    argv.env
        .insert(0, ("YO_SESSION_KEY".into(), e.key.0.clone()));
    plan.actions.push(Action::Launch(LaunchSpec {
        dir: r.worktree.clone(),
        title: r.title.clone().unwrap_or(name),
        argv,
        window: o.window,
        group: reg.window_group(repo, picked).replace("{repo}", &label),
    }));
    Ok(plan)
}

fn basename(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeSys, TempDir};
    use crate::ledger::{Key, Record};
    use crate::plan::{Report, execute};
    use crate::runner::{DryLaunch, Launched};

    const ID: &str = "abcdef12-0000-4000-8000-000000000000";

    fn ctx(t: &TempDir) -> (Ctx, Arc<FakeSys>) {
        let h = t.path().display();
        fs::create_dir_all(t.path().join("src/api-worktrees/feat")).unwrap();
        let cfg = t.path().join("c.json");
        fs::write(
            &cfg,
            format!(
                r#"{{"profiles": {{"p": {{"windowGroup": "g-{{repo}}",
                        "repos": {{"api": {{"path": "{h}/src/api"}}}}}}}}}}"#
            ),
        )
        .unwrap();
        let sys = Arc::new(FakeSys::new(t.path()));
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        (Ctx::load(sys.clone(), t.path()).unwrap(), sys)
    }

    fn entry(t: &TempDir) -> Entry {
        Entry {
            key: Key("src__api-worktrees__feat".into()),
            record: Record {
                worktree: t.path().join("src/api-worktrees/feat"),
                session_id: Some(ID.into()),
                name: Some("feat".into()),
                title: Some("[api] feat".into()),
                repo: Some("api".into()),
                profile: Some("p".into()),
                agent: Some("cindy".into()),
                ..Record::default()
            },
        }
    }

    fn launch(p: &Plan) -> &LaunchSpec {
        p.actions
            .iter()
            .find_map(|a| match a {
                Action::Launch(l) => Some(l),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn resuming_relaunches_with_its_repo_profile_and_message() {
        let t = TempDir::new().unwrap();
        let (c, _) = ctx(&t);
        let e = entry(&t);
        let o = ResumeOpts {
            message: Some("keep going".into()),
            model: Some("opus".into()),
            window: WindowChoice::Window,
        };
        let p = plan_reach(&c, &e, State::Ready, "feat", &o).unwrap();
        let l = launch(&p);
        assert_eq!(l.title, "[api] feat");
        assert_eq!(l.group, "g-api");
        assert_eq!(l.window, WindowChoice::Window);
        assert_eq!(
            l.argv.env,
            [("YO_SESSION_KEY".into(), "src__api-worktrees__feat".into())]
        );
        let settings = c.paths.settings().join("p.api.json");
        assert_eq!(
            l.argv.args,
            [
                "--settings",
                &settings.to_string_lossy(),
                "--model",
                "opus",
                "--agent",
                "cindy",
                "--name",
                "feat",
                "--resume",
                ID,
                "keep going"
            ]
        );
        assert!(
            p.notes.iter().any(|n| matches!(n,
            Note::Info(s) if s.starts_with("resuming feat (abcdef12") && s.ends_with(" as cindy")))
        );
        // Resuming never touches trust.
        assert!(!p.actions.iter().any(|a| matches!(a, Action::Prepare(_))));
    }

    #[test]
    fn a_repo_or_profile_that_went_away_is_said_and_skipped() {
        let t = TempDir::new().unwrap();
        let (c, _) = ctx(&t);
        let mut e = entry(&t);
        e.record.repo = Some("gone".into());
        e.record.profile = Some("nope".into());
        let p = plan_resume(&c, &e, &ResumeOpts::default()).unwrap();
        let warns: Vec<&String> = p
            .notes
            .iter()
            .filter_map(|n| match n {
                Note::Warn(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(warns.len(), 2, "{warns:?}");
        // Filed under its directory now.
        assert!(launch(&p).group == "yo");
        e.record.session_id = None;
        assert!(plan_resume(&c, &e, &ResumeOpts::default()).is_err());
    }

    #[test]
    fn a_guest_is_registered_again_from_its_clone() {
        let t = TempDir::new().unwrap();
        let (mut c, sys) = ctx(&t);
        sys.set_env("YO_CLONE_ROOT", &t.path().join("gh").to_string_lossy());
        let mut c2 = Ctx::load(sys.clone(), t.path()).unwrap();
        std::mem::swap(&mut c, &mut c2);
        fs::create_dir_all(t.path().join("gh/other/x/.git")).unwrap();
        let mut e = entry(&t);
        e.record.repo = Some("x".into());
        e.record.guest = Some("other/x".into());
        ensure_guest(&mut c, &e).unwrap();
        assert_eq!(c.registry.guest.as_deref(), Some("x"));
        // Registered now: nothing to do.
        assert!(ensure_guest(&mut c, &e).unwrap().is_empty());
    }

    #[derive(Default)]
    struct Said(Vec<String>);
    impl Report for Said {
        fn note(&mut self, n: &Note) {
            let (Note::Info(s) | Note::Detail(s) | Note::Warn(s)) = n;
            self.0.push(s.clone());
        }
        fn doing(&mut self, _: &Action) {}
        fn launched(&mut self, _: &LaunchSpec, _: &Launched) {}
    }

    #[test]
    fn focusing_a_running_session() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        sys.add_process(42, "claude", Some("ttys003"));
        sys.add_process(43, "claude", None);
        let p = plan_reach(
            &c,
            &entry(&t),
            State::Running { pid: 42 },
            "feat",
            &ResumeOpts {
                message: Some("hi".into()),
                ..ResumeOpts::default()
            },
        )
        .unwrap();
        assert!(
            matches!(&p.actions[..], [Action::Focus { tty: Some(t), message: Some(m), .. }]
            if t == "/dev/ttys003" && m == "hi")
        );
        let mut said = Said::default();
        execute(&c, &p, &DryLaunch, &mut said).unwrap();
        assert_eq!(said.0, ["dry focus: /dev/ttys003, then type: hi"]);

        // No terminal: says how else to reach it, and that the message didn't go.
        let mut e = entry(&t);
        e.record.remote_control = Some("feat-rc".into());
        let p = plan_reach(
            &c,
            &e,
            State::Running { pid: 43 },
            "feat",
            &ResumeOpts {
                message: Some("hi".into()),
                ..ResumeOpts::default()
            },
        )
        .unwrap();
        let mut said = Said::default();
        execute(&c, &p, &DryLaunch, &mut said).unwrap();
        assert_eq!(
            said.0,
            [
                "feat is running (pid 43), but not in an iTerm tab yo can find;",
                "reach it by Remote Control as feat-rc, or with SendMessage from another session (your message was not delivered)",
            ]
        );
    }
}
