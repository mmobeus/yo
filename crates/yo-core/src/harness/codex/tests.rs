use std::path::{Path, PathBuf};

use super::*;
use crate::fake::{FakeCodex, FakeSys, TempDir};
use crate::sys::RealSys;

fn spec(first: Option<&str>) -> SessionSpec {
    SessionSpec {
        command: "codex --yolo".into(),
        dir: PathBuf::from("/w/a"),
        name: "a".into(),
        id: None,
        model: Some("gpt-6".into()),
        persona: None,
        codename: "Plucky \"Gary\"".into(),
        first_message: first.map(str::to_string),
        settings: None,
        remote_control: None,
        args: Vec::new(),
    }
}

const INSTRUCTIONS: &str = r#"developer_instructions="Your codename for this session is \"Plucky \"Gary\"\". Introduce yourself with it, and use it whenever you identify this session, e.g. in notes, PR comments you're asked to post, or messages.""#;

#[test]
fn launching_tells_it_its_codename_out_of_band() {
    let a = Codex.tui_command(&spec(Some("/review do it")));
    assert_eq!(a.command, "codex --yolo");
    assert_eq!(
        a.args,
        ["-c", INSTRUCTIONS, "-m", "gpt-6", "/review do it"],
        "the message as it is, even a slash command"
    );
    // No brief: it still gets a first message, which is what makes Codex
    // record the session.
    let a = Codex.tui_command(&spec(None));
    assert_eq!(
        a.args.last().unwrap(),
        "Briefly introduce yourself by codename, then wait for instructions."
    );
}

#[test]
fn flags_from_prepare_follow_the_codename() {
    let mut s = spec(Some("hi"));
    s.args = vec!["-c".into(), "tui.status_line=[]".into()];
    assert_eq!(
        Codex.tui_command(&s).args,
        [
            "-c",
            INSTRUCTIONS,
            "-c",
            "tui.status_line=[]",
            "-m",
            "gpt-6",
            "hi"
        ]
    );
    assert_eq!(
        Codex.resume_command("id", &s).args,
        [
            "resume",
            "-c",
            INSTRUCTIONS,
            "-c",
            "tui.status_line=[]",
            "-m",
            "gpt-6",
            "id",
            "hi"
        ]
    );
}

#[test]
fn the_codename_survives_toml_and_the_shell() {
    // What Codex's `-c` parses is TOML: check the value round-trips through
    // a real shell to the same string toml_string meant.
    let line = Codex.tui_command(&spec(Some("x"))).line();
    let out = RealSys
        .cmd()
        .run(&CommandSpec::new("sh").arg("-c").arg(format!(
            "set -- {}; printf %s \"$2\"",
            line.trim_start_matches("codex --yolo ")
        )))
        .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), INSTRUCTIONS);
    assert_eq!(toml_string("a\\b\n\u{1}"), r#""a\\b\n\u0001""#);
}

#[test]
fn resuming_passes_the_id_and_message() {
    let a = Codex.resume_command("01a2-b", &spec(Some("and now?")));
    assert_eq!(
        a.args,
        [
            "resume",
            "-c",
            INSTRUCTIONS,
            "-m",
            "gpt-6",
            "01a2-b",
            "and now?"
        ]
    );
    let a = Codex.resume_command("01a2-b", &spec(None));
    assert_eq!(a.args.last().unwrap(), "01a2-b");
}

#[test]
fn models_and_ledger_hooks_are_available() {
    let c = Codex.capabilities();
    assert!(c.models && c.ledger_hooks);
    assert!(!c.personas && !c.settings_layer && !c.memory && !c.remote_control);
    assert_eq!(Codex.persona_dir(), None);
    assert_eq!(Codex.ids(), IdPolicy::Reported);
}

#[test]
fn its_own_session_from_the_environment() {
    let sys = FakeSys::new("/h");
    assert_eq!(Codex.own_session(&sys), None);
    assert!(!Codex.inside(&sys));
    sys.set_env("CODEX_THREAD_ID", "01a21b23-09e9-7f73-a7b7-d687bad4ff87");
    assert_eq!(
        Codex.own_session(&sys).as_deref(),
        Some("01a21b23-09e9-7f73-a7b7-d687bad4ff87")
    );
    sys.set_env("CODEX_THREAD_ID", "a b");
    assert_eq!(Codex.own_session(&sys), None);
}

#[test]
fn live_sessions_are_the_locks_codex_holds() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    assert!(Codex.live(&sys).is_empty(), "no ~/.codex at all");
    assert!(sys.calls().is_empty(), "and no lsof run for nothing");
    let fake = FakeCodex { sys: &sys };
    // A crash leaves a lock nobody holds; a pid that's no longer codex
    // doesn't count.
    fake.stale_lock("crashed");
    fake.running(&[
        ("bbb", 20, Path::new("/w/b")),
        ("aaa", 10, Path::new("/w/a")),
        ("ccc", 30, Path::new("/w/c")),
    ]);
    sys.add_process(30, "zsh", Some("ttys021"));
    // So only the other two are asked where they are.
    sys.answer_ok(
        "lsof",
        &["-a", "-d", "cwd", "-F", "pn", "-p", "10,20"],
        "p10\nfcwd\nn/w/a\np20\nfcwd\nn/w/b\n",
    );
    let got: Vec<(String, u32, PathBuf)> = Codex
        .live(&sys)
        .into_iter()
        .map(|l| (l.id, l.pid, l.cwd))
        .collect();
    assert_eq!(
        got,
        [
            ("aaa".into(), 10, "/w/a".into()),
            ("bbb".into(), 20, "/w/b".into()),
        ]
    );
}

#[test]
fn a_live_session_without_a_cwd_from_lsof_takes_its_rollouts() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    let fake = FakeCodex { sys: &sys };
    fake.running(&[("aaa", 10, Path::new("/w/a"))]);
    fake.rollout(
        "2026/11/26",
        "aaa",
        Path::new("/w/r"),
        "2026-11-27T03:53:30.908Z",
        "cli",
        "Plucky Gary",
    );
    // lsof can't say where the process is.
    sys.answer_ok("lsof", &["-a", "-d", "cwd", "-F", "pn", "-p", "10"], "");
    assert_eq!(Codex.live(&sys)[0].cwd, PathBuf::from("/w/r"));
}

#[test]
fn where_a_session_ran_is_in_its_rollout() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    let fake = FakeCodex { sys: &sys };
    fake.rollout(
        "2026/11/26",
        "aaa",
        Path::new("/w/a"),
        "2026-11-27T03:53:30.908Z",
        "cli",
        "Plucky Gary",
    );
    fake.rollout(
        "2025/01/02",
        "old",
        Path::new("/w/o"),
        "2025-01-02T00:00:00.000Z",
        "cli",
        "Plucky Gary",
    );
    assert_eq!(Codex.session_cwd(&sys, "aaa"), Some("/w/a".into()));
    assert_eq!(Codex.session_cwd(&sys, "old"), Some("/w/o".into()));
    assert_eq!(Codex.session_cwd(&sys, "nope"), None);
    assert_eq!(Codex.session_cwd(&sys, "../x"), None);
}

#[test]
fn the_late_id_is_the_rollout_of_the_launch() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    let fake = FakeCodex { sys: &sys };
    let dir = t.path().join("wt");
    std::fs::create_dir_all(&dir).unwrap();
    let since = unix_secs("2026-11-27T03:53:00Z").unwrap();
    let r = |day, id, cwd: &Path, at, source, codename| {
        fake.rollout(day, id, cwd, at, source, codename);
    };
    let gary = "Plucky Gary";
    // Before the launch, an exec run, another directory, another codename
    // (a later session there, yo's or not), more than a day later: none.
    r(
        "2026/11/26",
        "before",
        &dir,
        "2026-11-27T03:40:00.000Z",
        "cli",
        gary,
    );
    r(
        "2026/11/26",
        "exec",
        &dir,
        "2026-11-27T03:53:10.000Z",
        "exec",
        gary,
    );
    r(
        "2026/11/26",
        "other",
        Path::new("/w/x"),
        "2026-11-27T03:53:05.000Z",
        "cli",
        gary,
    );
    r(
        "2026/11/26",
        "unrelated",
        &dir,
        "2026-11-27T03:53:15.000Z",
        "cli",
        "Sleepy Osprey",
    );
    r(
        "2026/11/29",
        "late",
        &dir,
        "2026-11-29T03:53:20.000Z",
        "cli",
        gary,
    );
    let got = |codename: &str, taken: &[&str]| {
        let taken: Vec<String> = taken.iter().map(|s| s.to_string()).collect();
        let l = Launch {
            dir: &dir,
            since,
            codename,
        };
        Codex.reported_id(&sys, &l, &taken)
    };
    assert_eq!(got(gary, &[]), None, "nothing is this launch's yet");
    // Two with its codename: the earlier first, each to one record.
    r(
        "2026/11/27",
        "second",
        &dir,
        "2026-11-27T03:54:00.000Z",
        "cli",
        gary,
    );
    r(
        "2026/11/26",
        "first",
        &dir,
        "2026-11-27T03:53:20.000Z",
        "cli",
        gary,
    );
    assert_eq!(got(gary, &[]).as_deref(), Some("first"));
    assert_eq!(got(gary, &["first"]).as_deref(), Some("second"));
    assert_eq!(got(gary, &["first", "second"]), None);
    assert_eq!(got("", &[]), None, "no codename, nothing to tie it to");
    // A path that resolves to the same directory counts.
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    let l = Launch {
        dir: &link,
        since,
        codename: gary,
    };
    assert_eq!(Codex.reported_id(&sys, &l, &[]).as_deref(), Some("first"));
}

#[test]
fn a_rollout_that_isnt_one_is_skipped() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    let d = t.path().join(".codex/sessions/2026/11/26");
    std::fs::create_dir_all(&d).unwrap();
    for (n, text) in [
        ("rollout-a-x.jsonl", ""),
        ("rollout-b-x.jsonl", "not json\n"),
        (
            "rollout-c-x.jsonl",
            r#"{"type":"event_msg","payload":{"id":"c","cwd":"/w"}}"#,
        ),
        (
            "rollout-d-x.jsonl",
            r#"{"type":"session_meta","payload":{"id":"a b","cwd":"/w","timestamp":"2026-11-27T03:53:30Z","source":"cli"}}"#,
        ),
    ] {
        std::fs::write(d.join(n), text).unwrap();
    }
    let l = Launch {
        dir: Path::new("/w"),
        since: 0,
        codename: "Plucky Gary",
    };
    assert_eq!(Codex.reported_id(&sys, &l, &[]), None);
}

#[test]
fn dates() {
    assert_eq!(unix_secs("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(unix_secs("2026-11-27T03:53:30.908Z"), Some(1_795_751_610));
    assert_eq!(unix_secs("2000-02-29T12:00:00Z"), Some(951_825_600));
    for bad in [
        "2026-11-27T03:53:30",
        "2026-13-01T00:00:00Z",
        "x",
        "2026-11-27T25:00:00Z",
    ] {
        assert_eq!(unix_secs(bad), None, "{bad}");
    }
    for days in [0, 59, 60, 365, 11_016, 20_419, 30_000] {
        let (y, m, d) = civil(days);
        assert_eq!(days_from_civil(y, m, d), days);
    }
    assert_eq!(civil(20_419), (2025, 11, 27));
}

#[test]
fn any_session_end_is_a_deliberate_one() {
    // What Codex 0.172.0 sends for /quit and ctrl-d (and nothing for a
    // closed tab or a crash).
    let quit = r#"{"session_id":"01a2-b","transcript_path":"/h/.codex/sessions/r.jsonl","cwd":"/w/a","hook_event_name":"SessionEnd","reason":"other"}"#;
    assert_eq!(
        Codex.hook_event(quit),
        Some(HookEvent::End {
            id: "01a2-b".into(),
            cwd: "/w/a".into(),
            deliberate: true
        })
    );
    for other in [
        r#"{"session_id":"01a2-b","cwd":"/w/a","hook_event_name":"SessionStart"}"#,
        r#"{"session_id":"01a2-b","cwd":"/w/a"}"#,
        r#"{"session_id":"a b","cwd":"/w/a","hook_event_name":"SessionEnd"}"#,
        "",
    ] {
        assert_eq!(Codex.hook_event(other), None, "{other}");
    }
}

#[test]
fn trusting_writes_the_clone_into_codex_config() {
    let dir = TempDir::new().unwrap();
    let sys = FakeSys::new(dir.path());
    let cfg = dir.path().join(".codex/config.toml");
    let root = PathBuf::from("/w/repo");
    // No file yet: one is made.
    Prep::Trust { root: root.clone() }.run(&sys).unwrap();
    assert_eq!(
        fs::read_to_string(&cfg).unwrap(),
        "[projects.\"/w/repo\"]\ntrust_level = \"trusted\"\n"
    );
    // Once there, it isn't added twice.
    assert!(Prep::Trust { root }.run(&sys).is_err());
}
