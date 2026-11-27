//! `plan_down`'s refusals and plans, on the fake; and one teardown against
//! a real git worktree.

use std::fs;
use std::sync::Arc;

use super::*;
use crate::fake::{FakeSys, TempDir, ok};
use crate::ledger::Record;
use crate::plan::execute;
use crate::realgit::{GitSys, Said, run};
use crate::runner::DryLaunch;

struct T {
    _dir: TempDir,
    home: PathBuf,
    sys: Arc<FakeSys>,
}

impl T {
    fn new() -> T {
        let dir = TempDir::new().unwrap();
        let home = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(home.join("src/api/.git")).unwrap();
        fs::create_dir_all(home.join("src/api-worktrees/feat")).unwrap();
        let cfg = home.join("c.json");
        fs::write(
            &cfg,
            format!(
                r#"{{"repos": {{"api": {{"path": "{}/src/api"}}}}}}"#,
                home.display()
            ),
        )
        .unwrap();
        let sys = Arc::new(FakeSys::new(&home));
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        T {
            _dir: dir,
            home,
            sys,
        }
    }

    fn wt(&self) -> PathBuf {
        self.home.join("src/api-worktrees/feat")
    }

    fn git(&self, dir: &Path, args: &[&str], out: &str) {
        let d = dir.display().to_string();
        let mut a = vec!["-C", d.as_str()];
        a.extend_from_slice(args);
        self.sys.answer("git", &a, ok(out));
    }

    /// The worktree's git, as `plan_down` asks it.
    fn in_worktree(&self, dirty: &str) {
        let wt = self.wt();
        let w = wt.display().to_string();
        self.git(&wt, &["rev-parse", "--show-toplevel"], &w);
        self.git(
            &wt,
            &["rev-parse", "--git-common-dir"],
            &format!("{}/src/api/.git", self.home.display()),
        );
        self.git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"], "feat");
        self.git(&wt, &["status", "--porcelain"], dirty);
        self.git(
            &wt,
            &["log", "--oneline", "origin/main..HEAD"],
            "abc1 work\n",
        );
    }

    fn ctx(&self) -> Ctx {
        Ctx::load(self.sys.clone(), &self.home).unwrap()
    }

    fn record(&self, key: &str, r: Record) {
        let mut e = Entry {
            key: Key(key.into()),
            record: r,
        };
        self.ctx().ledger().write(&mut e).unwrap();
    }
}

fn refused(r: Result<(Found, Plan)>) -> String {
    match r {
        Err(Error::Refused(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_worktree_is_removed_its_branch_kept() {
    let t = T::new();
    t.in_worktree("");
    t.sys.set_env("ITERM_SESSION_ID", "w0t1p0:UUID");
    let (found, plan) = plan_down(&t.ctx(), &t.wt(), DownOpts::default()).unwrap();
    assert_eq!(
        found,
        Found::Worktree {
            repo: "api".into(),
            worktree: t.wt(),
            branch: "feat".into(),
            dirty: vec![],
            unpushed: vec!["abc1 work".into()],
            base: "origin/main".into(),
        }
    );
    assert_eq!(
        plan.actions,
        [
            Action::RemoveWorktree {
                repo: t.home.join("src/api"),
                path: t.wt(),
                force: false
            },
            Action::PruneWorktrees {
                repo: t.home.join("src/api")
            },
            Action::DropRecord(Key("src__api-worktrees__feat".into())),
            Action::CloseOwnTab {
                session: Some("w0t1p0:UUID".into()),
                tab: None,
            },
        ]
    );
}

#[test]
fn uncommitted_work_needs_discard_with_yes() {
    let t = T::new();
    t.in_worktree(" M f\n?? new\n");
    let m = refused(plan_down(
        &t.ctx(),
        &t.wt(),
        DownOpts {
            yes: true,
            discard: false,
        },
    ));
    assert!(
        m.starts_with(
            "refusing: these uncommitted changes would be lost:\n         M f\n        ?? new\n"
        ),
        "{m}"
    );
    // Asked (no --yes), or --discard: it plans a forced removal, shown first.
    for o in [
        DownOpts::default(),
        DownOpts {
            yes: true,
            discard: true,
        },
    ] {
        let (found, plan) = plan_down(&t.ctx(), &t.wt(), o).unwrap();
        assert!(matches!(found, Found::Worktree { dirty, .. } if dirty.len() == 2));
        assert!(matches!(
            plan.actions[0],
            Action::RemoveWorktree { force: true, .. }
        ));
    }
}

#[test]
fn an_agents_home_is_refused() {
    let t = T::new();
    t.in_worktree("");
    t.record(
        "h",
        Record {
            worktree: t.wt(),
            home: true,
            agent: Some("cindy".into()),
            ..Record::default()
        },
    );
    let m = refused(plan_down(
        &t.ctx(),
        &t.wt(),
        DownOpts {
            yes: true,
            discard: true,
        },
    ));
    assert!(m.starts_with("this is cindy's home checkout"), "{m}");
}

#[test]
fn an_unknown_repo_is_refused() {
    let t = T::new();
    let other = t.home.join("x-worktrees/w");
    fs::create_dir_all(&other).unwrap();
    t.git(
        &other,
        &["rev-parse", "--show-toplevel"],
        &other.display().to_string(),
    );
    let m = refused(plan_down(&t.ctx(), &other, DownOpts::default()));
    assert_eq!(m, "couldn't identify the repo from its remotes");
}

#[test]
fn in_place_touches_nothing_on_disk() {
    let t = T::new();
    let dir = t.home.join("src/api");
    // Not a yo worktree: only its record and the tab.
    let (found, plan) = plan_down(&t.ctx(), &dir, DownOpts::default()).unwrap();
    assert_eq!(
        found,
        Found::InPlace {
            dir: dir.clone(),
            key: None
        }
    );
    assert_eq!(
        plan.actions,
        [Action::CloseOwnTab {
            session: None,
            tab: None
        }]
    );

    // The one in-place record holding the directory.
    t.record(
        "id-a",
        Record {
            worktree: dir.clone(),
            session_id: Some("a".into()),
            mode: Some(Mode::Here),
            ..Record::default()
        },
    );
    let (found, _) = plan_down(&t.ctx(), &dir.join("sub"), DownOpts::default()).unwrap();
    assert!(matches!(found, Found::InPlace { key: Some(Key(k)), .. } if k == "id-a"));
    // Two there: neither, unless this session's id picks one.
    t.record(
        "id-b",
        Record {
            worktree: dir.clone(),
            session_id: Some("b".into()),
            mode: Some(Mode::Here),
            ..Record::default()
        },
    );
    let (found, _) = plan_down(&t.ctx(), &dir, DownOpts::default()).unwrap();
    assert!(matches!(found, Found::InPlace { key: None, .. }));
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "b");
    let (found, plan) = plan_down(&t.ctx(), &dir, DownOpts::default()).unwrap();
    assert!(matches!(found, Found::InPlace { key: Some(Key(k)), .. } if k == "id-b"));
    assert_eq!(plan.actions[0], Action::DropRecord(Key("id-b".into())));
    // YO_SESSION_KEY names it outright.
    t.sys.set_env("YO_SESSION_KEY", "id-a");
    let (found, _) = plan_down(&t.ctx(), &dir, DownOpts::default()).unwrap();
    assert!(matches!(found, Found::InPlace { key: Some(Key(k)), .. } if k == "id-a"));
}

#[test]
fn planning_runs_nothing_that_changes() {
    let t = T::new();
    t.in_worktree(" M f\n");
    let _ = plan_down(&t.ctx(), &t.wt(), DownOpts::default());
    for c in t.sys.calls() {
        let sub = c.args.get(2).map(String::as_str);
        assert!(
            matches!(
                sub,
                Some("rev-parse" | "status" | "log" | "remote" | "symbolic-ref" | "show-ref")
            ),
            "planning ran {:?}",
            c.args
        );
    }
}

/// A clone, its origin and two worktrees, all real; the fake's `HOME` and
/// environment, with real git.
#[test]
fn a_teardown_against_real_git() {
    let dir = TempDir::new().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let origin = home.join("origin.git");
    let clone = home.join("src/api");
    fs::create_dir_all(&clone).unwrap();
    run(&home, &["init", "-q", "--bare", &origin.to_string_lossy()]);
    run(&clone, &["init", "-q"]);
    run(
        &clone,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    run(&clone, &["commit", "-q", "--allow-empty", "-m", "init"]);
    run(&clone, &["push", "-q", "origin", "HEAD:main"]);
    run(&clone, &["fetch", "-q", "origin"]);
    let wt = |n: &str| home.join("src/api-worktrees").join(n);
    for n in ["feat", "late"] {
        run(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                n,
                &wt(n).to_string_lossy(),
                "origin/main",
            ],
        );
    }
    fs::write(wt("feat").join("f"), "x").unwrap();
    run(&wt("feat"), &["add", "f"]);
    run(&wt("feat"), &["commit", "-q", "-m", "work"]);

    let cfg = home.join("c.json");
    fs::write(
        &cfg,
        format!(
            r#"{{"repos": {{"api": {{"path": "{}"}}}}}}"#,
            clone.display()
        ),
    )
    .unwrap();
    let fake = FakeSys::new(&home);
    fake.set_env("YO_CONFIG", &cfg.to_string_lossy());
    let ctx = Ctx::load(Arc::new(GitSys(fake)), &home).unwrap();
    for n in ["feat", "late"] {
        let mut e = Entry {
            key: Key::for_worktree(&home, &wt(n)),
            record: Record {
                worktree: wt(n),
                name: Some(n.into()),
                session_id: Some(format!("id-{n}")),
                ..Record::default()
            },
        };
        ctx.ledger().write(&mut e).unwrap();
    }
    let yes = DownOpts {
        yes: true,
        discard: false,
    };
    let recorded = |n: &str| {
        ctx.ledger()
            .get(&wt(n).to_string_lossy())
            .unwrap()
            .is_some()
    };

    // Clean, with a commit not pushed: the worktree goes, its branch (and
    // the commit) stays, and only its own record is dropped.
    let (found, plan) = plan_down(&ctx, &wt("feat"), yes).unwrap();
    assert!(
        matches!(&found, Found::Worktree { dirty, unpushed, .. } if dirty.is_empty() && unpushed.len() == 1),
        "{found:?}"
    );
    execute(&ctx, &plan, &DryLaunch, &mut Said(Vec::new())).unwrap();
    assert!(!wt("feat").exists());
    assert!(run(&clone, &["branch", "--list", "feat"]).contains("feat"));
    assert!(!run(&clone, &["worktree", "list"]).contains("api-worktrees/feat"));
    assert!(!recorded("feat"));
    assert!(recorded("late"));

    // Clean when planned, dirty by the time it runs: the plan didn't ask
    // for --force, so git refuses and everything stays, the record too.
    let (_, plan) = plan_down(&ctx, &wt("late"), yes).unwrap();
    assert!(plan.actions.contains(&Action::RemoveWorktree {
        repo: clone.clone(),
        path: wt("late"),
        force: false,
    }));
    fs::write(wt("late").join("new"), "x").unwrap();
    assert!(execute(&ctx, &plan, &DryLaunch, &mut Said(Vec::new())).is_err());
    assert!(wt("late").join("new").is_file());
    assert!(recorded("late"));
}

/// The worktree's record, with session `s1` running in it as pid 30 (under
/// its shell, 20), in tab `pty-W/40`.
fn running(t: &T) {
    t.in_worktree("");
    t.record(
        "src__api-worktrees__feat",
        Record {
            worktree: t.wt(),
            session_id: Some("s1".into()),
            tab: Some("pty-W/40".into()),
            ..Record::default()
        },
    );
    let dir = t.home.join(".claude/sessions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("30.json"),
        format!(
            r#"{{"pid":30,"sessionId":"s1","cwd":"{}","kind":"interactive"}}"#,
            t.wt().display()
        ),
    )
    .unwrap();
    t.sys.add_process(20, "-zsh", Some("ttys009"));
    t.sys.add_child(30, "claude", Some("ttys009"), 20);
}

/// Ends sessions for real (on the fake), and says what it closed.
#[derive(Default)]
struct Closer(std::sync::Mutex<Vec<(String, Option<String>)>>);

impl crate::runner::Runner for Closer {
    fn kind(&self) -> crate::runner::RunnerKind {
        crate::runner::RunnerKind::Iterm
    }
    fn launch(
        &self,
        _: &dyn crate::sys::Sys,
        _: &crate::runner::LaunchSpec,
    ) -> Result<crate::runner::Launched> {
        unreachable!()
    }
    fn focus(
        &self,
        _: &dyn crate::sys::Sys,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<crate::runner::Focused> {
        unreachable!()
    }
    fn close_own_tab(&self, _: &dyn crate::sys::Sys, session: &str, tab: Option<&str>) -> bool {
        self.0
            .lock()
            .unwrap()
            .push((session.to_string(), tab.map(str::to_string)));
        true
    }
}

#[test]
fn from_a_shell_its_session_is_ended_before_the_tab_closes() {
    // `yo down` in the tab's CLI: a workgroup tab closed while it shows CLI
    // leaves the chat running (#35).
    let t = T::new();
    running(&t);
    t.sys.set_env("ITERM_SESSION_ID", "w0t1p0:CLI");
    let (_, plan) = plan_down(&t.ctx(), &t.wt(), DownOpts::default()).unwrap();
    assert_eq!(
        plan.actions[3..],
        [
            Action::EndSession {
                harness: "claude".into(),
                id: "s1".into(),
                pid: 30,
                after_close: false,
            },
            Action::CloseOwnTab {
                session: Some("w0t1p0:CLI".into()),
                tab: None,
            },
        ]
    );
    let run: Plan = Plan {
        notes: vec![],
        actions: plan.actions[3..].to_vec(),
    };
    let closer = Closer::default();
    let mut said = Said(Vec::new());
    execute(&t.ctx(), &run, &closer, &mut said).unwrap();
    // Its shell hung up, which takes claude with it; then the tab.
    assert_eq!(t.sys.signals(), [(20, crate::sys::Signal::Hup)]);
    assert_eq!(said.0, ["ended the claude session (pid 30)"]);
    assert_eq!(
        *closer.0.lock().unwrap(),
        [("w0t1p0:CLI".to_string(), None)]
    );
}

#[test]
fn from_inside_it_a_detached_helper_ends_it_after_the_close() {
    let t = T::new();
    running(&t);
    t.sys.set_env("ITERM_SESSION_ID", "w0t1p0:CHAT");
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "s1");
    let (_, plan) = plan_down(&t.ctx(), &t.wt(), DownOpts::default()).unwrap();
    assert_eq!(
        plan.actions[3..],
        [
            Action::EndSession {
                harness: "claude".into(),
                id: "s1".into(),
                pid: 30,
                after_close: true,
            },
            // By its recorded tab too: shown CLI, iTerm doesn't list the chat.
            Action::CloseOwnTab {
                session: Some("w0t1p0:CHAT".into()),
                tab: Some("pty-W/40".into()),
            },
        ]
    );
    let run: Plan = Plan {
        notes: vec![],
        actions: plan.actions[3..].to_vec(),
    };
    execute(&t.ctx(), &run, &Closer::default(), &mut Said(Vec::new())).unwrap();
    assert!(t.sys.signals().is_empty());
    let sp = t.sys.spawned();
    assert_eq!(sp.len(), 1);
    assert_eq!(sp[0].program, "/opt/yo/bin/yo");
    assert_eq!(sp[0].args, ["_end-session", "s1", "30"]);
}

#[test]
fn a_session_not_running_is_left_alone_and_a_dry_run_ends_nothing() {
    let t = T::new();
    running(&t);
    // Not running: nothing to end.
    fs::remove_file(t.home.join(".claude/sessions/30.json")).unwrap();
    let (_, plan) = plan_down(&t.ctx(), &t.wt(), DownOpts::default()).unwrap();
    assert!(
        !plan
            .actions
            .iter()
            .any(|a| matches!(a, Action::EndSession { .. }))
    );
    // Running, under a dry runner: said, not done.
    running(&t);
    let (_, plan) = plan_down(&t.ctx(), &t.wt(), DownOpts::default()).unwrap();
    let end: Vec<Action> = plan
        .actions
        .into_iter()
        .filter(|a| matches!(a, Action::EndSession { .. }))
        .collect();
    let mut said = Said(Vec::new());
    execute(
        &t.ctx(),
        &Plan {
            notes: vec![],
            actions: end,
        },
        &DryLaunch,
        &mut said,
    )
    .unwrap();
    assert!(t.sys.signals().is_empty() && t.sys.spawned().is_empty());
    assert_eq!(said.0, ["dry end: the claude session s1 (pid 30)"]);
}

#[test]
fn from_inside_it_with_no_tab_to_close_it_is_left_running() {
    // Another terminal: nothing closes, so nothing ends it either; the
    // /yo-down skill tells the user it's still open.
    let t = T::new();
    running(&t);
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "s1");
    let (_, plan) = plan_down(&t.ctx(), &t.wt(), DownOpts::default()).unwrap();
    assert_eq!(
        plan.actions[3..],
        [Action::CloseOwnTab {
            session: None,
            tab: Some("pty-W/40".into()),
        }]
    );
}
