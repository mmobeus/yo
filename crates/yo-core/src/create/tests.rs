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
    // The record that gets the tab it opens in.
    assert_eq!(l.record, Some(record(&p).key.clone()));
    assert_eq!(l.argv.command, "claude --permission-mode auto");
    // After the settings layer (`--settings FILE`).
    assert_eq!(l.argv.args[2..6], ["--name", "feat", "--session-id", ID]);
    assert!(l.argv.args[6].starts_with("do it (FYI, your codename"));
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
    assert_eq!(a[2], "--append-system-prompt");
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
fn an_unnamed_name_already_in_the_ledger_gets_a_suffix() {
    // Two started in the same second (#52).
    let t = T::new("");
    let mut e = crate::ledger::Entry {
        key: crate::ledger::Key("id-x".into()),
        record: crate::ledger::Record {
            name: Some("sesh_20261126_085422".into()),
            ..Default::default()
        },
    };
    t.ctx().ledger().write(&mut e).unwrap();
    let p = t.plan_in(&t.home, |_| {}).unwrap();
    assert_eq!(
        record(&p).record.name.as_deref(),
        Some("sesh_20261126_085422_2")
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
    assert_eq!(launch(&p).argv.args[2..4], ["--agent", "cindy"]);

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
    fn default_command(&self) -> &str {
        "bare"
    }
    fn ids(&self) -> IdPolicy {
        IdPolicy::Reported
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
    fn resume_command(&self, id: &str, _s: &SessionSpec) -> Argv {
        Argv {
            env: Vec::new(),
            command: "bare".into(),
            args: vec!["resume".into(), id.into()],
        }
    }
}

#[test]
fn a_harness_without_extras() {
    let t = T::new("");
    let mut ctx = t.ctx();
    ctx.harnesses = vec![Box::new(crate::harness::claude::Claude), Box::new(Bare)];
    let req = |f: fn(&mut CreateRequest)| {
        let mut r = CreateRequest {
            cwd: t.clone("api"),
            args: args(&["x"]),
            harness: Some("bare".into()),
            ..CreateRequest::default()
        };
        f(&mut r);
        r
    };
    let p = plan_create(&ctx, &req(|_| {})).unwrap();
    assert_eq!(record(&p).record.harness.as_deref(), Some("bare"));
    // It reports its own id once it starts: none yet, and none passed.
    assert_eq!(record(&p).record.session_id, None);
    match plan_create(&ctx, &req(|r| r.harness = Some("nope".into()))) {
        Err(Error::Refused(m)) => assert_eq!(m, "no harness 'nope' (yo has: claude, bare)"),
        other => panic!("{other:?}"),
    }
    // No settings layer, memory or trust: those are Claude's.
    assert!(!p.actions.iter().any(|a| matches!(
        a,
        Action::MakeDir { .. } | Action::WriteFile { .. } | Action::Prepare(_)
    )));
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
        if let Launched::Dry { line } = l {
            self.launched.push(line.clone());
        }
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

fn has_trust(p: &Plan) -> bool {
    p.actions.iter().any(|a| {
        matches!(
            a,
            Action::Prepare(crate::harness::Prep::Claude(
                crate::harness::claude::Prep::Trust { .. }
            ))
        )
    })
}

fn settings_file(p: &Plan) -> (&PathBuf, &str) {
    p.actions
        .iter()
        .find_map(|a| match a {
            Action::WriteFile { path, contents } => Some((path, contents.as_str())),
            _ => None,
        })
        .unwrap()
}

#[test]
fn claude_setup_for_a_trusted_worktree() {
    let t = T::new("");
    let ctx = t.ctx();
    let p = t.plan(|r| r.args = args(&["feat"])).unwrap();
    assert!(has_trust(&p));
    let memory = t.home.join(".claude/memories/yo");
    assert!(p.actions.iter().any(|a| matches!(a,
        Action::MakeDir { path, .. } if *path == memory)));
    let (path, contents) = settings_file(&p);
    assert_eq!(path, &ctx.paths.settings().join("api.json"));
    assert!(contents.contains("\"'/opt/yo/bin/yo' _session-hook\""));
    assert!(contents.contains(&format!(
        "\"autoMemoryDirectory\": \"{}\"",
        memory.display()
    )));
    let a = &launch(&p).argv.args;
    assert_eq!(
        a[..2],
        ["--settings".to_string(), path.display().to_string()]
    );

    // In place: yo didn't make the directory, so no trust.
    t.sys.push_ids([ID]);
    let p = t.plan(|r| r.mode = Some(Mode::Here)).unwrap();
    assert!(!has_trust(&p));
}

#[test]
fn mirror_and_mcp_come_from_the_repo() {
    let t = T::new("");
    let h = t.home.display();
    fs::write(
        t.home.join("config.json"),
        format!(r#"{{"repos": {{"api": {{"path": "{h}/src/api", "mcp": ["srv"], "mirror": [".env"]}}}}}}"#),
    )
    .unwrap();
    let p = t.plan(|r| r.args = args(&["feat"])).unwrap();
    assert!(p.actions.iter().any(|a| matches!(a,
        Action::Prepare(crate::harness::Prep::Claude(
            crate::harness::claude::Prep::LocalSettings { mcp, .. })) if mcp == &["srv"])));
    assert!(p.actions.iter().any(|a| matches!(a,
        Action::MirrorFiles { files, .. } if files == &[".env"])));
}

#[test]
fn an_untrusted_profile_gets_no_trust_or_mirror() {
    let t = T::new("");
    let h = t.home.display();
    fs::write(
        t.home.join("config.json"),
        format!(
            r#"{{"profiles": {{"u": {{"trust": false,
            "repos": {{"api": {{"path": "{h}/src/api", "mirror": [".env"]}}}}}}}}}}"#
        ),
    )
    .unwrap();
    let p = t.plan(|r| r.args = args(&["feat"])).unwrap();
    assert!(!has_trust(&p));
    assert!(
        !p.actions
            .iter()
            .any(|a| matches!(a, Action::MirrorFiles { .. }))
    );
    assert!(p.notes.iter().any(|n| matches!(n,
        Note::Info(s) if s.starts_with("not pre-trusting the worktree (api"))));
    // The profile's own layer is named for it and the repo.
    assert!(settings_file(&p).0.ends_with("settings/u.api.json"));
}

#[test]
fn a_profile_settings_file_must_exist() {
    let t = T::new(r#", "profiles": {"p": {"settings": "~/nope.json", "repos": {}}}"#);
    let m = refused(t.plan(|r| {
        r.profile = Some("p".into());
        r.repo = Some("api".into());
    }));
    assert!(m.starts_with("profile 'p': no settings file at"), "{m}");

    // Given, it's merged into the layer.
    fs::write(t.home.join("nope.json"), r#"{"model": "opus"}"#).unwrap();
    fs::write(
        t.home.join("claude-settings.json"),
        r#"{"model": "x", "a": 1}"#,
    )
    .unwrap();
    t.sys.push_ids([ID]);
    let p = t
        .plan(|r| {
            r.profile = Some("p".into());
            r.repo = Some("api".into());
        })
        .unwrap();
    let (path, contents) = settings_file(&p);
    assert!(path.ends_with("settings/p.api.json"));
    let v: serde_json::Value = serde_json::from_str(contents).unwrap();
    assert_eq!(
        (v["model"].as_str(), v["a"].as_i64()),
        (Some("opus"), Some(1))
    );
}

#[test]
fn memory_off() {
    let t = T::new("");
    t.sys.set_env("YO_MEMORIES", "");
    let p = t.plan(|r| r.args = args(&["feat"])).unwrap();
    assert!(
        !p.actions
            .iter()
            .any(|a| matches!(a, Action::MakeDir { .. }))
    );
    assert!(!settings_file(&p).1.contains("autoMemoryDirectory"));
}

#[test]
fn mirroring_copies_only_ignored_files() {
    let t = T::new("");
    let (c, w) = (t.clone("api"), t.home.join("wt"));
    fs::create_dir_all(&w).unwrap();
    for f in [".env", ".tracked", ".there"] {
        fs::write(c.join(f), f).unwrap();
    }
    fs::write(w.join(".there"), "mine").unwrap();
    let wd = w.display().to_string();
    t.sys
        .answer_ok("git", &["-C", &wd, "check-ignore", "-q", ".env"], "");
    let plan = Plan {
        notes: vec![],
        actions: vec![Action::MirrorFiles {
            from: c.clone(),
            to: w.clone(),
            files: args(&[".env", ".tracked", ".there", ".gone"]),
        }],
    };
    let mut rep = Notes::default();
    execute(&t.ctx(), &plan, &DryLaunch, &mut rep).unwrap();
    assert_eq!(fs::read_to_string(w.join(".env")).unwrap(), ".env");
    assert!(!w.join(".tracked").exists());
    assert_eq!(fs::read_to_string(w.join(".there")).unwrap(), "mine");
    let said = rep.0.join("\n");
    assert!(said.contains(".tracked is NOT gitignored"), "{said}");
    assert!(said.contains(".there already in the worktree"), "{said}");
    assert!(said.contains("no .gone in"), "{said}");
}

#[derive(Default)]
struct Notes(Vec<String>);

impl Report for Notes {
    fn note(&mut self, n: &Note) {
        let (Note::Info(s) | Note::Detail(s) | Note::Warn(s)) = n;
        self.0.push(s.clone());
    }
    fn doing(&mut self, _: &Action) {}
    fn launched(&mut self, _: &LaunchSpec, _: &Launched) {}
}

#[test]
fn a_codex_session() {
    let t = T::new(r#", "harnesses": {"codex": {"args": ["--search"]}}"#);
    let p = t
        .plan(|r| {
            r.args = args(&["feat", "do it"]);
            r.harness = Some("codex".into());
            r.from = Some("home".into());
        })
        .unwrap();
    let r = &record(&p).record;
    assert_eq!(r.harness.as_deref(), Some("codex"));
    assert_eq!(r.session_id, None, "Codex reports its own, later");
    assert!(r.created_at.is_some(), "what the id is learned against");
    // No settings layer, memory or trust: Claude's.
    assert!(!p.actions.iter().any(|a| matches!(
        a,
        Action::MakeDir { .. } | Action::WriteFile { .. } | Action::Prepare(_)
    )));
    let l = launch(&p);
    assert_eq!(l.argv.command, "codex --search");
    assert_eq!(l.argv.args[0], "-c");
    assert!(l.argv.args[1].contains("Plucky Gary"));
    // The brief as it is, with a report-back it can do: yo types it.
    let brief = l.argv.args.last().unwrap();
    assert!(brief.starts_with("do it When you finish"), "{brief}");
    assert!(brief.contains("by running `yo home '<report>'`"), "{brief}");
    assert!(!brief.contains("SendMessage"), "{brief}");

    // In place, it's keyed by the id yo made up, which it never passes.
    let t = T::new("");
    let p = t
        .plan(|r| {
            r.args = args(&["-"]);
            r.mode = Some(Mode::Here);
            r.harness = Some("codex".into());
        })
        .unwrap();
    assert_eq!(record(&p).key.0, format!("id-{ID}"));
    assert_eq!(record(&p).record.session_id, None);
    assert!(!launch(&p).argv.args.iter().any(|a| a.contains(ID)));

    // Flags it has nothing for name it.
    let t = T::new("");
    let m = refused(t.plan(|r| {
        r.args = args(&["x"]);
        r.harness = Some("codex".into());
        r.agent = Some("cindy".into());
    }));
    assert!(
        m.starts_with("--agent: the codex harness has no agents"),
        "{m}"
    );
}

/// Asks yes (or no) and records what's said.
struct Answers(bool, Vec<String>);

impl Report for Answers {
    fn note(&mut self, n: &Note) {
        let (Note::Info(s) | Note::Detail(s) | Note::Warn(s)) = n;
        self.1.push(s.clone());
    }
    fn doing(&mut self, _: &Action) {}
    fn launched(&mut self, _: &LaunchSpec, _: &Launched) {}
    fn ask(&mut self, q: &str) -> bool {
        self.1.push(format!("asked: {q}"));
        self.0
    }
}

fn asks(p: &Plan) -> Vec<&Action> {
    p.actions
        .iter()
        .filter(|a| matches!(a, Action::Ask { .. }))
        .collect()
}

#[test]
fn a_codex_worktree_of_a_repo_codex_hasnt_trusted_asks_first() {
    let t = T::new("");
    let codex = |r: &mut CreateRequest| {
        r.args = args(&["feat"]);
        r.harness = Some("codex".into());
    };
    let p = t.plan(codex).unwrap();
    let real = fs::canonicalize(t.clone("api")).unwrap();
    assert!(
        matches!(asks(&p)[..], [Action::Ask { prep: crate::harness::Prep::Codex(
        crate::harness::codex::Prep::Trust { root }), otherwise, .. }]
        if *root == real && otherwise.starts_with("Codex will ask to trust"))
    );

    // Said no (or nobody to ask): only the warning, and nothing written.
    let mut no = Answers(false, Vec::new());
    let only: Plan = Plan {
        notes: Vec::new(),
        actions: asks(&p).into_iter().cloned().collect(),
    };
    execute(&t.ctx(), &only, &DryLaunch, &mut no).unwrap();
    assert!(no.1[1].starts_with("Codex will ask to trust"), "{:?}", no.1);
    let cfg = t.home.join(".codex/config.toml");
    assert!(!cfg.exists());
    // Said yes: the clone's entry, and no question next time.
    let mut yes = Answers(true, Vec::new());
    execute(&t.ctx(), &only, &DryLaunch, &mut yes).unwrap();
    assert!(
        fs::read_to_string(&cfg)
            .unwrap()
            .contains(&format!("[projects.\"{}\"]", real.display()))
    );
    assert!(asks(&t.plan(codex).unwrap()).is_empty());
}

#[test]
fn a_codex_worktree_of_an_untrusted_repo_is_only_warned_about() {
    let t = T::new("");
    let h = t.home.display();
    fs::write(
        t.home.join("config.json"),
        format!(r#"{{"profiles": {{"u": {{"trust": false, "repos": {{"api": {{"path": "{h}/src/api"}}}}}}}}}}"#),
    )
    .unwrap();
    let p = t
        .plan(|r| {
            r.args = args(&["feat"]);
            r.harness = Some("codex".into());
        })
        .unwrap();
    assert!(asks(&p).is_empty());
    assert!(p.notes.iter().any(|n| matches!(n,
        Note::Warn(s) if s.starts_with(&format!("Codex will ask to trust {h}/src/api")))));
}

#[test]
fn a_codex_session_gets_yos_status_line_unless_it_has_its_own() {
    let codex = |r: &mut CreateRequest| {
        r.args = args(&["feat"]);
        r.harness = Some("codex".into());
    };
    let line = |t: &T| {
        let l = launch(&t.plan(codex).unwrap()).clone();
        l.argv
            .args
            .iter()
            .position(|a| a.starts_with("tui.status_line="))
            .map(|i| (l.argv.args[i - 1].clone(), l.argv.args[i].clone()))
    };
    let t = T::new("");
    let (c, v) = line(&t).expect("yo's line");
    assert_eq!(c, "-c");
    assert!(
        v.contains("\"git-branch\"") && v.contains("\"context-used\""),
        "{v}"
    );

    // The person's own, in Codex's config: a -c would beat it.
    let cfg = t.home.join(".codex/config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, "[tui]\nstatus_line = [\"model\"]\n").unwrap();
    assert_eq!(line(&t), None);

    // Or in harnesses.codex, which comes first, so yo's would win.
    let t = T::new(r#", "harnesses": {"codex": {"args": ["-c", "tui.status_line=[\"model\"]"]}}"#);
    assert_eq!(line(&t), None);
}

#[test]
fn an_unapproved_codex_hook_is_warned_about() {
    let t = T::new("");
    let codex = |r: &mut CreateRequest| {
        r.args = args(&["feat"]);
        r.harness = Some("codex".into());
    };
    let unapproved = |p: &Plan| {
        p.notes
            .iter()
            .any(|n| matches!(n, Note::Warn(s) if s.starts_with("yo's Codex hook isn't approved")))
    };
    assert!(
        !unapproved(&t.plan(codex).unwrap()),
        "no hook, nothing to say"
    );
    let hooks = t.home.join(".codex/hooks.json");
    fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    fs::write(
        &hooks,
        r#"{"hooks":{"SessionEnd":[{"hooks":[{"type":"command","command":"'/yo' _session-hook codex"}]}]}}"#,
    )
    .unwrap();
    assert!(unapproved(&t.plan(codex).unwrap()));
    fs::write(
        t.home.join(".codex/config.toml"),
        format!(
            "[hooks.state.\"{}:session_end:0:0\"]\ntrusted_hash = \"sha256:x\"\n",
            hooks.display()
        ),
    )
    .unwrap();
    assert!(!unapproved(&t.plan(codex).unwrap()));
}
