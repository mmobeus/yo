//! The session ledger: one JSON file per session in
//! `$XDG_STATE_HOME/yo/sessions/`, keyed by the worktree path (or by the
//! session id, for a session in place). Same files as bash writes, plus
//! `machine`, `harness` and `runner`.
//!
//! What's here so far: reading, writing back, dropping, liveness
//! ([`state_of`]), the hook a harness's session start and end call (its payload made
//! neutral by [`crate::harness::Harness::hook_event`]), and
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
    /// The session it reports back to (`--from`), as it was named: the
    /// session that started it, which `yo down --children` tears it down
    /// from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
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
    /// The tab the runner last opened it in (iTerm: `<window id>/<tab id>`),
    /// so yo can tell a session with a tab from one whose tab is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
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

    /// The recorded tab, if it's one a runner could have given.
    pub fn tab(&self) -> Option<&str> {
        self.tab.as_deref().filter(|t| crate::runner::valid_tab(t))
    }

    /// The session id, if it's one yo can use: a record edited by hand to
    /// hold something else counts as having none, so it never reaches a
    /// command line.
    pub fn id(&self) -> Option<&str> {
        self.session_id
            .as_deref()
            .filter(|s| crate::harness::valid_session_id(s))
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

/// How long after launch a session whose harness reports its id is taken
/// to be stuck without one, in seconds.
const WAIT: i64 = 30;

/// Whether a record without an id is likely stuck at a prompt in its tab:
/// its harness reports ids (Codex records a session once its first message
/// goes in), and it's had a while since launch to do so. Anything that
/// stops the harness before that message (its trust prompt, its hook
/// review, a login) leaves it here.
pub fn waiting(rec: &Record, state: State, now: i64) -> bool {
    let reports = crate::harness::all().iter().any(|h| {
        h.name() == rec.harness.as_deref().unwrap_or(crate::harness::DEFAULT)
            && h.ids() == crate::harness::IdPolicy::Reported
    });
    state == State::NoId && reports && rec.created_at.is_some_and(|t| now - t > WAIT)
}

/// A session starting or ending, as a harness's hook reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEvent {
    Start {
        id: String,
        cwd: PathBuf,
    },
    /// `deliberate`: the user quit (Claude: `/exit`, ctrl-d), so the
    /// session is ended for good rather than left resumable.
    End {
        id: String,
        cwd: PathBuf,
        deliberate: bool,
    },
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

    /// Records the tab session `key` was opened in. `false` when there's no
    /// such record.
    pub fn set_tab(&self, key: &Key, tab: &str) -> Result<bool> {
        let Some(mut e) = self.all().into_iter().find(|e| &e.key == key) else {
            return Ok(false);
        };
        e.record.tab = Some(tab.to_string());
        self.write(&mut e)?;
        Ok(true)
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

    /// The ledger hook: a harness's session starting or ending (the harness
    /// turns its own hook payload into a [`HookEvent`]). It never fails the
    /// caller: a ledger hiccup must never get in the way of a session.
    ///
    /// The record it updates is the one `YO_SESSION_KEY` names (a session yo
    /// launched), else every record whose worktree holds the event's `cwd`
    /// (a session in place only by its own id). A start records the session
    /// id and clears `endedAt`; a deliberate end sets `endedAt` (and the
    /// id, on the record `YO_SESSION_KEY` names when it has none yet). Any
    /// other end (a closed tab, a crash) leaves it resumable.
    pub fn hook(&self, ev: &HookEvent) {
        let (id, cwd) = match ev {
            HookEvent::Start { id, cwd } | HookEvent::End { id, cwd, .. } => (id.as_str(), cwd),
        };
        if !crate::harness::valid_session_id(id) {
            return;
        }
        let key = self.sys.env_nonempty("YO_SESSION_KEY");
        for mut e in self.all() {
            match &key {
                Some(k) if e.key.0 != *k => continue,
                Some(_) => {}
                None => {
                    if !owns(&e.record.worktree, cwd)
                        || (e.record.by_id && e.record.id() != Some(id))
                    {
                        continue;
                    }
                }
            }
            match ev {
                HookEvent::End { deliberate, .. } => {
                    // A harness that reports its ids can end before yo
                    // has learned the id: the record YO_SESSION_KEY names
                    // is this session's, so it learns it here.
                    let learns = key.is_some() && e.record.id().is_none();
                    if *deliberate && (learns || e.record.id() == Some(id)) {
                        e.record.session_id = Some(id.to_string());
                        e.record.ended_at = Some(self.sys.unix_now());
                        let _ = self.write(&mut e);
                    }
                }
                HookEvent::Start { .. } => {
                    let mut changed = e.record.id() != Some(id);
                    e.record.session_id = Some(id.to_string());
                    if e.record.ended_at.take().is_some() {
                        changed = true; // resumed after a deliberate exit
                    }
                    if changed {
                        let _ = self.write(&mut e);
                    }
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
        // updated_at is in each harness's own units: order within a harness.
        live.sort_by(|a, b| (&a.harness, a.updated_at).cmp(&(&b.harness, b.updated_at)));
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
                if matches!(name, "" | "." | "..") || !wt.join(".git").exists() {
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
                if rec.id() != Some(l.id.as_str()) || rec.ended_at.is_some_and(|t| t != 0) {
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

/// A session recorded from inside it (`yo adopt`).
#[derive(Clone, Debug)]
pub struct Adopt {
    pub dir: PathBuf,
    pub id: String,
    pub title: String,
    /// The title was given (`--title`), so it replaces what's recorded.
    pub title_given: bool,
    pub repo: Option<String>,
    pub name: String,
    pub profile: Option<String>,
    pub mode: Mode,
    pub harness: Option<String>,
}

impl Ledger {
    /// The record for `q`: its id or directory, else an id prefix of 8 or
    /// more, else its yo name. `Err` holds the matches when there's more
    /// than one.
    pub fn get(&self, q: &str) -> std::result::Result<Option<Entry>, Vec<Entry>> {
        let q = crate::paths::strip_trailing_slash(q);
        let all = self.all();
        let pick = |f: &dyn Fn(&Entry) -> bool| -> Vec<Entry> {
            all.iter().filter(|e| f(e)).cloned().collect()
        };
        let mut hits = pick(&|e| {
            e.record.session_id.as_deref() == Some(q) || e.record.worktree == Path::new(q)
        });
        if hits.is_empty() && q.len() >= 8 {
            hits = pick(&|e| {
                e.record
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with(q))
            });
        }
        if hits.is_empty() {
            hits = pick(&|e| e.record.name.as_deref() == Some(q));
        }
        match hits.len() {
            0 => Ok(None),
            1 => Ok(hits.pop()),
            _ => Err(hits),
        }
    }

    /// Records a session from inside it. One the ledger already holds by its
    /// id (even under an older id's key, after a `/clear`) keeps its record;
    /// a worktree's record keeps what yo wrote when it created it. Either
    /// way it's no longer ended.
    pub fn adopt(&self, a: &Adopt) -> Result<Key> {
        let all = self.all();
        let mut e = if let Some(held) = all
            .iter()
            .find(|e| e.record.session_id.as_deref() == Some(a.id.as_str()))
        {
            let mut e = held.clone();
            if a.title_given {
                e.record.title = Some(a.title.clone());
            }
            e
        } else {
            let key = match a.mode {
                Mode::Here => Key::for_id(&a.id),
                Mode::Worktree => Key::for_worktree(&self.sys.home(), &a.dir),
            };
            let nonempty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
            let mut rec = Record {
                worktree: a.dir.clone(),
                session_id: Some(a.id.clone()),
                title: Some(a.title.clone()).filter(|t| !t.is_empty()),
                repo: nonempty(&a.repo),
                name: Some(a.name.clone()).filter(|n| !n.is_empty()),
                profile: nonempty(&a.profile),
                mode: Some(a.mode),
                by_id: a.mode == Mode::Here,
                harness: a.harness.clone(),
                created_at: Some(self.sys.unix_now()),
                ..Record::default()
            };
            if let Some(old) = all.iter().find(|e| e.key == key) {
                let o = &old.record;
                let keep = |new: &mut Option<String>, old: &Option<String>| {
                    if let Some(v) = old.clone().filter(|v| !v.is_empty()) {
                        *new = Some(v);
                    }
                };
                if o.created_at.is_some_and(|t| t != 0) {
                    rec.created_at = o.created_at;
                }
                keep(&mut rec.profile, &o.profile);
                keep(&mut rec.guest, &o.guest);
                keep(&mut rec.agent, &o.agent);
                keep(&mut rec.remote_control, &o.remote_control);
                keep(&mut rec.codename, &o.codename);
                keep(&mut rec.brief, &o.brief);
                rec.home = rec.home || o.home;
                if !a.title_given {
                    keep(&mut rec.title, &o.title);
                }
            }
            Entry { key, record: rec }
        };
        e.record.ended_at = None;
        self.write(&mut e)?;
        Ok(e.key)
    }

    /// Sets the title of record `key`, else the one holding session `id`,
    /// else every worktree record holding `cwd` (a plain shell in a yo
    /// worktree). `false` when none matched.
    pub fn retitle(
        &self,
        key: Option<&str>,
        id: Option<&str>,
        cwd: &Path,
        title: &str,
    ) -> Result<bool> {
        let all = self.all();
        let mut hit: Vec<Entry> = all
            .iter()
            .filter(|e| key.is_some_and(|k| !k.is_empty() && e.key.0 == k))
            .cloned()
            .collect();
        if hit.is_empty() {
            hit = all
                .iter()
                .filter(|e| {
                    id.is_some_and(|i| !i.is_empty() && e.record.session_id.as_deref() == Some(i))
                })
                .cloned()
                .collect();
        }
        if hit.is_empty() {
            hit = all
                .iter()
                .filter(|e| !e.record.by_id && owns(&e.record.worktree, cwd))
                .cloned()
                .collect();
        }
        for e in &mut hit {
            e.record.title = Some(title.to_string());
            self.write(e)?;
        }
        Ok(!hit.is_empty())
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
            idle: None,
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
        assert_eq!(state_of(&rec(Some("a b"), None, &wt), &l), State::NoId);
        assert_eq!(state_of(&rec(Some("s2"), Some(9), &wt), &l), State::Ended);
        assert_eq!(state_of(&rec(Some("s2"), None, &wt), &l), State::Ready);
        // Python took 0 as false: not ended.
        assert_eq!(state_of(&rec(Some("s2"), Some(0), &wt), &l), State::Ready);
    }

    fn start(id: &str, cwd: &Path) -> HookEvent {
        HookEvent::Start {
            id: id.into(),
            cwd: cwd.to_path_buf(),
        }
    }

    fn end(id: &str, cwd: &Path, deliberate: bool) -> HookEvent {
        HookEvent::End {
            id: id.into(),
            cwd: cwd.to_path_buf(),
            deliberate,
        }
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
        t.ledger.hook(&start("new", &cwd));
        let r = t.get("x");
        assert_eq!((r.session_id.as_deref(), r.ended_at), (Some("new"), None));

        // A closed tab: still resumable.
        t.ledger.hook(&end("new", &cwd, false));
        assert_eq!(t.get("x").ended_at, None);
        // A deliberate quit, but by another session: not this one's end.
        t.ledger.hook(&end("other", &cwd, true));
        assert_eq!(t.get("x").ended_at, None);
        // /exit: ended.
        t.ledger.hook(&end("new", &cwd, true));
        assert_eq!(t.get("x").ended_at, Some(1_795_683_262));
    }

    #[test]
    fn hook_by_key_and_by_id() {
        let t = setup();
        let wt = t.worktree("clone");
        for (key, id) in [("id-a", "a"), ("id-b", "b")] {
            t.put(
                key,
                Record {
                    worktree: wt.clone(),
                    session_id: Some(id.into()),
                    by_id: true,
                    ..Record::default()
                },
            );
        }
        t.put(
            "other",
            Record {
                worktree: "/elsewhere".into(),
                session_id: Some("c".into()),
                ..Record::default()
            },
        );
        // In place, only the record with its own id matches: nothing changes.
        t.ledger.hook(&start("a", &wt));
        assert_eq!(t.get("id-b").session_id.as_deref(), Some("b"));
        // YO_SESSION_KEY names the record outright, wherever the cwd is.
        t.sys.set_env("YO_SESSION_KEY", "other");
        t.ledger.hook(&start("d", Path::new("/nowhere")));
        assert_eq!(t.get("other").session_id.as_deref(), Some("d"));
        assert_eq!(t.get("id-a").session_id.as_deref(), Some("a"));
        // Matched by key, a deliberate end with another id leaves it alone.
        t.ledger.hook(&end("zzz", Path::new("/nowhere"), true));
        assert_eq!(t.get("other").ended_at, None);
    }

    #[test]
    fn a_codex_session_with_no_id_a_while_on_is_waiting() {
        let rec = |harness: &str, at: i64| Record {
            harness: Some(harness.into()),
            created_at: Some(at),
            ..Record::default()
        };
        assert!(waiting(&rec("codex", 100), State::NoId, 131));
        assert!(
            !waiting(&rec("codex", 100), State::NoId, 130),
            "just launched"
        );
        assert!(!waiting(&rec("codex", 100), State::Ready, 500));
        assert!(
            !waiting(&rec("claude", 100), State::NoId, 500),
            "yo picks Claude's"
        );
        assert!(!waiting(&Record::default(), State::NoId, 500));
    }

    #[test]
    fn a_deliberate_end_before_the_id_is_learned_records_both() {
        // A Codex session quit before any yo command learned its id: the
        // record its key names takes the id with the end.
        let t = setup();
        let wt = t.worktree("w");
        t.put(
            "k",
            Record {
                worktree: wt.clone(),
                ..Record::default()
            },
        );
        t.put(
            "same-dir",
            Record {
                worktree: wt.clone(),
                ..Record::default()
            },
        );
        // Without a key, a record with no id is nobody's.
        t.ledger.hook(&end("cx", &wt, true));
        assert_eq!(t.get("k").ended_at, None);
        t.sys.set_env("YO_SESSION_KEY", "k");
        // A closed tab isn't an end, even with the key.
        t.ledger.hook(&end("cx", &wt, false));
        assert_eq!(t.get("k").session_id, None);
        t.ledger.hook(&end("cx", &wt, true));
        assert_eq!(t.get("k").session_id.as_deref(), Some("cx"));
        assert!(t.get("k").ended_at.is_some());
        assert_eq!(t.get("same-dir").session_id, None, "only the keyed record");
    }

    #[test]
    fn hook_with_no_id_does_nothing() {
        let t = setup();
        let wt = t.worktree("w");
        t.put(
            "x",
            Record {
                worktree: wt.clone(),
                ..Record::default()
            },
        );
        t.ledger.hook(&start("", &wt));
        assert_eq!(t.get("x").session_id, None);
    }

    #[test]
    fn mode_of_old_records() {
        let r = |by_id| Record {
            by_id,
            ..Record::default()
        };
        assert_eq!(r(true).mode_or_default(), Mode::Here);
        assert_eq!(r(false).mode_or_default(), Mode::Worktree);
        let mut set = r(true);
        set.mode = Some(Mode::Worktree);
        assert_eq!(set.mode_or_default(), Mode::Worktree);
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
        // z ended with /exit, and is running again under the same id.
        let z = t.worktree("dev/a-worktrees/z");
        t.put(
            "dev__a-worktrees__z",
            Record {
                worktree: z.clone(),
                session_id: Some("sz".into()),
                ended_at: Some(7),
                ..Record::default()
            },
        );
        // A cwd that climbs out of the worktree root, to a directory that's a
        // clone: never a worktree.
        fs::create_dir_all(t.home.join("dev/.git")).unwrap();
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
            live("sz", 5, &z, 5),
            live("sdots", 6, &t.home.join("dev/a-worktrees/../a"), 6),
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
                },
                Adopted {
                    name: "z".into(),
                    id: "sz".into()
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
        assert_eq!(t.get("dev__a-worktrees__z").ended_at, None);
        assert!(t.ledger.all().iter().all(|e| !e.key.0.contains("..")));
        // Nothing new the second time.
        assert!(t.ledger.sync(&clones, &l).unwrap().is_empty());
    }

    fn adopt_req(t: &T, wt: &str, id: &str, title: &str, given: bool, mode: Mode) -> Adopt {
        Adopt {
            dir: t.worktree(wt),
            id: id.into(),
            title: title.into(),
            title_given: given,
            repo: Some("api".into()),
            name: wt.into(),
            profile: None,
            mode,
            harness: Some("claude".into()),
        }
    }

    #[test]
    fn adopt_new_known_and_after_a_clear() {
        let t = setup();
        // New, in place: keyed by its id.
        let k = t
            .ledger
            .adopt(&adopt_req(&t, "x", "id1", "X", false, Mode::Here))
            .unwrap();
        assert_eq!(k.0, "id-id1");
        assert!(t.get("id-id1").by_id);

        // A worktree's record keeps what yo wrote when it created it, and is
        // no longer ended.
        let wt = t.worktree("feat");
        let key = Key::for_worktree(&t.home, &wt);
        t.put(
            &key.0,
            Record {
                worktree: wt.clone(),
                session_id: Some("old".into()),
                title: Some("[api] feat".into()),
                agent: Some("cindy".into()),
                codename: Some("Plucky Gary".into()),
                created_at: Some(5),
                ended_at: Some(9),
                ..Record::default()
            },
        );
        let k = t
            .ledger
            .adopt(&adopt_req(
                &t,
                "feat",
                "new",
                "derived",
                false,
                Mode::Worktree,
            ))
            .unwrap();
        let r = t.get(&k.0);
        assert_eq!(r.session_id.as_deref(), Some("new"));
        assert_eq!(r.title.as_deref(), Some("[api] feat"));
        assert_eq!(
            (r.agent.as_deref(), r.codename.as_deref()),
            (Some("cindy"), Some("Plucky Gary"))
        );
        assert_eq!((r.created_at, r.ended_at), (Some(5), None));

        // Already held by id: its record stays, with a given title.
        t.ledger
            .adopt(&adopt_req(
                &t,
                "elsewhere",
                "new",
                "Given",
                true,
                Mode::Here,
            ))
            .unwrap();
        assert_eq!(t.get(&k.0).title.as_deref(), Some("Given"));
        assert_eq!(t.ledger.all().len(), 2);
    }

    #[test]
    fn get_by_id_dir_prefix_and_name() {
        let t = setup();
        let rec = |id: &str, name: &str, wt: &str| Record {
            worktree: t.worktree(wt),
            session_id: Some(id.into()),
            name: Some(name.into()),
            ..Record::default()
        };
        t.put("a", rec("abcdef12-aaaa", "one", "one"));
        t.put("b", rec("abcdef12-bbbb", "two", "two"));
        let key = |r: Result<Option<Entry>, Vec<Entry>>| r.unwrap().map(|e| e.key.0);
        assert_eq!(key(t.ledger.get("abcdef12-aaaa")).as_deref(), Some("a"));
        let dir = format!("{}/", t.worktree("two").display());
        assert_eq!(key(t.ledger.get(&dir)).as_deref(), Some("b"));
        assert_eq!(key(t.ledger.get("abcdef12-b")).as_deref(), Some("b"));
        assert_eq!(key(t.ledger.get("one")).as_deref(), Some("a"));
        assert_eq!(t.ledger.get("abcdef12").unwrap_err().len(), 2);
        assert_eq!(key(t.ledger.get("abc")), None);
    }

    #[test]
    fn retitle_by_key_id_or_worktree() {
        let t = setup();
        let wt = t.worktree("feat");
        t.put(
            "w",
            Record {
                worktree: wt.clone(),
                session_id: Some("s1".into()),
                ..Record::default()
            },
        );
        t.put(
            "h",
            Record {
                worktree: wt.clone(),
                session_id: Some("s2".into()),
                by_id: true,
                ..Record::default()
            },
        );
        assert!(
            t.ledger
                .retitle(Some("h"), None, Path::new("/"), "A")
                .unwrap()
        );
        assert_eq!(t.get("h").title.as_deref(), Some("A"));
        assert!(
            t.ledger
                .retitle(None, Some("s1"), Path::new("/"), "B")
                .unwrap()
        );
        assert_eq!(t.get("w").title.as_deref(), Some("B"));
        // A shell in the worktree: its worktree record, not the in-place one.
        assert!(t.ledger.retitle(None, None, &wt.join("sub"), "C").unwrap());
        assert_eq!(
            (t.get("w").title.as_deref(), t.get("h").title.as_deref()),
            (Some("C"), Some("A"))
        );
        assert!(
            !t.ledger
                .retitle(Some(""), Some(""), Path::new("/nowhere"), "D")
                .unwrap()
        );
    }
}
