//! What the read-only commands print, against the fake `Sys` under a
//! temporary `HOME`.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use clap::Parser;
use yo_core::fake::{FakeSys, TempDir};

use crate::cli::Cli;
use crate::run::run;

struct Out {
    code: i32,
    out: String,
    err: String,
}

fn yo(sys: &Arc<FakeSys>, cwd: &Path, args: &[&str]) -> Out {
    let cli = Cli::try_parse_from(std::iter::once("yo").chain(args.iter().copied())).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&cli, sys.clone(), cwd, &mut out, &mut err);
    Out {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

/// A sandbox with a config at `~/.config/yo/config.json`.
fn sandbox(config: Option<&str>) -> (TempDir, Arc<FakeSys>) {
    let t = TempDir::new().unwrap();
    if let Some(c) = config {
        let p = t.path().join(".config/yo");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("config.json"), c).unwrap();
    }
    let sys = Arc::new(FakeSys::new(t.path()));
    (t, sys)
}

/// Everything after the guide: the registry.
fn registry_part(help: &str) -> &str {
    &help[help.find("configured repos:").unwrap()..]
}

#[test]
fn help_and_config_show_list_the_registry() {
    let (t, sys) = sandbox(Some(
        r#"{"profiles": {"work": {"dir": "~/work", "owners": ["acme"],
                                  "repos": {"api": {"path": "/src/api", "base": "dev"}}},
                         "empty": {}},
            "repos": {"yo": {"path": "~/yo"}, "home": {"path": "/src/home", "mode": "here"}},
            "agents": {"cindy": {"repo": "yo"}}}"#,
    ));
    let home = t.path().display().to_string();
    sys.answer_ok(
        "git",
        &[
            "-C",
            &format!("{home}/yo"),
            "symbolic-ref",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        "origin/main",
    );
    let help = yo(&sys, t.path(), &["help"]);
    assert_eq!(help.code, 0);
    assert!(
        help.out
            .starts_with("yo — spin up / tear down an iTerm2 work session")
    );
    let want = format!(
        "configured repos:
  (from {home}/.config/yo/config.json)
  * yo           {home}/yo (base: main)
    home         /src/home (base: main, in place)
    api          /src/api (base: dev, profile: work)
  (* = default)
profiles:
    work         {home}/work  owners: acme  repos: api
    empty        repos: (none)
agents:
    cindy        repo: yo
"
    );
    assert_eq!(registry_part(&help.out), want);

    let show = yo(&sys, t.path(), &["config"]);
    assert_eq!(
        show.out,
        format!("config: {home}/.config/yo/config.json\n{want}")
    );
    let path = yo(&sys, t.path(), &["config", "path"]);
    assert_eq!(path.out, format!("{home}/.config/yo/config.json\n"));
}

#[test]
fn single_repo_mode() {
    let (t, sys) = sandbox(None);
    let home = t.path().display().to_string();
    sys.answer_ok(
        "git",
        &["-C", "/w/proj", "rev-parse", "--show-toplevel"],
        "/w/proj",
    );
    sys.answer_ok(
        "git",
        &["-C", "/w/proj", "rev-parse", "--git-common-dir"],
        ".git",
    );
    let out = yo(&sys, Path::new("/w/proj"), &["config", "show"]);
    assert_eq!(
        out.out,
        format!(
            "config: none yet ({home}/.config/yo/config.json)
configured repos:
  (no config file — single-repo mode, inferred from the current clone)
  * proj         /w/proj (base: main)
  (* = default)
"
        )
    );
}

#[test]
fn a_bad_config_is_an_error() {
    let (t, sys) = sandbox(Some(r#"{"repo": {}}"#));
    let out = yo(&sys, t.path(), &["help"]);
    assert_eq!(out.code, 1);
    assert!(out.err.starts_with(&format!(
        "yo: {}/.config/yo/config.json: unknown field `repo`",
        t.path().display()
    )));
    assert_eq!(out.out, "");
}

#[test]
fn sessions_empty() {
    let (t, sys) = sandbox(Some("{}"));
    let out = yo(&sys, t.path(), &["sessions"]);
    assert_eq!(
        out.out,
        format!(
            "no sessions in the ledger ({}/.local/state/yo/sessions)\n",
            t.path().display()
        )
    );
}

#[test]
fn sessions_lists_and_adopts() {
    let (t, sys) = sandbox(Some(r#"{"repos": {"api": {"path": "~/api"}}}"#));
    let home = t.path();
    let ledger = home.join(".local/state/yo/sessions");
    fs::create_dir_all(&ledger).unwrap();
    let wt = |n: &str| {
        let p = home.join("api-worktrees").join(n);
        fs::create_dir_all(p.join(".git")).unwrap();
        p.display().to_string()
    };
    let rec =
        |key: &str, json: String| fs::write(ledger.join(format!("{key}.json")), json).unwrap();
    let (a, b, c) = (wt("a"), wt("b"), wt("c"));
    rec(
        "api-worktrees__a",
        format!(
            r#"{{"worktree": "{a}", "sessionId": "s-a", "name": "a", "title": "Fix it", "repo": "api", "codename": "Plucky Gary"}}"#
        ),
    );
    rec(
        "api-worktrees__b",
        format!(
            r#"{{"worktree": "{b}", "sessionId": "s-b", "name": "b", "repo": "api", "agent": "cindy", "home": true, "title": "Cindy (home)", "endedAt": 5}}"#
        ),
    );
    rec("id-s-g", r#"{"worktree": "/gone", "sessionId": "s-g", "name": "g", "profile": "work", "byId": true}"#.to_string());
    rec("zz", format!(r#"{{"worktree": "{a}", "name": "n"}}"#));
    // a is running; c is running and not in the ledger yet.
    let sessions = home.join(".claude/sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("41.json"),
        format!(r#"{{"pid": 41, "sessionId": "s-a", "cwd": "{a}"}}"#),
    )
    .unwrap();
    fs::write(
        sessions.join("42.json"),
        format!(r#"{{"pid": 42, "sessionId": "s-c", "cwd": "{c}"}}"#),
    )
    .unwrap();
    sys.add_process(41, "claude", None);
    sys.add_process(42, "claude", None);

    let out = yo(&sys, home, &["sessions"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.err, "adopted c (s-c) from a live session\n");
    let rows: Vec<&str> = out.out.lines().collect();
    assert_eq!(
        rows[..6],
        [
            "STATUS   PID     REPO         AGENT      SESSION                              NAME",
            "running  41      api          -          s-a                                  a — Fix it (Plucky Gary)",
            "ended    -       api          cindy      s-b                                  b — Cindy (home)",
            "running  42      api          -          s-c                                  c",
            "gone     -       (work)       -          s-g                                  g  (in place)",
            "no-id    -       -            -          -                                    n",
        ]
    );
    assert_eq!(
        rows[6..],
        [
            "",
            "ready = resumable (yo resume <name|id>, or yo resume-all); ended = quit with /exit",
            "(yo resume <name> still reopens it; resume-all skips it); gone = directory removed",
        ]
    );
}

#[test]
fn commands_not_ported_yet_say_so() {
    let (t, sys) = sandbox(None);
    let out = yo(&sys, t.path(), &["down", "--yes"]);
    assert_eq!(
        (out.code, out.err.as_str()),
        (
            1,
            "yo: `yo down` isn't in the Rust yo yet; scripts/yo still has it\n"
        )
    );
    // Creating with no repo to work in refuses before anything is created.
    let out = yo(&sys, t.path(), &["some-name", "a message"]);
    assert_eq!(out.code, 1);
    assert!(out.err.starts_with("yo: no repo given"), "{}", out.err);
    // Nothing was written.
    assert!(!t.path().join(".local").exists());
}

/// A sandbox with one clone, `api`, that `detect_repo` finds from inside it.
fn with_clone() -> (TempDir, Arc<FakeSys>, std::path::PathBuf) {
    let t = TempDir::new().unwrap();
    let api = t.path().join("src/api");
    fs::create_dir_all(api.join(".git")).unwrap();
    let cfg = t.path().join(".config/yo");
    fs::create_dir_all(&cfg).unwrap();
    fs::write(
        cfg.join("config.json"),
        format!(r#"{{"repos": {{"api": {{"path": "{}"}}}}}}"#, api.display()),
    )
    .unwrap();
    let sys = Arc::new(FakeSys::new(t.path()));
    let a = api.display().to_string();
    sys.answer_ok("git", &["-C", &a, "rev-parse", "--show-toplevel"], &a);
    sys.answer_ok("git", &["-C", &a, "rev-parse", "--git-common-dir"], ".git");
    sys.push_ids(["11111111-2222-4333-8444-555555555555"]);
    sys.push_picks([15, 24]);
    (t, sys, api)
}

#[test]
fn create_dry_run_shows_the_plan_and_changes_nothing() {
    let (t, sys, api) = with_clone();
    let out = yo(&sys, &api, &["--dry-run", "feat", "do it"]);
    assert_eq!(out.code, 0, "{}", out.err);
    let wt = api.with_file_name("api-worktrees").join("feat");
    for want in [
        "dry run: would\n".to_string(),
        "  - fetch origin/main\n".to_string(),
        format!(
            "  - create worktree {} on feat (new, from origin/main)\n",
            wt.display()
        ),
        "  - record the session in the ledger as src__api-worktrees__feat\n".to_string(),
        format!("  - open [feat] in {}\n", wt.display()),
        "  - pre-trust the worktree in ~/.claude.json\n".to_string(),
        format!(
            "      YO_SESSION_KEY='src__api-worktrees__feat' claude --permission-mode auto --settings {} --name feat --session-id 11111111-2222-4333-8444-555555555555 'do it (FYI, your codename for this session is \"Plucky Gary\"",
            t.path().join(".local/state/yo/settings/api.json").display()
        ),
        "dry run: nothing changed.\n".to_string(),
    ] {
        assert!(out.out.contains(&want), "missing {want:?} in:\n{}", out.out);
    }
    assert!(!t.path().join(".local").exists());
    assert!(
        !sys.calls()
            .iter()
            .any(|c| c.args.iter().any(|a| a == "fetch" || a == "worktree"))
    );
}

#[test]
fn create_with_a_dry_launch() {
    let (_t, sys, api) = with_clone();
    sys.set_env("YO_DRY_LAUNCH", "1");
    let a = api.display().to_string();
    let wt = api.with_file_name("api-worktrees").join("feat");
    sys.answer_ok("git", &["-C", &a, "fetch", "origin", "main"], "");
    sys.answer_ok(
        "git",
        &[
            "-C",
            &a,
            "worktree",
            "add",
            "-b",
            "feat",
            &wt.to_string_lossy(),
            "origin/main",
        ],
        "",
    );
    let out = yo(&sys, &api, &["feat"]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(
        out.out
            .starts_with("==> fetching origin/main\n==> creating worktree: ")
    );
    assert!(out.out.contains("        codename: Plucky Gary\n"));
    assert!(out.out.contains(&format!(
        "dry launch: [feat] in {}\n  YO_SESSION_KEY=",
        wt.display()
    )));
    assert!(out.out.ends_with("==> ready.\n"));
}

#[test]
fn the_session_hook_updates_its_record_and_never_fails() {
    let (t, sys, api) = with_clone();
    sys.set_env("YO_DRY_LAUNCH", "1");
    let a = api.display().to_string();
    let wt = api.with_file_name("api-worktrees").join("feat");
    sys.answer_ok("git", &["-C", &a, "fetch", "origin", "main"], "");
    sys.answer_ok(
        "git",
        &[
            "-C",
            &a,
            "worktree",
            "add",
            "-b",
            "feat",
            &wt.to_string_lossy(),
            "origin/main",
        ],
        "",
    );
    assert_eq!(yo(&sys, &api, &["feat"]).code, 0);
    // The worktree is the fake's: make it real for the record's checks.
    fs::create_dir_all(&wt).unwrap();
    sys.set_env("YO_SESSION_KEY", "src__api-worktrees__feat");
    let cwd = wt.display();
    crate::run::session_hook(
        sys.clone(),
        &format!(
            r#"{{"hook_event_name": "SessionStart", "session_id": "new-id", "cwd": "{cwd}"}}"#
        ),
    );
    let rec = fs::read_to_string(
        t.path()
            .join(".local/state/yo/sessions/src__api-worktrees__feat.json"),
    )
    .unwrap();
    assert!(rec.contains(r#""sessionId": "new-id""#), "{rec}");
    for junk in ["", "{", "null", r#"{"session_id": "a b"}"#] {
        crate::run::session_hook(sys.clone(), junk);
    }
}

#[test]
fn yo_name_resumes_a_ready_session_and_refuses_an_ambiguous_one() {
    let (t, sys, api) = with_clone();
    sys.set_env("YO_DRY_LAUNCH", "1");
    let wt = api.with_file_name("api-worktrees").join("feat");
    fs::create_dir_all(&wt).unwrap();
    let ledger = t.path().join(".local/state/yo/sessions");
    fs::create_dir_all(&ledger).unwrap();
    let rec = |id: &str, name: &str| {
        format!(
            r#"{{"worktree": "{}", "sessionId": "{id}", "name": "{name}", "repo": "api", "title": "feat"}}"#,
            wt.display()
        )
    };
    fs::write(
        ledger.join("a.json"),
        rec("abcdef12-0000-4000-8000-000000000000", "feat"),
    )
    .unwrap();
    let out = yo(&sys, &api, &["feat", "keep going"]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(out.out.starts_with(&format!(
        "==> resuming feat (abcdef12-0000-4000-8000-000000000000) in {}\n",
        wt.display()
    )));
    assert!(
        out.out
            .contains("--name feat --resume abcdef12-0000-4000-8000-000000000000 'keep going'\n")
    );
    assert!(out.out.ends_with("==> ready.\n"));

    fs::write(
        ledger.join("b.json"),
        rec("abcdef12-1111-4000-8000-000000000000", "feat"),
    )
    .unwrap();
    let out = yo(&sys, &api, &["feat"]);
    assert_eq!(out.code, 1);
    assert!(
        out.err
            .starts_with("yo: ambiguous: 2 sessions match 'feat'\n"),
        "{}",
        out.err
    );
    assert!(
        out.err
            .ends_with("yo: be more specific (use the session id)\n")
    );
    // Nothing was created.
    assert!(
        !sys.calls()
            .iter()
            .any(|c| c.args.iter().any(|a| a == "worktree"))
    );
}

/// A ledger with one ready session, `feat`, and a transcript for an id the
/// ledger doesn't have.
fn with_session() -> (
    TempDir,
    Arc<FakeSys>,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let (t, sys, api) = with_clone();
    sys.set_env("YO_DRY_LAUNCH", "1");
    let wt = api.with_file_name("api-worktrees").join("feat");
    fs::create_dir_all(&wt).unwrap();
    let ledger = t.path().join(".local/state/yo/sessions");
    fs::create_dir_all(&ledger).unwrap();
    fs::write(
        ledger.join("src__api-worktrees__feat.json"),
        format!(
            r#"{{"worktree": "{}", "sessionId": "abcdef12-0000-4000-8000-000000000000", "name": "feat", "repo": "api", "title": "feat"}}"#,
            wt.display()
        ),
    )
    .unwrap();
    let proj = t.path().join(".claude/projects/-x");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("11111111-2222-4333-8444-555555555555.jsonl"),
        format!("{{\"cwd\": \"{}\"}}\n", api.display()),
    )
    .unwrap();
    (t, sys, api, wt)
}

#[test]
fn resume_by_name_prefix_and_transcript() {
    let (t, sys, api, wt) = with_session();
    for q in ["feat", "abcdef12", &wt.display().to_string()] {
        let out = yo(&sys, &api, &["resume", q]);
        assert_eq!(out.code, 0, "{q}: {}", out.err);
        assert!(
            out.out
                .contains("--resume abcdef12-0000-4000-8000-000000000000\n"),
            "{}",
            out.out
        );
        assert!(out.out.ends_with("==> ready.\n"));
    }
    // Known only from its transcript: recorded, then resumed.
    let out = yo(
        &sys,
        &api,
        &["resume", "11111111-2222-4333-8444-555555555555"],
    );
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(
        t.path()
            .join(".local/state/yo/sessions/id-11111111-2222-4333-8444-555555555555.json")
            .is_file()
    );
    let out = yo(&sys, &api, &["resume", "nosuch"]);
    assert_eq!(
        out.err,
        "yo: no session matches 'nosuch' (yo sessions lists them)\n"
    );
    fs::remove_dir_all(&wt).unwrap();
    let out = yo(&sys, &api, &["resume", "feat"]);
    assert!(
        out.err.starts_with("yo: feat's directory is gone"),
        "{}",
        out.err
    );
}

#[test]
fn resume_all_dry_and_not() {
    let (_t, sys, api, _wt) = with_session();
    let out = yo(&sys, &api, &["resume-all", "--dry-run"]);
    assert!(
        out.out
            .starts_with("would resume: feat (abcdef12-0000-4000-8000-000000000000) in ")
    );
    assert!(out.out.ends_with("==> 1 session(s) would be resumed\n"));
    assert!(sys.calls().iter().all(|c| c.program != "osascript"));
    let out = yo(&sys, &api, &["resume-all"]);
    assert!(out.out.contains("dry launch: [feat]"), "{}", out.out);
    assert!(out.out.ends_with("==> resumed 1 session(s)\n"));
}

#[test]
fn adopt_forget_and_title() {
    let (t, sys, api, _wt) = with_session();
    let out = yo(&sys, &api, &["adopt"]);
    assert!(out.err.starts_with("yo: no session id"), "{}", out.err);
    sys.set_env(
        "CLAUDE_CODE_SESSION_ID",
        "11111111-2222-4333-8444-555555555555",
    );
    let out = yo(&sys, &api, &["adopt", "--title", "Mine"]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(
        out.out
            .starts_with("==> recorded: Mine\n        session: 11111111")
    );
    let rec = fs::read_to_string(
        t.path()
            .join(".local/state/yo/sessions/id-11111111-2222-4333-8444-555555555555.json"),
    )
    .unwrap();
    assert!(rec.contains(r#""title": "Mine""#), "{rec}");

    let out = yo(&sys, &api, &["forget", "feat"]);
    assert_eq!(
        out.out,
        "forgot feat (abcdef12-0000-4000-8000-000000000000); its transcript and directory are untouched\n"
    );
    assert!(
        yo(&sys, &api, &["forget", "feat"])
            .err
            .starts_with("yo: no single session")
    );

    let out = yo(&sys, &api, &["title", "x"]);
    assert_eq!(
        out.err,
        "yo: not inside an iTerm2 session — no tab to retitle\n"
    );
    sys.set_env("ITERM_SESSION_ID", "w0t1p0:UUID");
    let out = yo(&sys, &api, &["title", "x"]);
    assert_eq!(out.err, "yo: couldn't find this session's iTerm tab\n");
    let c = sys.calls().into_iter().last().unwrap();
    assert_eq!(c.args, ["-", "UUID", "x"]);
}
