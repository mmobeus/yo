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
    let out = yo(&sys, t.path(), &["some-name", "a message"]);
    assert_eq!(
        out.err,
        "yo: starting or reaching a session isn't in the Rust yo yet; scripts/yo still has it\n"
    );
    // Nothing was written.
    assert!(!t.path().join(".local").exists());
}
