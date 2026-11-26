//! The session ledger: one JSON file per session in
//! `$XDG_STATE_HOME/yo/sessions/`, keyed by the worktree path (or by the
//! session id, for a session in place). Same files as bash writes, plus
//! `machine`, `harness` and `runner`.
//!
//! What's here so far: reading, writing back, dropping, liveness
//! ([`state_of`]), the hook Claude's `SessionStart`/`SessionEnd` call, and
//! `sync` (what `yo sessions` runs first). `put`, `adopt`, `retitle` and
//! `get` arrive with the commands that use them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::harness::LiveSession;
use crate::machine::MachineId;
use crate::paths::Paths;
use crate::registry::Mode;
use crate::runner::RunnerKind;
use crate::sys::Sys;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub worktree: PathBuf,
    /// The harness's id; not always known at launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// `owner/repo` of a guest repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// An agent's home.
    #[serde(default, skip_serializing_if = "is_false")]
    pub home: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_control: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// Keyed by the session id (a session in place), not the worktree.
    #[serde(default, skip_serializing_if = "is_false")]
    pub by_id: bool,
    /// Absent: claude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Absent: this machine. Written on every write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<MachineId>,
    /// Absent: iterm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerKind>,
    /// Unix seconds, as bash writes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// Keys yo doesn't know survive a rewrite.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Record {
    /// The mode a record means, for records written before `mode` was.
    pub fn mode_or_default(&self) -> Mode {
        self.mode.unwrap_or(if self.by_id {
            Mode::Here
        } else {
            Mode::Worktree
        })
    }

    fn id(&self) -> Option<&str> {
        self.session_id.as_deref().filter(|s| !s.is_empty())
    }
}

/// A ledger file's name, without `.json`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key(pub String);

impl Key {
    /// A worktree's key: its path under `HOME` (else from the root), with
    /// `/` as `__`.
    pub fn for_worktree(home: &Path, wt: &Path) -> Key {
        let w = wt.to_string_lossy();
        let h = home.to_string_lossy();
        let rel = match w.strip_prefix(&format!("{h}/")) {
            Some(r) if !h.is_empty() => r.to_string(),
            _ => w.trim_start_matches('/').to_string(),
        };
        Key(rel.replace('/', "__"))
    }

    /// A session in place: keyed by its id.
    pub fn for_id(id: &str) -> Key {
        Key(format!("id-{id}"))
    }
}

/// A record and the file it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: Key,
    pub record: Record,
}

/// What a session is doing, as far as the ledger and the live sessions say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running {
        pid: u32,
    },
    /// Resumable.
    Ready,
    /// Quit deliberately (`/exit`): `yo resume NAME` still reopens it;
    /// `resume-all` skips it.
    Ended,
    /// Its directory is gone.
    Gone,
    /// No session id recorded.
    NoId,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Running { .. } => "running",
            State::Ready => "ready",
            State::Ended => "ended",
            State::Gone => "gone",
            State::NoId => "no-id",
        }
    }
}

/// bash's `status_of`.
pub fn state_of(rec: &Record, live: &[LiveSession]) -> State {
    if let Some(l) = rec.id().and_then(|id| live.iter().find(|l| l.id == id)) {
        return State::Running { pid: l.pid };
    }
    if !rec.worktree.is_dir() {
        State::Gone
    } else if rec.id().is_none() {
        State::NoId
    } else if rec.ended_at.is_some_and(|t| t != 0) {
        State::Ended
    } else {
        State::Ready
    }
}

/// A registered clone, for [`Ledger::sync`].
#[derive(Clone, Debug)]
pub struct SyncClone {
    pub path: PathBuf,
    pub repo: String,
    pub title_prefix: String,
    pub profile: Option<String>,
}

/// A live session `sync` recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adopted {
    pub name: String,
    pub id: String,
}

pub struct Ledger {
    sys: Arc<dyn Sys>,
    paths: Paths,
}

impl Ledger {
    pub fn new(sys: Arc<dyn Sys>, paths: Paths) -> Ledger {
        Ledger { sys, paths }
    }

    pub fn dir(&self) -> PathBuf {
        self.paths.sessions()
    }

    /// Every record, by file name. Files that aren't records are skipped.
    pub fn all(&self) -> Vec<Entry> {
        let Ok(rd) = fs::read_dir(self.dir()) else {
            return Vec::new();
        };
        let mut out: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let key = name.strip_suffix(".json")?.to_string();
                let text = fs::read_to_string(e.path()).ok()?;
                let record = serde_json::from_str(&text).ok()?;
                Some(Entry {
                    key: Key(key),
                    record,
                })
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Writes a record back to its own file (even when its session id has
    /// changed since it was read), stamping `updatedAt` and `machine`.
    pub fn write(&self, e: &mut Entry) -> Result<()> {
        e.record.updated_at = Some(self.sys.unix_now());
        e.record.machine = Some(MachineId::get(&*self.sys, &self.paths)?);
        // Sorted keys and two-space indents, as bash's json.dump writes.
        let value =
            serde_json::to_value(&e.record).map_err(|err| Error::Setting(err.to_string()))?;
        let sorted: BTreeMap<String, Value> = match value {
            Value::Object(m) => m.into_iter().collect(),
            _ => BTreeMap::new(),
        };
        let text =
            serde_json::to_string_pretty(&sorted).map_err(|err| Error::Setting(err.to_string()))?;
        crate::write_atomic(
            &self.dir().join(format!("{}.json", e.key.0)),
            text.as_bytes(),
        )
    }

    /// Removes a record. `false` when there was none.
    pub fn drop(&self, key: &Key) -> Result<bool> {
        let f = self.dir().join(format!("{}.json", key.0));
        match fs::remove_file(&f) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::io(f, e)),
        }
    }

    /// Removes a worktree's record.
    pub fn drop_dir(&self, worktree: &Path) -> Result<bool> {
        self.drop(&Key::for_worktree(&self.sys.home(), worktree))
    }

    /// The ledger hook: Claude's `SessionStart` and `SessionEnd`, with the
    /// hook's JSON payload. It never fails the caller: a ledger hiccup must
    /// never get in the way of a session.
    ///
    /// The record it updates is the one `YO_SESSION_KEY` names (a session yo
    /// launched), else every record whose worktree holds the payload's
    /// `cwd` (a session in place only by its own id). `SessionStart`
    /// records the session id and clears `endedAt`; `SessionEnd` with
    /// `prompt_input_exit` (`/exit`, ctrl-d) sets `endedAt`. Any other end
    /// (a closed tab, a crash) leaves it resumable.
    pub fn hook(&self, payload: &str) {
        let Ok(d) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("");
        let (cwd, sid) = (s("cwd"), s("session_id"));
        if sid.is_empty() {
            return;
        }
        let event = d
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or("SessionStart");
        let key = self.sys.env_nonempty("YO_SESSION_KEY");
        for mut e in self.all() {
            match &key {
                Some(k) if e.key.0 != *k => continue,
                Some(_) => {}
                None => {
                    if !owns(&e.record.worktree, Path::new(cwd))
                        || (e.record.by_id && e.record.id() != Some(sid))
                    {
                        continue;
                    }
                }
            }
            if event == "SessionEnd" {
                if s("reason") == "prompt_input_exit" && e.record.id() == Some(sid) {
                    e.record.ended_at = Some(self.sys.unix_now());
                    let _ = self.write(&mut e);
                }
            } else {
                let mut changed = e.record.id() != Some(sid);
                e.record.session_id = Some(sid.to_string());
                if e.record.ended_at.take().is_some() {
                    changed = true; // resumed after a deliberate exit
                }
                if changed {
                    let _ = self.write(&mut e);
                }
            }
        }
    }

    /// Records live sessions running in a registered clone's worktrees
    /// (`<clone>-worktrees/<name>`) that the ledger doesn't know, or knows
    /// under another id or as ended. Returns what it recorded.
    pub fn sync(&self, clones: &[SyncClone], live: &[LiveSession]) -> Result<Vec<Adopted>> {
        let home = self.sys.home();
        let mut recs: BTreeMap<Key, Record> =
            self.all().into_iter().map(|e| (e.key, e.record)).collect();
        let mut live: Vec<&LiveSession> = live.iter().collect();
        live.sort_by_key(|l| l.updated_at);
        let mut adopted = Vec::new();
        for l in live {
            let cwd = l.cwd.to_string_lossy();
            for c in clones {
                let clone = c.path.to_string_lossy();
                let root = format!("{}-worktrees/", clone.trim_end_matches('/'));
                let Some(rest) = cwd.strip_prefix(&root) else {
                    continue;
                };
                let name = rest.split('/').next().unwrap_or("");
                let wt = PathBuf::from(format!("{root}{name}"));
                if name.is_empty() || !wt.join(".git").exists() {
                    continue;
                }
                let key = Key::for_worktree(&home, &wt);
                let rec = recs.get(&key).cloned().unwrap_or_else(|| Record {
                    worktree: wt.clone(),
                    repo: Some(c.repo.clone()),
                    name: Some(name.to_string()),
                    title: Some(format!("{}{name}", c.title_prefix)),
                    mode: Some(Mode::Worktree),
                    profile: c.profile.clone().filter(|p| !p.is_empty()),
                    harness: Some(l.harness.clone()),
                    created_at: Some(self.sys.unix_now()),
                    ..Record::default()
                });
                if rec.id() != Some(l.id.as_str()) || rec.ended_at.is_some() {
                    let mut e = Entry { key, record: rec };
                    e.record.session_id = Some(l.id.clone());
                    e.record.ended_at = None;
                    self.write(&mut e)?;
                    recs.insert(e.key, e.record);
                    adopted.push(Adopted {
                        name: name.to_string(),
                        id: l.id.clone(),
                    });
                }
            }
        }
        Ok(adopted)
    }
}

/// Whether `cwd` is `wt` or inside it.
fn owns(wt: &Path, cwd: &Path) -> bool {
    !wt.as_os_str().is_empty() && cwd.starts_with(wt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, TempDir};

    struct T {
        _dir: TempDir,
        home: PathBuf,
        sys: Arc<FakeSys>,
        ledger: Ledger,
    }

    fn setup() -> T {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let sys = Arc::new(FakeSys::new(&home));
        sys.push_ids(["m1"]);
        let paths = Paths::new(&*sys);
        let ledger = Ledger::new(sys.clone(), paths);
        T {
            _dir: dir,
            home,
            sys,
            ledger,
        }
    }

    impl T {
        fn put(&self, key: &str, rec: Record) {
            let mut e = Entry {
                key: Key(key.into()),
                record: rec,
            };
            self.ledger.write(&mut e).unwrap();
        }

        fn get(&self, key: &str) -> Record {
            self.ledger
                .all()
                .into_iter()
                .find(|e| e.key.0 == key)
                .unwrap()
                .record
        }

        fn worktree(&self, rel: &str) -> PathBuf {
            let p = self.home.join(rel);
            fs::create_dir_all(p.join(".git")).unwrap();
            p
        }
    }

    fn live(id: &str, pid: u32, cwd: &Path, updated: i64) -> LiveSession {
        LiveSession {
            harness: "claude".into(),
            id: id.into(),
            pid,
            cwd: cwd.to_path_buf(),
            name: None,
            updated_at: updated,
        }
    }

    #[test]
    fn keys() {
        let h = Path::new("/Users/me");
        assert_eq!(
            Key::for_worktree(h, Path::new("/Users/me/dev/a-worktrees/x")).0,
            "dev__a-worktrees__x"
        );
        assert_eq!(Key::for_worktree(h, Path::new("/srv/a")).0, "srv__a");
        assert_eq!(Key::for_id("abc").0, "id-abc");
    }

    #[test]
    fn written_as_bash_writes_with_machine_and_unknown_keys_kept() {
        let t = setup();
        let dir = t.ledger.dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("k.json"),
            r#"{"worktree": "/w", "sessionId": "s", "createdAt": 5, "home": true, "futureKey": [1]}"#,
        )
        .unwrap();
        let mut e = t.ledger.all().pop().unwrap();
        assert!(e.record.home);
        t.ledger.write(&mut e).unwrap();
        let text = fs::read_to_string(dir.join("k.json")).unwrap();
        assert_eq!(
            text,
            "{\n  \"createdAt\": 5,\n  \"futureKey\": [\n    1\n  ],\n  \"home\": true,\n  \"machine\": \"m1\",\n  \"sessionId\": \"s\",\n  \"updatedAt\": 1795683262,\n  \"worktree\": \"/w\"\n}"
        );
    }

    #[test]
    fn unreadable_files_are_skipped_and_drop() {
        let t = setup();
        let dir = t.ledger.dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("bad.json"), "{").unwrap();
        fs::write(dir.join("noworktree.json"), "{}").unwrap();
        fs::write(dir.join("other.txt"), "{}").unwrap();
        t.put(
            "good",
            Record {
                worktree: "/w".into(),
                ..Record::default()
            },
        );
        let keys: Vec<String> = t.ledger.all().into_iter().map(|e| e.key.0).collect();
        assert_eq!(keys, ["good"]);
        assert!(t.ledger.drop(&Key("good".into())).unwrap());
        assert!(!t.ledger.drop(&Key("good".into())).unwrap());
    }

    #[test]
    fn states() {
        let t = setup();
        let wt = t.worktree("w");
        let rec = |sid: Option<&str>, ended: Option<i64>, dir: &Path| Record {
            worktree: dir.to_path_buf(),
            session_id: sid.map(str::to_string),
            ended_at: ended,
            ..Record::default()
        };
        let l = [live("s1", 42, &wt, 0)];
        assert_eq!(
            state_of(&rec(Some("s1"), Some(9), Path::new("/gone")), &l),
            State::Running { pid: 42 }
        );
        assert_eq!(
            state_of(&rec(Some("s2"), None, Path::new("/gone")), &l),
            State::Gone
        );
        assert_eq!(state_of(&rec(None, None, &wt), &l), State::NoId);
        assert_eq!(state_of(&rec(Some(""), None, &wt), &l), State::NoId);
        assert_eq!(state_of(&rec(Some("s2"), Some(9), &wt), &l), State::Ended);
        assert_eq!(state_of(&rec(Some("s2"), None, &wt), &l), State::Ready);
    }

    #[test]
    fn hook_start_and_end() {
        let t = setup();
        let wt = t.worktree("a-worktrees/x");
        t.put(
            "x",
            Record {
                worktree: wt.clone(),
                session_id: Some("old".into()),
                ended_at: Some(3),
                ..Record::default()
            },
        );
        let cwd = wt.join("sub");
        let start = format!(
            r#"{{"hook_event_name":"SessionStart","session_id":"new","cwd":"{}"}}"#,
            cwd.display()
        );
        t.ledger.hook(&start);
        let r = t.get("x");
        assert_eq!((r.session_id.as_deref(), r.ended_at), (Some("new"), None));

        // A closed tab: still resumable.
        let end = |reason: &str| {
            format!(
                r#"{{"hook_event_name":"SessionEnd","session_id":"new","cwd":"{}","reason":"{reason}"}}"#,
                cwd.display()
            )
        };
        t.ledger.hook(&end("other"));
        assert_eq!(t.get("x").ended_at, None);
        // /exit: ended.
        t.ledger.hook(&end("prompt_input_exit"));
        assert_eq!(t.get("x").ended_at, Some(1_795_683_262));
    }

    #[test]
    fn hook_by_key_and_by_id() {
        let t = setup();
        let wt = t.worktree("clone");
        t.put(
            "id-a",
            Record {
                worktree: wt.clone(),
                session_id: Some("a".into()),
                by_id: true,
                ..Record::default()
            },
        );
        t.put(
            "id-b",
            Record {
                worktree: wt.clone(),
                session_id: Some("b".into()),
                by_id: true,
                ..Record::default()
            },
        );
        t.put(
            "other",
            Record {
                worktree: "/elsewhere".into(),
                ..Record::default()
            },
        );
        // In place, only the record with its own id matches: nothing changes.
        let p = format!(r#"{{"session_id":"a","cwd":"{}"}}"#, wt.display());
        t.ledger.hook(&p);
        assert_eq!(t.get("id-b").session_id.as_deref(), Some("b"));
        // YO_SESSION_KEY names the record outright, wherever the cwd is.
        t.sys.set_env("YO_SESSION_KEY", "other");
        t.ledger.hook(r#"{"session_id":"c","cwd":"/nowhere"}"#);
        assert_eq!(t.get("other").session_id.as_deref(), Some("c"));
        assert_eq!(t.get("id-a").session_id.as_deref(), Some("a"));
    }

    #[test]
    fn hook_never_fails() {
        let t = setup();
        t.put(
            "x",
            Record {
                worktree: "/w".into(),
                ..Record::default()
            },
        );
        for p in [
            "",
            "null",
            "[]",
            "{",
            r#"{"session_id": 5}"#,
            r#"{"session_id":"s","cwd":null}"#,
            "\u{0}",
        ] {
            t.ledger.hook(p);
        }
        assert_eq!(t.get("x").session_id, None);
    }

    #[test]
    fn sync_adopts_live_sessions_in_worktrees() {
        let t = setup();
        let clone = t.home.join("dev/a");
        let x = t.worktree("dev/a-worktrees/x");
        let y = t.worktree("dev/a-worktrees/y");
        fs::create_dir_all(t.home.join("dev/a-worktrees/noclone")).unwrap();
        let clones = [SyncClone {
            path: clone.clone(),
            repo: "a".into(),
            title_prefix: "[a] ".into(),
            profile: Some("p".into()),
        }];
        // y is known under an older id; x isn't known at all.
        t.put(
            "dev__a-worktrees__y",
            Record {
                worktree: y.clone(),
                session_id: Some("old".into()),
                name: Some("y".into()),
                ..Record::default()
            },
        );
        let l = [
            live("sx", 1, &x.join("deep"), 2),
            live("sy", 2, &y, 1),
            live("sn", 3, &t.home.join("dev/a-worktrees/noclone"), 3),
            live("sc", 4, &clone, 4),
        ];
        let got = t.ledger.sync(&clones, &l).unwrap();
        assert_eq!(
            got,
            [
                Adopted {
                    name: "y".into(),
                    id: "sy".into()
                },
                Adopted {
                    name: "x".into(),
                    id: "sx".into()
                }
            ]
        );
        let rx = t.get("dev__a-worktrees__x");
        assert_eq!(rx.title.as_deref(), Some("[a] x"));
        assert_eq!(rx.profile.as_deref(), Some("p"));
        assert_eq!(rx.mode, Some(Mode::Worktree));
        assert_eq!(rx.harness.as_deref(), Some("claude"));
        assert_eq!(
            t.get("dev__a-worktrees__y").session_id.as_deref(),
            Some("sy")
        );
        // Nothing new the second time.
        assert!(t.ledger.sync(&clones, &l).unwrap().is_empty());
    }
}
