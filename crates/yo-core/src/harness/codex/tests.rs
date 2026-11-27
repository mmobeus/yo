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
fn only_models_are_available() {
    let c = Codex.capabilities();
    assert!(c.models);
    assert!(!c.personas && !c.settings_layer && !c.memory && !c.ledger_hooks && !c.remote_control);
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
    );
    fake.rollout(
        "2025/01/02",
        "old",
        Path::new("/w/o"),
        "2025-01-02T00:00:00.000Z",
        "cli",
    );
    assert_eq!(Codex.session_cwd(&sys, "aaa"), Some("/w/a".into()));
    assert_eq!(Codex.session_cwd(&sys, "old"), Some("/w/o".into()));
    assert_eq!(Codex.session_cwd(&sys, "nope"), None);
    assert_eq!(Codex.session_cwd(&sys, "../x"), None);
}

#[test]
fn the_late_id_is_the_first_new_tui_session_in_its_directory() {
    let t = TempDir::new().unwrap();
    let sys = FakeSys::new(t.path());
    let fake = FakeCodex { sys: &sys };
    let dir = t.path().join("wt");
    std::fs::create_dir_all(&dir).unwrap();
    let since = unix_secs("2026-11-27T03:53:00Z").unwrap();
    // Before the launch, an exec run, another directory: none of them.
    fake.rollout(
        "2026/11/26",
        "before",
        &dir,
        "2026-11-27T03:40:00.000Z",
        "cli",
    );
    fake.rollout(
        "2026/11/26",
        "exec",
        &dir,
        "2026-11-27T03:53:10.000Z",
        "exec",
    );
    fake.rollout(
        "2026/11/26",
        "other",
        Path::new("/w/x"),
        "2026-11-27T03:53:05.000Z",
        "cli",
    );
    // Two launched in the same directory: the earlier first.
    fake.rollout(
        "2026/11/27",
        "second",
        &dir,
        "2026-11-27T03:54:00.000Z",
        "cli",
    );
    fake.rollout(
        "2026/11/26",
        "first",
        &dir,
        "2026-11-27T03:53:20.000Z",
        "cli",
    );
    let got = |taken: &[&str]| {
        let taken: Vec<String> = taken.iter().map(|s| s.to_string()).collect();
        Codex.reported_id(&sys, &dir, since, &taken)
    };
    assert_eq!(got(&[]).as_deref(), Some("first"));
    assert_eq!(got(&["first"]).as_deref(), Some("second"));
    assert_eq!(got(&["first", "second"]), None);
    // A path that resolves to the same directory counts.
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    assert_eq!(
        Codex.reported_id(&sys, &link, since, &[]).as_deref(),
        Some("first")
    );
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
    assert_eq!(Codex.reported_id(&sys, Path::new("/w"), 0, &[]), None);
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
