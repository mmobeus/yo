//! `plan_down_named` and `children`: tearing down another session, from
//! outside it.

use std::sync::Mutex;

use super::*;
use crate::runner::{Pane, Tabs};

/// Shows `tabs`, and says which tabs it closed.
struct Shows(Tabs, Mutex<Vec<String>>);

impl Shows {
    fn new(panes: &[(&str, &str, &str)]) -> Shows {
        let panes: Vec<Pane> = panes
            .iter()
            .map(|(tty, tab, title)| Pane {
                tty: (*tty).into(),
                tab: (*tab).into(),
                title: (*title).into(),
            })
            .collect();
        Shows(
            Tabs {
                ttys: panes.iter().map(|p| p.tty.clone()).collect(),
                tabs: panes.iter().map(|p| p.tab.clone()).collect(),
                panes,
            },
            Mutex::default(),
        )
    }
}

impl crate::runner::Runner for Shows {
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
    fn close_own_tab(&self, _: &dyn crate::sys::Sys, _: &str, _: Option<&str>) -> bool {
        unreachable!("another session's tab is never closed as yo's own")
    }
    fn close_tab(&self, _: &dyn crate::sys::Sys, tab: &str) -> bool {
        self.1.lock().unwrap().push(tab.to_string());
        true
    }
    fn tabs(&self, _: &dyn crate::sys::Sys) -> Option<Tabs> {
        Some(self.0.clone())
    }
}

const KEY: &str = "src__api-worktrees__feat";

/// `running`, with a name and title on its record, and its harness saying
/// `status` (none: it doesn't say).
fn named(t: &T, status: Option<&str>) -> Entry {
    running(t);
    let mut e = t
        .ctx()
        .ledger()
        .all()
        .into_iter()
        .find(|e| e.key.0 == KEY)
        .unwrap();
    e.record.name = Some("feat".into());
    e.record.title = Some("[api] feat".into());
    t.ctx().ledger().write(&mut e).unwrap();
    let status = status
        .map(|s| format!(r#","status":"{s}""#))
        .unwrap_or_default();
    fs::write(
        t.home.join(".claude/sessions/30.json"),
        format!(
            r#"{{"pid":30,"sessionId":"s1","cwd":"{}","kind":"interactive"{status}}}"#,
            t.wt().display()
        ),
    )
    .unwrap();
    e
}

fn end_s1() -> Action {
    Action::EndSession {
        harness: "claude".into(),
        id: "s1".into(),
        pid: 30,
        after_close: false,
    }
}

#[test]
fn an_idle_session_is_ended_its_worktree_removed_and_its_tab_closed() {
    let t = T::new();
    let e = named(&t, Some("idle"));
    // yo runs in some other tab, not this session's.
    t.sys.set_env("ITERM_SESSION_ID", "w0t2p0:ME");
    let shows = Shows::new(&[("/dev/ttys009", "pty-W/40", "[api] feat")]);
    let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(other.name, "feat");
    assert_eq!(other.activity, Activity::Idle);
    assert_eq!(other.tab, OtherTab::Close("pty-W/40".into()));
    assert!(matches!(other.found, Found::Worktree { .. }));
    // Ended first, so nothing writes to the worktree as it goes; the tab
    // last, in case it's the one yo runs in.
    assert_eq!(
        plan.actions,
        [
            end_s1(),
            Action::RemoveWorktree {
                repo: t.home.join("src/api"),
                path: t.wt(),
                force: false
            },
            Action::PruneWorktrees {
                repo: t.home.join("src/api")
            },
            Action::DropRecord(Key(KEY.into())),
            Action::CloseTab {
                tab: "pty-W/40".into()
            },
        ]
    );
    let run = Plan {
        notes: vec![],
        actions: vec![plan.actions[0].clone(), plan.actions[4].clone()],
    };
    execute(&t.ctx(), &run, &shows, &mut Said(Vec::new())).unwrap();
    assert_eq!(t.sys.signals(), [(20, crate::sys::Signal::Hup)]);
    assert_eq!(*shows.1.lock().unwrap(), ["pty-W/40"]);
}

#[test]
fn one_that_may_still_be_working_is_asked_about_and_refused_with_yes() {
    let t = T::new();
    let shows = Shows::new(&[("/dev/ttys009", "pty-W/40", "[api] feat")]);
    let yes = DownOpts {
        yes: true,
        discard: false,
    };
    let e = named(&t, Some("busy"));
    let (other, _) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(other.activity, Activity::Busy);
    assert!(other.activity.working());
    let m = refused(plan_down_named(&t.ctx(), &shows, &e, yes).map(|(o, p)| (o.found, p)));
    assert_eq!(m, "refusing: feat is still working");
    // Its harness doesn't say: it may be working.
    let e = named(&t, None);
    let (other, _) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(other.activity, Activity::Unknown);
    let m = refused(plan_down_named(&t.ctx(), &shows, &e, yes).map(|(o, p)| (o.found, p)));
    assert!(
        m.contains("can't tell whether feat is still working"),
        "{m}"
    );
    // Not running: nothing to end, and --yes is fine.
    fs::remove_file(t.home.join(".claude/sessions/30.json")).unwrap();
    let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, yes).unwrap();
    assert_eq!(other.activity, Activity::NotRunning);
    assert!(!plan.actions.contains(&end_s1()));
}

#[test]
fn a_tab_is_closed_only_once_it_is_confirmed_to_be_the_sessions() {
    let t = T::new();
    let e = named(&t, Some("idle"));
    let tab = |shows: &Shows| {
        let (other, plan) = plan_down_named(&t.ctx(), shows, &e, DownOpts::default()).unwrap();
        let closes = plan
            .actions
            .iter()
            .any(|a| matches!(a, Action::CloseTab { .. }));
        assert_eq!(closes, matches!(other.tab, OtherTab::Close(_)));
        other.tab
    };
    let close = OtherTab::Close("pty-W/40".into());
    // By its title, or by its terminal (retitled by hand).
    assert_eq!(
        tab(&Shows::new(&[("/dev/ttys004", "pty-W/40", "[api] feat")])),
        close
    );
    assert_eq!(
        tab(&Shows::new(&[("/dev/ttys009", "pty-W/40", "other")])),
        close
    );
    // Hidden in its tab: a workgroup showing CLI, which has no title.
    assert_eq!(tab(&Shows::new(&[("/dev/ttys004", "pty-W/40", "")])), close);
    // Its terminal shown nowhere (running outside iTerm), and its old tab id
    // now another session's, with another yo title: not hidden there.
    assert_eq!(
        tab(&Shows::new(&[("/dev/ttys004", "pty-W/40", "mine")])),
        OtherTab::Unconfirmed("pty-W/40".into())
    );
    // Its terminal is in another tab: that tab id is someone else's now.
    assert_eq!(
        tab(&Shows::new(&[
            ("/dev/ttys004", "pty-W/40", "mine"),
            ("/dev/ttys009", "pty-W/41", "[api] feat"),
        ])),
        OtherTab::Unconfirmed("pty-W/40".into())
    );
    assert_eq!(
        tab(&Shows::new(&[("/dev/ttys004", "pty-W/41", "")])),
        OtherTab::NotOpen
    );
    // A runner that can't say what it shows.
    let (other, plan) = plan_down_named(&t.ctx(), &DryLaunch, &e, DownOpts::default()).unwrap();
    assert_eq!(other.tab, OtherTab::CantTell);
    assert!(
        !plan
            .actions
            .iter()
            .any(|a| matches!(a, Action::CloseTab { .. }))
    );

    // Not running, after iTerm restarted: its old tab id is another tab's,
    // with another title. Left open.
    fs::remove_file(t.home.join(".claude/sessions/30.json")).unwrap();
    assert_eq!(
        tab(&Shows::new(&[("/dev/ttys004", "pty-W/40", "mine")])),
        OtherTab::Unconfirmed("pty-W/40".into())
    );
    // Ended, its tab still open at a shell: closed, by its title.
    assert_eq!(
        tab(&Shows::new(&[("/dev/ttys009", "pty-W/40", "[api] feat")])),
        close
    );
}

#[test]
fn this_session_and_a_home_are_refused() {
    let t = T::new();
    let mut e = named(&t, Some("idle"));
    let shows = Shows::new(&[]);
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "s1");
    let m = refused(
        plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).map(|(o, p)| (o.found, p)),
    );
    assert_eq!(m, "feat is this session: run yo down without a name");
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "other");
    e.record.home = true;
    e.record.agent = Some("cindy".into());
    let m = refused(
        plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).map(|(o, p)| (o.found, p)),
    );
    assert!(m.starts_with("feat is cindy's home"), "{m}");
}

#[test]
fn only_a_yo_worktree_is_removed() {
    let t = T::new();
    let shows = Shows::new(&[]);
    // In place: its directory stays.
    let dir = t.home.join("src/api");
    let e = Entry {
        key: Key("id-a".into()),
        record: Record {
            worktree: dir.clone(),
            session_id: Some("a".into()),
            mode: Some(Mode::Here),
            ..Record::default()
        },
    };
    let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(
        other.found,
        Found::InPlace {
            dir: dir.clone(),
            key: Some(Key("id-a".into()))
        }
    );
    assert_eq!(plan.actions, [Action::DropRecord(Key("id-a".into()))]);
    // A worktree record for a directory that isn't under -worktrees, or
    // is gone: nothing on disk is touched.
    for wt in [dir, t.home.join("src/api-worktrees/gone")] {
        let e = Entry {
            key: Key("k".into()),
            record: Record {
                worktree: wt,
                session_id: Some("a".into()),
                ..Record::default()
            },
        };
        let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
        assert!(matches!(other.found, Found::InPlace { .. }));
        assert_eq!(plan.actions, [Action::DropRecord(Key("k".into()))]);
    }
    // Uncommitted work, with --yes: refused, as from inside.
    let t = T::new();
    let e = named(&t, Some("idle"));
    t.in_worktree(" M src/lib.rs\n");
    let yes = DownOpts {
        yes: true,
        discard: false,
    };
    let m = refused(plan_down_named(&t.ctx(), &shows, &e, yes).map(|(o, p)| (o.found, p)));
    assert!(m.contains("uncommitted changes would be lost"), "{m}");
}

#[test]
fn children_are_the_sessions_naming_this_one_as_from() {
    let t = T::new();
    // Outside a session: no "this one".
    let m = match children(&t.ctx()) {
        Err(Error::Refused(m)) => m,
        other => panic!("{other:?}"),
    };
    assert!(m.starts_with("not inside a session"), "{m}");

    let rec = |name: &str, id: &str, from: Option<&str>| {
        t.record(
            &format!("id-{name}"),
            Record {
                worktree: t.home.join("src/api"),
                session_id: Some(id.into()),
                name: Some(name.into()),
                from: from.map(str::to_string),
                ..Record::default()
            },
        );
    };
    rec("me", "m1", None);
    rec("by-name", "c1", Some("me"));
    rec("by-id", "c2", Some("m1"));
    rec("by-harness-name", "c3", Some("sesh_1"));
    rec("someone-elses", "c4", Some("other"));
    rec("no-from", "c5", None);
    let dir = t.home.join(".claude/sessions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("50.json"),
        r#"{"pid":50,"sessionId":"m1","cwd":"/w","kind":"interactive","name":"sesh_1"}"#,
    )
    .unwrap();
    t.sys.add_process(50, "claude", Some("ttys020"));
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "m1");
    let mut names: Vec<String> = children(&t.ctx())
        .unwrap()
        .into_iter()
        .filter_map(|e| e.record.name)
        .collect();
    names.sort();
    assert_eq!(names, ["by-harness-name", "by-id", "by-name"]);
}

#[test]
fn a_worktree_a_session_runs_in_is_held_even_without_its_id() {
    let t = T::new();
    let shows = Shows::new(&[]);
    let yes = DownOpts {
        yes: true,
        discard: false,
    };
    // No id recorded (or another's), but claude runs in the worktree.
    let mut e = named(&t, Some("idle"));
    e.record.session_id = None;
    let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(other.activity, Activity::Unknown);
    assert_eq!(plan.actions[0], end_s1());
    let m = refused(plan_down_named(&t.ctx(), &shows, &e, yes).map(|(o, p)| (o.found, p)));
    assert!(
        m.contains("can't tell whether feat is still working"),
        "{m}"
    );

    // A Codex worker with no id yet, nothing live to match: it may be at
    // its first prompt.
    fs::remove_file(t.home.join(".claude/sessions/30.json")).unwrap();
    e.record.harness = Some("codex".into());
    let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(other.activity, Activity::Unknown);
    assert!(
        !plan
            .actions
            .iter()
            .any(|a| matches!(a, Action::EndSession { .. }))
    );
    refused(plan_down_named(&t.ctx(), &shows, &e, yes).map(|(o, p)| (o.found, p)));
    // Claude picks its id at launch: none recorded and none live is ended.
    e.record.harness = None;
    let (other, _) = plan_down_named(&t.ctx(), &shows, &e, yes).unwrap();
    assert_eq!(other.activity, Activity::NotRunning);
}

#[test]
fn children_that_may_be_working_or_would_lose_work_are_skipped() {
    let t = T::new();
    let shows = Shows::new(&[]);
    let mut e = named(&t, Some("busy"));
    e.record.from = Some("home".into());
    t.ctx().ledger().write(&mut e).unwrap();
    t.record(
        "id-home",
        Record {
            worktree: t.home.join("src/api"),
            session_id: Some("h1".into()),
            name: Some("home".into()),
            mode: Some(Mode::Here),
            ..Record::default()
        },
    );
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "h1");
    let Children { go, skipped } = plan_down_children(&t.ctx(), &shows, false).unwrap();
    assert!(go.is_empty());
    assert_eq!(
        skipped,
        [Skipped {
            name: "feat".into(),
            why: Skip::Working
        }]
    );
    // Done, but dirty: skipped without --discard, torn down with it.
    fs::remove_file(t.home.join(".claude/sessions/30.json")).unwrap();
    t.in_worktree(" M src/lib.rs\n");
    let Children { skipped, .. } = plan_down_children(&t.ctx(), &shows, false).unwrap();
    assert_eq!(
        skipped[0].why,
        Skip::Uncommitted(vec![" M src/lib.rs".into()])
    );
    let Children { go, skipped } = plan_down_children(&t.ctx(), &shows, true).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(go[0].0.name, "feat");
    assert!(go[0].1.actions.contains(&Action::RemoveWorktree {
        repo: t.home.join("src/api"),
        path: t.wt(),
        force: true,
    }));
}

#[test]
fn started_from_matches_a_session_however_it_is_named() {
    let t = T::new();
    let rec = |name: &str, id: &str, from: Option<&str>| {
        t.record(
            &format!("id-{name}"),
            Record {
                worktree: t.home.join("src/api"),
                session_id: Some(id.into()),
                name: Some(name.into()),
                from: from.map(str::to_string),
                ..Record::default()
            },
        );
    };
    // The home: yo name "home", id m1, Claude name sesh_1 (live).
    rec("home", "m1", None);
    rec("by-name", "c1", Some("home"));
    rec("by-id", "c2", Some("m1"));
    rec("by-harness-name", "c3", Some("sesh_1"));
    rec("someone-elses", "c4", Some("other"));
    let dir = t.home.join(".claude/sessions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("50.json"),
        r#"{"pid":50,"sessionId":"m1","cwd":"/w","kind":"interactive","name":"sesh_1"}"#,
    )
    .unwrap();
    t.sys.add_process(50, "claude", Some("ttys020"));
    let names = |q: &str| {
        let mut v: Vec<String> = crate::session::started_from(&t.ctx(), q)
            .into_iter()
            .filter_map(|e| e.record.name)
            .collect();
        v.sort();
        v
    };
    for q in ["m1", "home", "sesh_1"] {
        assert_eq!(names(q), ["by-harness-name", "by-id", "by-name"], "{q}");
    }
    assert_eq!(names("other"), ["someone-elses"]);
}

#[test]
fn the_session_yo_runs_in_isnt_found_by_directory() {
    let t = T::new();
    let shows = Shows::new(&[]);
    // A record with no id for the worktree yo itself runs in.
    let mut e = named(&t, Some("idle"));
    e.record.session_id = None;
    t.sys.set_env("CLAUDE_CODE_SESSION_ID", "s1");
    let m = refused(
        plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).map(|(o, p)| (o.found, p)),
    );
    assert!(m.starts_with("feat is this session"), "{m}");
    // Another session in there too: that one holds it, and yo isn't ended.
    let dir = t.home.join(".claude/sessions");
    fs::write(
        dir.join("31.json"),
        format!(
            r#"{{"pid":31,"sessionId":"s2","cwd":"{}","kind":"interactive"}}"#,
            t.wt().display()
        ),
    )
    .unwrap();
    t.sys.add_process(31, "claude", Some("ttys011"));
    let (other, plan) = plan_down_named(&t.ctx(), &shows, &e, DownOpts::default()).unwrap();
    assert_eq!(other.activity, Activity::Unknown);
    assert!(plan.actions.contains(&Action::EndSession {
        harness: "claude".into(),
        id: "s2".into(),
        pid: 31,
        after_close: false,
    }));
    assert!(!plan.actions.contains(&end_s1()));
}
