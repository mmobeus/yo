//! `resolve`, one step of the lookup at a time.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::fake::{FakeSys, TempDir};
use crate::ledger::{Key, Record};

struct T {
    _dir: TempDir,
    home: PathBuf,
    reg: Registry,
}

/// Repos `api` and `cindy-home` (a clone at `src/cindy-home`), `bob`'s home
/// by its clone's name, profile `work`, agent `ag`.
fn t() -> T {
    let dir = TempDir::new().unwrap();
    let home = dir.path().to_path_buf();
    for d in [
        "src/api",
        "src/cindy-home",
        "src/x/bob-home",
        "wt/feat",
        "wt/two",
    ] {
        fs::create_dir_all(home.join(d)).unwrap();
    }
    let h = home.display();
    let cfg = home.join("c.json");
    fs::write(
        &cfg,
        format!(
            r#"{{"profiles": {{"work": {{}}}},
                "repos": {{"api": {{"path": "{h}/src/api"}},
                          "cindy-home": {{"path": "{h}/src/cindy-home"}},
                          "bobs": {{"path": "{h}/src/x/bob-home"}}}},
                "agents": {{"ag": {{"repo": "api"}}}}}}"#
        ),
    )
    .unwrap();
    let sys = Arc::new(FakeSys::new(&home));
    let reg = Registry::load(&*sys, &cfg, &home).unwrap();
    T {
        _dir: dir,
        home,
        reg,
    }
}

fn entry(t: &T, key: &str, name: &str, id: Option<&str>, wt: &str) -> Entry {
    Entry {
        key: Key(key.into()),
        record: Record {
            worktree: t.home.join(wt),
            session_id: id.map(str::to_string),
            name: Some(name.into()),
            ..Record::default()
        },
    }
}

fn live(id: &str, pid: u32, name: Option<&str>, cwd: PathBuf) -> LiveSession {
    LiveSession {
        harness: "claude".into(),
        id: id.into(),
        pid,
        cwd,
        name: name.map(str::to_string),
        updated_at: 0,
        idle: None,
    }
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_string()).collect()
}

const ID: &str = "abcdef12-0000-4000-8000-000000000000";

#[test]
fn create_when_flags_repos_profiles_or_too_many_args() {
    let t = t();
    let es = [entry(&t, "k", "feat", Some(ID), "wt/feat")];
    let r = |a: &[&str], flags| resolve(&t.reg, &es, &[], &args(a), flags);
    assert_eq!(r(&[], false), Ok(Resolved::New));
    assert_eq!(r(&["feat"], true), Ok(Resolved::New));
    assert_eq!(r(&["api"], false), Ok(Resolved::New));
    assert_eq!(r(&["work", "x"], false), Ok(Resolved::New));
    assert_eq!(r(&["feat", "a", "b"], false), Ok(Resolved::New));
    assert_eq!(r(&["nothing"], false), Ok(Resolved::New));
}

#[test]
fn an_agent_before_anything_live() {
    let t = t();
    let l = [live(ID, 7, Some("ag"), t.home.clone())];
    assert_eq!(
        resolve(&t.reg, &[], &l, &args(&["ag"]), false),
        Ok(Resolved::Agent("ag".into()))
    );
}

#[test]
fn a_live_name_before_the_ledger() {
    let t = t();
    let es = [entry(&t, "k", "feat", Some("other"), "wt/feat")];
    let l = [live(ID, 7, Some("feat"), t.home.clone())];
    assert_eq!(
        resolve(&t.reg, &es, &l, &args(&["feat", "hi"]), false),
        Ok(Resolved::Live(l[0].clone()))
    );
    // Two live sessions with one name: ambiguous.
    let l2 = [l[0].clone(), live("x", 8, Some("feat"), t.home.clone())];
    let a = resolve(&t.reg, &es, &l2, &args(&["feat"]), false).unwrap_err();
    assert_eq!((a.live, a.entries.len()), (2, 0));
}

#[test]
fn the_ledger_by_name_id_and_prefix() {
    let t = t();
    let e = entry(&t, "k", "feat", Some(ID), "wt/feat");
    let es = [e.clone()];
    for q in ["feat", ID, "abcdef12"] {
        assert_eq!(
            resolve(&t.reg, &es, &[], &args(&[q]), false),
            Ok(Resolved::Ledger(Box::new(e.clone()), State::Ready)),
            "{q}"
        );
    }
    // A prefix shorter than 8 isn't one.
    assert_eq!(
        resolve(&t.reg, &es, &[], &args(&["abcdef1"]), false),
        Ok(Resolved::New)
    );
    // Running, by its id among the live sessions.
    let l = [live(ID, 42, None, t.home.join("wt/feat"))];
    assert_eq!(
        resolve(&t.reg, &es, &l, &args(&["feat"]), false),
        Ok(Resolved::Ledger(
            Box::new(e.clone()),
            State::Running { pid: 42 }
        ))
    );
    // Ended still reaches (it resumes).
    let mut ended = e.clone();
    ended.record.ended_at = Some(5);
    assert!(matches!(
        resolve(&t.reg, &[ended], &[], &args(&["feat"]), false),
        Ok(Resolved::Ledger(_, State::Ended))
    ));
}

#[test]
fn gone_or_no_id_falls_through() {
    let t = t();
    let gone = entry(&t, "k", "feat", Some(ID), "wt/missing");
    let noid = entry(&t, "j", "two", None, "wt/two");
    assert_eq!(
        resolve(&t.reg, &[gone, noid], &[], &args(&["feat"]), false),
        Ok(Resolved::New)
    );
}

#[test]
fn ambiguous_ledger_matches() {
    let t = t();
    let es = [
        entry(&t, "a", "feat", Some(ID), "wt/feat"),
        entry(&t, "b", "feat", Some("abcdef12-1111"), "wt/two"),
    ];
    let a = resolve(&t.reg, &es, &[], &args(&["feat"]), false).unwrap_err();
    assert_eq!(a.entries.len(), 2);
    let a = resolve(&t.reg, &es, &[], &args(&["abcdef12"]), false).unwrap_err();
    assert_eq!(a.entries.len(), 2);
    // The full id is one.
    assert!(matches!(
        resolve(&t.reg, &es, &[], &args(&[ID]), false),
        Ok(Resolved::Ledger(e, _)) if e.key.0 == "a"
    ));
}

#[test]
fn homes() {
    let t = t();
    let home = |es: &[Entry], l: &[LiveSession], q: &str| {
        resolve(&t.reg, es, l, &args(&[q]), false).unwrap()
    };
    // None there: start one. By key, and by the clone's name.
    assert_eq!(
        home(&[], &[], "cindy"),
        Resolved::Home {
            repo: "cindy-home".into(),
            target: HomeTarget::New
        }
    );
    assert!(matches!(home(&[], &[], "bob"), Resolved::Home { repo, .. } if repo == "bobs"));

    // The latest ready one, else a live one there, else a running one.
    let mut old = entry(&t, "o", "old", Some("id-old"), "src/cindy-home");
    old.record.updated_at = Some(1);
    let mut new = entry(&t, "n", "new", Some("id-new"), "src/cindy-home");
    new.record.updated_at = Some(2);
    let es = [old.clone(), new.clone()];
    assert!(matches!(home(&es, &[], "cindy"),
        Resolved::Home { target: HomeTarget::Ledger(e, State::Ready), .. } if e.key.0 == "n"));
    let stray = live("id-stray", 9, None, t.home.join("src/cindy-home"));
    assert_eq!(
        home(&es, std::slice::from_ref(&stray), "cindy"),
        Resolved::Home {
            repo: "cindy-home".into(),
            target: HomeTarget::Live(stray.clone())
        }
    );
    let running = live("id-old", 10, None, t.home.join("src/cindy-home"));
    assert!(matches!(home(&es, &[stray, running], "cindy"),
        Resolved::Home { target: HomeTarget::Ledger(e, State::Running { pid: 10 }), .. } if e.key.0 == "o"));
    // One quit with /exit isn't brought back.
    let mut ended = new;
    ended.record.ended_at = Some(3);
    assert!(matches!(
        home(&[ended], &[], "cindy"),
        Resolved::Home {
            target: HomeTarget::New,
            ..
        }
    ));
}
