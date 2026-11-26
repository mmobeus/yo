//! Sweep against real git repos: what git itself reports as dirty and
//! landed, and what it refuses to remove. The environment and `HOME` stay
//! the fake's; only commands are real.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::fake::{FakeSys, TempDir};
use crate::harness::{Argv, Capabilities, Harness, SessionSpec};
use crate::ledger::Record;
use crate::plan::execute;
use crate::realgit::{GitSys, Said, run};
use crate::runner::DryLaunch;
use crate::sys::Sys;

/// A harness whose only live session runs in `cwd`.
struct OneLive(PathBuf);

impl Harness for OneLive {
    fn name(&self) -> &str {
        "claude"
    }
    fn live(&self, _: &dyn Sys) -> Vec<LiveSession> {
        vec![LiveSession {
            harness: "claude".into(),
            id: "live-id".into(),
            pid: 4242,
            cwd: self.0.clone(),
            name: None,
            updated_at: 0,
        }]
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn tui_command(&self, _: &SessionSpec) -> Argv {
        Argv::default()
    }
    fn resume_command(&self, _: &str, _: &SessionSpec) -> Argv {
        Argv::default()
    }
}

struct T {
    _dir: TempDir,
    home: PathBuf,
    clone: PathBuf,
    ctx: Ctx,
}

impl T {
    fn wt(&self, name: &str) -> PathBuf {
        self.home.join("src/api-worktrees").join(name)
    }

    /// Worktrees and branches, as git lists them.
    fn state(&self) -> (String, String) {
        (
            run(&self.clone, &["worktree", "list", "--porcelain"]),
            run(&self.clone, &["branch", "--list"]),
        )
    }
}

/// An origin, a clone, and worktrees in every state sweep tells apart.
fn setup() -> T {
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
    let add = |name: &str| {
        let p = home.join("src/api-worktrees").join(name);
        run(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                name,
                &p.to_string_lossy(),
                "origin/main",
            ],
        );
        p
    };
    for n in ["landed", "dirty", "held", "home", "cwd", "live", "ended"] {
        add(n);
    }
    let work = add("work");
    fs::write(work.join("f"), "x").unwrap();
    run(&work, &["add", "f"]);
    run(&work, &["commit", "-q", "-m", "work"]);
    fs::write(home.join("src/api-worktrees/dirty/new"), "x").unwrap();
    // A detached checkout, as a PR review makes.
    run(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            &home.join("src/api-worktrees/review").to_string_lossy(),
            "origin/main",
        ],
    );

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
    let mut ctx = Ctx::load(Arc::new(GitSys(fake)), &home).unwrap();
    ctx.harnesses = vec![Box::new(OneLive(home.join("src/api-worktrees/live/sub")))];
    let ledger = ctx.ledger();
    let rec = |name: &str, f: &dyn Fn(&mut Record)| {
        let wt = home.join("src/api-worktrees").join(name);
        let mut r = Record {
            worktree: wt.clone(),
            name: Some(name.into()),
            session_id: Some(format!("id-{name}")),
            ..Record::default()
        };
        f(&mut r);
        let mut e = Entry {
            key: Key::for_worktree(&home, &wt),
            record: r,
        };
        ledger.write(&mut e).unwrap();
    };
    rec("held", &|_| {});
    rec("home", &|r| {
        r.home = true;
        r.agent = Some("cindy".into());
    });
    rec("ended", &|r| r.ended_at = Some(5));
    T {
        _dir: dir,
        home,
        clone,
        ctx,
    }
}

fn verdicts(reports: &[RepoReport]) -> Vec<(String, Verdict, String)> {
    let mut v: Vec<_> = reports[0]
        .rows
        .iter()
        .map(|r| {
            (
                r.worktree
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                r.verdict,
                r.why.clone(),
            )
        })
        .collect();
    v.sort();
    v
}

fn swept(plan: &Plan) -> Vec<String> {
    let mut v: Vec<String> = plan
        .actions
        .iter()
        .filter_map(|a| match a {
            Action::SweepWorktree { path, .. } => {
                Some(path.file_name().unwrap().to_string_lossy().into_owned())
            }
            _ => None,
        })
        .collect();
    v.sort();
    v
}

#[test]
fn every_verdict_against_real_git() {
    let t = setup();
    let cwd = t.wt("cwd").join("deep");
    fs::create_dir_all(&cwd).unwrap();
    let before = t.state();
    let (reports, notes, plan) = plan_sweep(&t.ctx, &cwd, &SweepOpts::default()).unwrap();
    // Without a GitHub owner/repo, PRs can't be read: said so.
    assert!(matches!(&notes[0], Note::Warn(s) if s.starts_with("no owner/repo for 'api'")));
    let v = verdicts(&reports);
    let get = |n: &str| v.iter().find(|(name, ..)| name == n).unwrap();
    assert_eq!(get("landed").1, Verdict::Dead);
    assert_eq!(get("landed").2, "no commits beyond origin/main (no PR)");
    assert_eq!(get("ended").1, Verdict::Dead);
    assert_eq!(get("review").1, Verdict::Dead);
    assert_eq!(get("work").1, Verdict::Stale);
    assert_eq!(get("held").1, Verdict::Suspended);
    assert_eq!(get("dirty").1, Verdict::Keep);
    assert_eq!(get("dirty").2, "1 uncommitted (no PR)");
    assert_eq!(get("home").2, "cindy's home (yo agent cindy)");
    assert_eq!(get("cwd").2, "current directory");
    assert_eq!(get("live").2, "live session, pid 4242");
    // The primary clone isn't even listed.
    assert!(!v.iter().any(|(n, ..)| n == "api"));
    assert_eq!(swept(&plan), ["ended", "landed", "review"]);

    let (_, _, plan) = plan_sweep(
        &t.ctx,
        &cwd,
        &SweepOpts {
            include_stale: true,
            include_suspended: true,
            ..SweepOpts::default()
        },
    )
    .unwrap();
    // Never the dirty one, an agent's home, the current directory or a
    // live session's.
    assert_eq!(swept(&plan), ["ended", "held", "landed", "review", "work"]);
    // Planning (a dry run) changed nothing.
    assert_eq!(t.state(), before);
}

#[test]
fn applying_removes_only_what_was_planned_and_never_forces() {
    let t = setup();
    let (_, _, plan) = plan_sweep(
        &t.ctx,
        &t.home,
        &SweepOpts {
            include_stale: true,
            delete_branches: true,
            ..SweepOpts::default()
        },
    )
    .unwrap();
    // Run from HOME, so the `cwd` worktree isn't the current directory here.
    assert_eq!(swept(&plan), ["cwd", "ended", "landed", "review", "work"]);
    // "ended" turns dirty after it was classified: git keeps it.
    fs::write(t.wt("ended").join("late"), "x").unwrap();
    let mut said = Said::default();
    execute(&t.ctx, &plan, &DryLaunch, &mut said).unwrap();
    for gone in ["cwd", "landed", "review", "work"] {
        assert!(!t.wt(gone).exists(), "{gone}");
    }
    for kept in ["ended", "dirty", "held", "home", "live"] {
        assert!(t.wt(kept).is_dir(), "{kept}");
    }
    assert!(
        said.0.iter().any(|s| s.starts_with("kept ended:")),
        "{:?}",
        said.0
    );
    let branches = run(&t.clone, &["branch", "--list"]);
    assert!(!branches.contains("landed") && !branches.contains("work"));
    assert!(branches.contains("ended") && branches.contains("dirty"));
    assert!(t.clone.join(".git").is_dir());
    // The removed worktrees' records are gone; others stay.
    let keys: Vec<String> = t.ctx.ledger().all().into_iter().map(|e| e.key.0).collect();
    assert_eq!(keys.len(), 3, "{keys:?}");
    assert!(!run(&t.clone, &["branch", "--list"]).contains(" cwd"));
}

#[test]
fn unknown_repos_are_refused() {
    let t = setup();
    let r = plan_sweep(
        &t.ctx,
        &t.home,
        &SweepOpts {
            repos: vec!["nope".into()],
            ..SweepOpts::default()
        },
    );
    assert!(matches!(r, Err(Error::Refused(m)) if m == "unknown repo(s): nope"));
}

#[test]
fn prs_decide_landed_and_open() {
    let mut idx = BTreeMap::new();
    let t = setup();
    // Through the index directly: gh isn't faked under real commands.
    idx.insert(
        "work".to_string(),
        Pr {
            number: 7,
            state: "MERGED".into(),
        },
    );
    idx.insert(
        "landed".to_string(),
        Pr {
            number: 8,
            state: "OPEN".into(),
        },
    );
    let entries = t.ctx.ledger().all();
    let live = t.ctx.live();
    let ask = Ask {
        ctx: &t.ctx,
        clone: &t.clone,
        base: "origin/main",
        here: t.home.clone(),
        entries: &entries,
        live: &live,
        prs: Some(&idx),
    };
    let wt = |n: &str| Worktree {
        path: t.wt(n),
        branch: Some(n.into()),
        detached: false,
    };
    assert_eq!(
        classify(&ask, &wt("work")),
        (Verdict::Dead, "PR #7 merged (squash-merged)".into())
    );
    assert_eq!(
        classify(&ask, &wt("landed")),
        (Verdict::Keep, "PR #8 open, 0 commit(s)".into())
    );
}

#[test]
fn the_most_decisive_pr_per_branch() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    sys.answer(
        "gh",
        &[
            "pr",
            "list",
            "--repo",
            "acme/api",
            "--state",
            "all",
            "--limit",
            "800",
            "--json",
            "number,headRefName,state",
        ],
        crate::fake::ok(
            r#"[{"number": 1, "headRefName": "a", "state": "CLOSED"},
                {"number": 2, "headRefName": "a", "state": "MERGED"},
                {"number": 3, "headRefName": "b", "state": "OPEN"},
                {"number": 4, "headRefName": "b", "state": "MERGED"},
                {"number": 5, "headRefName": "c", "state": "CLOSED"}]"#,
        ),
    );
    let ctx = Ctx::load(Arc::new(sys), t.path()).unwrap();
    let idx = pr_index(&ctx, "acme/api").unwrap();
    let got: Vec<(&str, u64)> = idx.iter().map(|(k, p)| (k.as_str(), p.number)).collect();
    assert_eq!(got, [("a", 2), ("b", 3), ("c", 5)]);
    assert!(pr_index(&ctx, "other/x").is_none());
}
