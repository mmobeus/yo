//! `plan_down`'s refusals and plans, on the fake; and one teardown against
//! a real git worktree.

use std::fs;
use std::sync::Arc;

use super::*;
use crate::fake::{FakeSys, TempDir, ok};
use crate::ledger::Record;

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
                session: Some("w0t1p0:UUID".into())
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
    assert_eq!(plan.actions, [Action::CloseOwnTab { session: None }]);

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
