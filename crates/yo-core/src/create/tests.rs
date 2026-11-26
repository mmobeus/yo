//! `plan_create`, against the fake `Sys` under a temporary `HOME`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::fake::{FakeSys, TempDir, ok};
use crate::harness::{Argv, Capabilities, LiveSession};
use crate::plan::{Report, execute};
use crate::runner::{DryLaunch, Launched};
use crate::sys::Sys;

const ID: &str = "11111111-2222-4333-8444-555555555555";

struct T {
    _dir: TempDir,
    home: PathBuf,
    sys: Arc<FakeSys>,
}

impl T {
    /// A sandbox with clones `api` (github acme/api) and `web`, and
    /// whatever else `extra` adds to the config's top level.
    fn new(extra: &str) -> T {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        for r in ["api", "web"] {
            fs::create_dir_all(home.join(format!("src/{r}/.git"))).unwrap();
        }
        let cfg = home.join("config.json");
        let h = home.display();
        fs::write(
            &cfg,
            format!(
                r#"{{"repos": {{"api": {{"path": "{h}/src/api", "github": "acme/api"}},
                              "web": {{"path": "{h}/src/web", "github": "acme/web"}}}}{extra}}}"#
            ),
        )
        .unwrap();
        let sys = Arc::new(FakeSys::new(&home));
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        sys.push_ids([ID]);
        sys.push_picks([15, 24]); // Plucky Gary
        let t = T {
            _dir: dir,
            home,
            sys,
        };
        // Each clone is itself, to `detect_repo`.
        for r in ["api", "web"] {
            let c = t.clone(r).display().to_string();
            t.git(r, &["rev-parse", "--show-toplevel"], &c);
            t.git(r, &["rev-parse", "--git-common-dir"], ".git");
        }
        t
    }

    fn clone(&self, r: &str) -> PathBuf {
        self.home.join(format!("src/{r}"))
    }

    fn git(&self, repo: &str, args: &[&str], out: &str) {
        let c = self.clone(repo).display().to_string();
        let mut a = vec!["-C", c.as_str()];
        a.extend_from_slice(args);
        self.sys.answer("git", &a, ok(out));
    }

    fn ctx(&self) -> Ctx {
        Ctx::load(self.sys.clone(), &self.home).unwrap()
    }

    fn plan(&self, f: impl FnOnce(&mut CreateRequest)) -> Result<Plan> {
        self.plan_in(&self.clone("api"), f)
    }

    fn plan_in(&self, cwd: &Path, f: impl FnOnce(&mut CreateRequest)) -> Result<Plan> {
        let mut req = CreateRequest {
            cwd: cwd.to_path_buf(),
            ..CreateRequest::default()
        };
        f(&mut req);
        plan_create(&self.ctx(), &req)
    }
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_string()).collect()
}

fn record(p: &Plan) -> &Entry {
    p.actions
        .iter()
        .find_map(|a| match a {
            Action::PutRecord(e) => Some(e),
            _ => None,
        })
        .unwrap()
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

fn worktree(p: &Plan) -> (&PathBuf, &BranchSpec) {
    p.actions
        .iter()
        .find_map(|a| match a {
            Action::AddWorktree { path, branch, .. } => Some((path, branch)),
            _ => None,
        })
        .unwrap()
}

fn fetches(p: &Plan) -> Vec<(String, String)> {
    p.actions
        .iter()
        .filter_map(|a| match a {
            Action::Fetch {
                remote, refspec, ..
            } => Some((remote.clone(), refspec.clone())),
            _ => None,
        })
        .collect()
}

fn refused(r: Result<Plan>) -> String {
    match r {
        Err(Error::Refused(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_new_branch_from_the_base() {
    let t = T::new("");
    // The repo's remote for acme/api is `upstream`, not origin.
    t.git("api", &["remote"], "origin\nupstream\n");
    t.git(
        "api",
        &["remote", "get-url", "upstream"],
        "git@github.com:acme/api.git",
    );
    t.git(
        "api",
        &["remote", "get-url", "origin"],
        "git@github.com:me/api.git",
    );
    let p = t.plan(|r| r.args = args(&["feat", "do it"])).unwrap();
    assert_eq!(fetches(&p), [("upstream".into(), "main".into())]);
    let (path, branch) = worktree(&p);
    assert_eq!(path, &t.home.join("src/api-worktrees/feat"));
    assert_eq!(
        branch,
        &BranchSpec::New {
            branch: "feat".into(),
            start: "upstream/main".into()
        }
    );
    let e = record(&p);
    assert_eq!(e.key.0, "src__api-worktrees__feat");
    let r = &e.record;
    assert_eq!(r.session_id.as_deref(), Some(ID));
    assert_eq!(r.name.as_deref(), Some("feat"));
    assert_eq!(r.title.as_deref(), Some("feat"));
    assert_eq!(r.repo.as_deref(), Some("api"));
    assert_eq!(r.mode, Some(Mode::Worktree));
    assert_eq!(r.codename.as_deref(), Some("Plucky Gary"));
    assert_eq!(r.brief.as_deref(), Some("do it"));
    assert_eq!(r.harness.as_deref(), Some("claude"));
    assert!(!r.by_id);
    let l = launch(&p);
    assert_eq!(l.group, "yo");
    assert_eq!(
        l.argv.env,
        [("YO_SESSION_KEY".into(), "src__api-worktrees__feat".into())]
    );
    assert_eq!(l.argv.command, "claude --permission-mode auto");
    assert_eq!(l.argv.args[..4], ["--name", "feat", "--session-id", ID]);
    assert!(l.argv.args[4].starts_with("do it (FYI, your codename"));
}

#[test]
fn branches_that_exist() {
    let t = T::new(r#", "defaults": {"branchPrefix": "me"}"#);
    // A branch named verbatim, checked out as it is; its name loses the prefix.
    t.git(
        "api",
        &["show-ref", "--verify", "--quiet", "refs/heads/me/topic/x"],
        "",
    );
    let p = t.plan(|r| r.args = args(&["me/topic/x"])).unwrap();
    assert!(fetches(&p).is_empty());
    let (path, branch) = worktree(&p);
    assert!(path.ends_with("api-worktrees/topic-x"));
    assert_eq!(branch, &BranchSpec::Existing("me/topic/x".into()));
    assert_eq!(record(&p).record.name.as_deref(), Some("topic-x"));

    // <prefix>/<name> only on origin: a local branch tracking it.
    t.sys.push_ids([ID]);
    t.git(
        "api",
        &["ls-remote", "--exit-code", "--heads", "origin", "me/y"],
        "",
    );
    let p = t.plan(|r| r.args = args(&["y"])).unwrap();
    assert_eq!(fetches(&p), [("origin".into(), "me/y".into())]);
    assert_eq!(
        worktree(&p).1,
        &BranchSpec::Track {
            branch: "me/y".into(),
            start: "origin/me/y".into()
        }
    );
}

#[test]
fn a_slashed_name_that_isnt_a_branch_is_flattened() {
    let t = T::new("");
    let p = t.plan(|r| r.args = args(&["typo/name"])).unwrap();
    assert!(
        p.notes
            .iter()
            .any(|n| matches!(n, Note::Info(s) if s.contains("flattened: typo-name")))
    );
    assert_eq!(worktree(&p).1.name(), "typo-name");
}

#[test]
fn a_pr_checks_out_its_head_and_follows_it() {
    let t = T::new("");
    t.sys.answer(
        "gh",
        &[
            "pr", "view", "3", "--repo", "acme/api", "--json", "title", "--jq", ".title",
        ],
        ok("Fix the thing\n"),
    );
    let p = t
        .plan_in(&t.home, |r| {
            r.args = args(&["https://github.com/acme/api/pull/3/files"]);
        })
        .unwrap();
    assert_eq!(fetches(&p), [("origin".into(), "pull/3/head".into())]);
    assert_eq!(
        worktree(&p).1,
        &BranchSpec::New {
            branch: "review_pr_3".into(),
            start: "FETCH_HEAD".into()
        }
    );
    assert!(p.actions.iter().any(|a| matches!(a,
        Action::SetUpstream { merge, remote, .. } if merge == "refs/pull/3/head" && remote == "origin")));
    let r = &record(&p).record;
    assert_eq!(r.title.as_deref(), Some("PR#3: Fix the thing"));
    assert_eq!(
        r.brief.as_deref(),
        Some("/code-review https://github.com/acme/api/pull/3")
    );
    // A slash command keeps its arguments: the codename goes elsewhere.
    let a = &launch(&p).argv.args;
    assert_eq!(a[0], "--append-system-prompt");
    assert_eq!(
        a.last().unwrap(),
        "/code-review https://github.com/acme/api/pull/3"
    );

    // From another repo, it can't be checked out.
    t.sys.push_ids([ID]);
    let m = refused(t.plan(|r| {
        r.repo = Some("web".into());
        r.args = args(&["https://github.com/acme/api/pull/3"]);
    }));
    assert!(m.contains("conflicts with PR URL repo (api)"), "{m}");
}

#[test]
fn issues_and_their_repos() {
    let t = T::new("");
    let p = t
        .plan(|r| r.args = args(&["https://github.com/acme/api/issues/9", "and this"]))
        .unwrap();
    let r = &record(&p).record;
    assert_eq!(r.name.as_deref(), Some("issue_9"));
    let b = r.brief.as_deref().unwrap();
    assert!(b.starts_with("take a look at the following issue"));
    assert!(b.ends_with("only focus on a part of it. and this"));
    // gh had no answer, so the title is the name.
    assert_eq!(r.title.as_deref(), Some("issue_9"));

    // An issue in one repo, worked in another.
    t.sys.push_ids([ID]);
    let p = t
        .plan(|r| {
            r.repo = Some("web".into());
            r.args = args(&["https://github.com/acme/api/issues/9"]);
        })
        .unwrap();
    let r = &record(&p).record;
    assert_eq!(r.repo.as_deref(), Some("web"));
    assert!(
        r.brief
            .as_deref()
            .unwrap()
            .contains("NOTE: the issue lives in acme/api, but this session works in the web repo")
    );
    assert_eq!(r.title.as_deref(), Some("[web] issue_9"));

    let m = refused(t.plan(|r| r.args = args(&["https://github.com/other/x/issues/1"])));
    assert!(m.starts_with("unrecognized repo in URL: other/x"), "{m}");
}

#[test]
fn unnamed_sessions_take_their_codename_as_title() {
    let t = T::new("");
    // From somewhere yo doesn't know: the default repo.
    let p = t.plan_in(&t.home, |_| {}).unwrap();
    let r = &record(&p).record;
    assert_eq!(r.name.as_deref(), Some("sesh_20261126_085422"));
    assert_eq!(r.title.as_deref(), Some("Plucky Gary"));
    assert_eq!(r.repo.as_deref(), Some("api"));
    assert_eq!(r.brief, None);
    assert!(
        launch(&p)
            .argv
            .args
            .last()
            .unwrap()
            .ends_with("then wait for instructions.")
    );
}

#[test]
fn in_place() {
    let t = T::new("");
    let dir = t.home.join("scratch");
    fs::create_dir_all(&dir).unwrap();
    let p = t
        .plan_in(&dir, |r| {
            r.mode = Some(Mode::Here);
            r.args = args(&["x"]);
        })
        .unwrap();
    assert!(
        !p.actions
            .iter()
            .any(|a| matches!(a, Action::Fetch { .. } | Action::AddWorktree { .. }))
    );
    let e = record(&p);
    assert_eq!(e.key.0, format!("id-{ID}"));
    let r = &e.record;
    assert_eq!(r.worktree, fs::canonicalize(&dir).unwrap());
    assert_eq!((r.mode, r.by_id), (Some(Mode::Here), true));
    assert_eq!(r.repo, None);
    assert_eq!(r.title.as_deref(), Some("[scratch] x"));

    // A named repo runs in its clone.
    t.sys.push_ids([ID]);
    let p = t
        .plan_in(&dir, |r| {
            r.mode = Some(Mode::Here);
            r.repo = Some("web".into());
        })
        .unwrap();
    assert_eq!(
        record(&p).record.worktree,
        fs::canonicalize(t.clone("web")).unwrap()
    );
}

#[test]
fn a_profile_home_that_isnt_a_clone_runs_in_place() {
    let t = T::new(r#", "profiles": {"p": {"dir": "~/other"}}"#);
    fs::create_dir_all(t.home.join("other")).unwrap();
    let p = t.plan(|r| r.args = args(&["p"])).unwrap();
    let r = &record(&p).record;
    assert_eq!(r.mode, Some(Mode::Here));
    assert_eq!(r.profile.as_deref(), Some("p"));
    assert_eq!(r.title.as_deref(), Some("[p] Plucky Gary"));
    let m = refused(t.plan(|r| {
        r.mode = Some(Mode::Worktree);
        r.args = args(&["p"]);
    }));
    assert!(m.contains("isn't a registered clone"), "{m}");
}

#[test]
fn agents() {
    let t = T::new(r#", "agents": {"cindy": {"repo": "web", "title": "Cindy!"}}"#);
    let m = refused(t.plan(|r| r.agent = Some("cindy".into())));
    assert!(
        m.contains("no agent named 'cindy' in origin/main:.claude/agents"),
        "{m}"
    );

    // Merged on the base branch: its repo, its name first in the codename.
    t.sys.push_ids([ID]);
    t.sys.push_picks([15, 24]);
    let pat = git::agent_pattern("cindy");
    t.git(
        "web",
        &[
            "grep",
            "-q",
            "-E",
            &pat,
            "origin/main",
            "--",
            ".claude/agents",
        ],
        "",
    );
    let p = t
        .plan(|r| {
            r.agent = Some("cindy".into());
            r.args = args(&["x"]);
        })
        .unwrap();
    let r = &record(&p).record;
    assert_eq!(r.repo.as_deref(), Some("web"));
    assert_eq!(r.agent.as_deref(), Some("cindy"));
    assert_eq!(r.codename.as_deref(), Some("Cindy! · Plucky Gary"));
    assert_eq!(r.title.as_deref(), Some("[web] Cindy! · x"));
    assert_eq!(launch(&p).argv.args[..2], ["--agent", "cindy"]);

    // In place, from ~/.claude/agents.
    let agents = t.home.join(".claude/agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("bob.md"), "---\nname: \"bob\"\n---\n").unwrap();
    t.sys.push_ids([ID]);
    assert!(
        t.plan(|r| {
            r.mode = Some(Mode::Here);
            r.agent = Some("bob".into());
        })
        .is_ok()
    );
}

#[test]
fn refusals() {
    let t = T::new("");
    for bad in ["a b", "..", "x/../y", "-x", "a//b", "x;rm"] {
        let m = refused(t.plan(|r| r.args = args(&[bad])));
        assert!(m.starts_with("name "), "{bad}: {m}");
    }
    assert!(refused(t.plan(|r| r.repo = Some("nope".into()))).starts_with("--repo must be"));
    assert!(refused(t.plan(|r| r.profile = Some("nope".into()))).starts_with("--profile must be"));
    assert!(refused(t.plan(|r| r.args = args(&["a", "b", "c"]))).starts_with("usage:"));
    fs::create_dir_all(t.home.join("src/api-worktrees/taken")).unwrap();
    assert!(
        refused(t.plan(|r| r.args = args(&["taken"]))).starts_with("worktree path already exists")
    );
    // A named session from a directory with no repo.
    let m = refused(t.plan_in(&t.home, |r| r.args = args(&["x"])));
    assert!(m.starts_with("no repo given"), "{m}");
}

/// Planning only reads: no command that changes anything, no file written.
#[test]
fn planning_changes_nothing() {
    let t = T::new(r#", "agents": {"cindy": {"repo": "api"}}"#);
    let before = tree(&t.home);
    for a in [
        vec!["feat"],
        vec!["https://github.com/acme/api/pull/3"],
        vec!["https://github.com/acme/api/issues/3"],
        vec![],
    ] {
        t.sys.push_ids([ID]);
        let _ = t.plan(|r| r.args = args(&a));
    }
    t.sys.push_ids([ID]);
    let _ = t.plan(|r| r.mode = Some(Mode::Here));
    for c in t.sys.calls() {
        let reads = match c.program.as_str() {
            "git" => {
                matches!(
                    c.args.get(2).map(String::as_str),
                    Some(
                        "show-ref"
                            | "ls-remote"
                            | "remote"
                            | "symbolic-ref"
                            | "rev-parse"
                            | "branch"
                            | "grep"
                    )
                ) || (c.args.get(2).map(String::as_str) == Some("config") && c.args.len() == 4)
            }
            "gh" | "date" => true,
            _ => false,
        };
        assert!(reads, "planning ran {} {:?}", c.program, c.args);
    }
    assert_eq!(tree(&t.home), before);
}

#[test]
fn executing_runs_git_records_and_launches() {
    let t = T::new("");
    let ctx = t.ctx();
    let p = t.plan(|r| r.args = args(&["feat"])).unwrap();
    for a in [
        vec!["fetch", "origin", "main"],
        vec![
            "worktree",
            "add",
            "-b",
            "feat",
            &t.home.join("src/api-worktrees/feat").to_string_lossy(),
            "origin/main",
        ],
    ] {
        t.git("api", &a, "");
    }
    let mut rep = Rec::default();
    execute(&ctx, &p, &DryLaunch, &mut rep).unwrap();
    let git: Vec<String> = t
        .sys
        .calls()
        .into_iter()
        .filter(|c| c.program == "git")
        .map(|c| c.args[2..].join(" "))
        .filter(|a| a.starts_with("fetch") || a.starts_with("worktree"))
        .collect();
    assert_eq!(git.len(), 2, "{git:?}");
    assert_eq!(ctx.ledger().all().len(), 1);
    assert_eq!(rep.launched.len(), 1);
    assert!(rep.launched[0].contains("--session-id 11111111"));

    // A failed step stops the plan: nothing recorded, nothing launched.
    let t = T::new("");
    let ctx = t.ctx();
    let p = t.plan(|r| r.args = args(&["feat"])).unwrap();
    let mut rep = Rec::default();
    assert!(matches!(
        execute(&ctx, &p, &DryLaunch, &mut rep),
        Err(Error::Failed(_))
    ));
    assert!(ctx.ledger().all().is_empty());
    assert!(rep.launched.is_empty());
}

/// Another harness, with none of the optional capabilities.
struct Bare;

impl Harness for Bare {
    fn name(&self) -> &str {
        "bare"
    }
    fn live(&self, _sys: &dyn Sys) -> Vec<LiveSession> {
        Vec::new()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn tui_command(&self, s: &SessionSpec) -> Argv {
        Argv {
            env: Vec::new(),
            command: "bare".into(),
            args: vec![s.codename.clone()],
        }
    }
}

#[test]
fn a_harness_without_extras() {
    let t = T::new("");
    let mut ctx = t.ctx();
    ctx.harnesses = vec![Box::new(Bare)];
    let req = |f: fn(&mut CreateRequest)| {
        let mut r = CreateRequest {
            cwd: t.clone("api"),
            args: args(&["x"]),
            ..CreateRequest::default()
        };
        f(&mut r);
        r
    };
    let p = plan_create(&ctx, &req(|_| {})).unwrap();
    assert_eq!(record(&p).record.harness.as_deref(), Some("bare"));
    assert_eq!(launch(&p).argv.args, ["Plucky Gary"]);
    for (f, want) in [
        (
            (|r: &mut CreateRequest| r.model = Some("opus".into())) as fn(&mut CreateRequest),
            "--model: the bare harness",
        ),
        (
            |r| r.agent = Some("cindy".into()),
            "--agent: the bare harness",
        ),
    ] {
        match plan_create(&ctx, &req(f)) {
            Err(Error::Refused(m)) => assert!(m.starts_with(want), "{m}"),
            other => panic!("{other:?}"),
        }
    }
}

#[derive(Default)]
struct Rec {
    launched: Vec<String>,
}

impl Report for Rec {
    fn note(&mut self, _: &Note) {}
    fn doing(&mut self, _: &Action) {}
    fn launched(&mut self, _: &LaunchSpec, l: &Launched) {
        let Launched::Dry { line } = l;
        self.launched.push(line.clone());
    }
}

/// Every path under `dir`, with each file's contents.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p.clone());
                out.push((p, Vec::new()));
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}
