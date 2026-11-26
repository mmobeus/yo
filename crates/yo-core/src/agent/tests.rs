//! Agents' homes, on the fake.

use std::fs;
use std::sync::Arc;

use super::*;
use crate::fake::{FakeSys, TempDir, ok};

struct T {
    _dir: TempDir,
    home: PathBuf,
    sys: Arc<FakeSys>,
}

const ID: &str = "11111111-2222-4333-8444-555555555555";

impl T {
    fn new() -> T {
        let dir = TempDir::new().unwrap();
        let home = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(home.join("src/api/.git")).unwrap();
        let cfg = home.join("c.json");
        fs::write(
            &cfg,
            format!(
                r#"{{"repos": {{"api": {{"path": "{}/src/api"}}}},
                    "agents": {{"cindy": {{"repo": "api"}}, "lost": {{"repo": "gone"}}}}}}"#,
                home.display()
            ),
        )
        .unwrap();
        let sys = Arc::new(FakeSys::new(&home));
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        sys.push_ids([ID]);
        T {
            _dir: dir,
            home,
            sys,
        }
    }

    fn merged(&self) {
        let c = self.home.join("src/api").display().to_string();
        let pat = git::agent_pattern("cindy");
        self.sys.answer(
            "git",
            &[
                "-C",
                &c,
                "grep",
                "-q",
                "-E",
                &pat,
                "origin/main",
                "--",
                ".claude/agents",
            ],
            ok(""),
        );
    }

    fn ctx(&self) -> Ctx {
        Ctx::load(self.sys.clone(), &self.home).unwrap()
    }

    fn wt(&self) -> PathBuf {
        self.home.join("src/api-worktrees/agent-cindy")
    }
}

fn refused(r: Result<Plan>) -> String {
    match r {
        Err(Error::Refused(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
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
fn listing() {
    let t = T::new();
    let l = list(&t.ctx());
    assert_eq!(l[0].home.as_deref(), Some(t.wt().as_path()));
    assert_eq!((l[1].name.as_str(), l[1].home.as_ref()), ("lost", None));
}

#[test]
fn refusals() {
    let t = T::new();
    let o = ResumeOpts::default();
    assert!(refused(plan_home(&t.ctx(), "nope", &o)).starts_with("unknown agent 'nope'"));
    assert!(
        refused(plan_home(&t.ctx(), "lost", &o))
            .starts_with("agent 'lost' names unknown repo 'gone'")
    );
    // Not merged on the base, nor in ~/.claude/agents.
    assert!(
        refused(plan_home(&t.ctx(), "cindy", &o))
            .starts_with("no agent named 'cindy' in origin/main:")
    );
}

#[test]
fn a_new_home() {
    let t = T::new();
    t.merged();
    let p = plan_home(
        &t.ctx(),
        "cindy",
        &ResumeOpts {
            message: Some("look around".into()),
            ..ResumeOpts::default()
        },
    )
    .unwrap();
    assert!(p.actions.iter().any(|a| matches!(a,
        Action::AddWorktree { path, branch: BranchSpec::Detached(s), .. } if *path == t.wt() && s == "origin/main")));
    let r = p
        .actions
        .iter()
        .find_map(|a| match a {
            Action::PutRecord(e) => Some(&e.record),
            _ => None,
        })
        .unwrap();
    assert!(r.home);
    assert_eq!(r.name.as_deref(), Some("agent-cindy"));
    assert_eq!(r.title.as_deref(), Some("Cindy (home)"));
    assert_eq!(r.codename.as_deref(), Some("Cindy (home)"));
    assert_eq!(r.remote_control.as_deref(), Some("cindy-home"));
    assert_eq!(r.brief.as_deref(), Some("look around"));
    let a = &launch(&p).argv.args;
    let i = a.iter().position(|x| x == "--agent").unwrap();
    assert_eq!(
        a[i..i + 8],
        [
            "--agent",
            "cindy",
            "--name",
            "cindy-home",
            "--remote-control",
            "cindy-home",
            "--session-id",
            ID
        ]
    );
    assert!(
        a.last()
            .unwrap()
            .starts_with("look around (FYI, your codename for this session is \"Cindy (home)\"")
    );
}

#[test]
fn an_existing_home_is_refreshed_and_resumed_or_reached() {
    let t = T::new();
    fs::create_dir_all(t.wt()).unwrap();
    let mut e = Entry {
        key: Key::for_worktree(&t.home, &t.wt()),
        record: Record {
            worktree: t.wt(),
            session_id: Some("abc".into()),
            name: Some("agent-cindy".into()),
            agent: Some("cindy".into()),
            home: true,
            remote_control: Some("cindy-home".into()),
            ..Record::default()
        },
    };
    t.ctx().ledger().write(&mut e).unwrap();
    let p = plan_home(&t.ctx(), "cindy", &ResumeOpts::default()).unwrap();
    assert!(matches!(&p.actions[0], Action::RefreshHome { path, .. } if *path == t.wt()));
    let a = &launch(&p).argv.args;
    assert!(a.windows(2).any(|w| w == ["--resume", "abc"]), "{a:?}");
    assert!(
        a.windows(2)
            .any(|w| w == ["--remote-control", "cindy-home"])
    );

    // Gone (no directory): forgotten, and a fresh one made.
    fs::remove_dir_all(t.wt()).unwrap();
    t.merged();
    let p = plan_home(&t.ctx(), "cindy", &ResumeOpts::default()).unwrap();
    assert!(matches!(&p.actions[0], Action::DropRecord(k) if *k == e.key));
    assert!(
        p.actions
            .iter()
            .any(|a| matches!(a, Action::AddWorktree { .. }))
    );
}
