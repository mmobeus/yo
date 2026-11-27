//! A fake [`Sys`] for tests: a settable clock, queued ids and picks, a
//! private environment, canned command answers with every call recorded, and
//! a process table. Built for yo-core's own tests, and for other crates'
//! tests with the `fake` feature (as a dev-dependency only).
//!
//! It makes no outside call of its own. [`TempDir`] gives a test a
//! throwaway directory to use as `HOME` and the state directory.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sys::{CommandRunner, CommandSpec, Output, ProcessTable, Signal, Sys};

/// A fake system. Everything starts empty: no variables, no processes, and
/// every command fails (exit 127) until it's given an answer.
pub struct FakeSys {
    now: Mutex<SystemTime>,
    ids: Mutex<VecDeque<String>>,
    picks: Mutex<VecDeque<usize>>,
    env: Mutex<BTreeMap<String, String>>,
    home: PathBuf,
    hostname: String,
    commands: FakeCommands,
    procs: FakeProcs,
    slept: Mutex<Vec<Duration>>,
}

impl FakeSys {
    /// A fake whose `HOME` is `home`, at 2026-11-26 08:54:22 UTC.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        FakeSys {
            now: Mutex::new(UNIX_EPOCH + Duration::from_secs(1_795_683_262)),
            ids: Mutex::new(VecDeque::new()),
            picks: Mutex::new(VecDeque::new()),
            env: Mutex::new(BTreeMap::new()),
            home: home.into(),
            hostname: "fakehost".to_string(),
            commands: FakeCommands::default(),
            procs: FakeProcs::default(),
            slept: Mutex::new(Vec::new()),
        }
    }

    pub fn set_now(&self, secs: u64) {
        *lock(&self.now) = UNIX_EPOCH + Duration::from_secs(secs);
    }

    /// Queues the next ids `new_session_id` returns.
    pub fn push_ids<I: IntoIterator<Item = S>, S: Into<String>>(&self, ids: I) {
        lock(&self.ids).extend(ids.into_iter().map(Into::into));
    }

    /// Queues the next answers `pick` gives (each taken modulo `n`).
    pub fn push_picks(&self, picks: impl IntoIterator<Item = usize>) {
        lock(&self.picks).extend(picks);
    }

    pub fn set_env(&self, key: &str, value: &str) {
        lock(&self.env).insert(key.to_string(), value.to_string());
    }

    /// Answers a command run with exactly this program and these args.
    pub fn answer(&self, program: &str, args: &[&str], out: Output) {
        lock(&self.commands.answers).push(Answer {
            program: program.to_string(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            out,
        });
    }

    /// Answers with success and this stdout.
    pub fn answer_ok(&self, program: &str, args: &[&str], stdout: &str) {
        self.answer(program, args, ok(stdout));
    }

    /// Every sleep asked for.
    pub fn slept(&self) -> Vec<Duration> {
        lock(&self.slept).clone()
    }

    /// Every command spawned (left running) so far.
    pub fn spawned(&self) -> Vec<CommandSpec> {
        lock(&self.commands.spawned).clone()
    }

    /// Every command run so far, in order.
    pub fn calls(&self) -> Vec<CommandSpec> {
        lock(&self.commands.calls).clone()
    }

    /// A running process, with its command name and tty.
    pub fn add_process(&self, pid: u32, comm: &str, tty: Option<&str>) {
        self.add_child(pid, comm, tty, 1);
    }

    /// A running process whose parent is `ppid`.
    pub fn add_child(&self, pid: u32, comm: &str, tty: Option<&str>, ppid: u32) {
        lock(&self.procs.table).insert(
            pid,
            Proc {
                comm: comm.to_string(),
                tty: tty.map(str::to_string),
                ppid,
            },
        );
    }

    /// `pid` shrugs off `sig` (by default a signal ends the process, and its
    /// children with it, as a shell passes a hangup on).
    pub fn ignore_signal(&self, pid: u32, sig: Signal) {
        lock(&self.procs.ignored).push((pid, sig));
    }

    /// Every signal sent so far, in order, whether or not it ended anything.
    pub fn signals(&self) -> Vec<(u32, Signal)> {
        lock(&self.procs.sent).clone()
    }
}

/// A successful command's output.
pub fn ok(stdout: &str) -> Output {
    Output {
        status: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

/// A failed command's output.
pub fn failed(code: i32) -> Output {
    Output {
        status: Some(code),
        ..Output::default()
    }
}

impl Sys for FakeSys {
    fn now(&self) -> SystemTime {
        *lock(&self.now)
    }

    fn new_session_id(&self) -> String {
        lock(&self.ids)
            .pop_front()
            .unwrap_or_else(|| "00000000-0000-4000-8000-000000000000".to_string())
    }

    fn pick(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        lock(&self.picks).pop_front().unwrap_or(0) % n
    }

    fn env(&self, key: &str) -> Option<String> {
        lock(&self.env).get(key).cloned()
    }

    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn cmd(&self) -> &dyn CommandRunner {
        &self.commands
    }

    fn procs(&self) -> &dyn ProcessTable {
        &self.procs
    }

    fn hostname(&self) -> String {
        self.hostname.clone()
    }

    fn exe(&self) -> PathBuf {
        PathBuf::from("/opt/yo/bin/yo")
    }

    /// Doesn't wait; records how long it was asked to.
    fn sleep(&self, d: Duration) {
        lock(&self.slept).push(d);
    }
}

struct Answer {
    program: String,
    args: Vec<String>,
    out: Output,
}

#[derive(Default)]
struct FakeCommands {
    answers: Mutex<Vec<Answer>>,
    calls: Mutex<Vec<CommandSpec>>,
    spawned: Mutex<Vec<CommandSpec>>,
}

impl CommandRunner for FakeCommands {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output> {
        lock(&self.calls).push(cmd.clone());
        let answers = lock(&self.answers);
        // The latest answer wins, so a test can override an earlier one.
        let hit = answers
            .iter()
            .rev()
            .find(|a| a.program == cmd.program && a.args == cmd.args);
        Ok(hit.map_or_else(
            || Output {
                status: Some(127),
                stdout: Vec::new(),
                stderr: format!("fake: no answer for {} {:?}", cmd.program, cmd.args).into_bytes(),
            },
            |a| a.out.clone(),
        ))
    }

    fn spawn(&self, cmd: &CommandSpec) -> io::Result<()> {
        lock(&self.spawned).push(cmd.clone());
        Ok(())
    }
}

struct Proc {
    comm: String,
    tty: Option<String>,
    ppid: u32,
}

#[derive(Default)]
struct FakeProcs {
    table: Mutex<BTreeMap<u32, Proc>>,
    ignored: Mutex<Vec<(u32, Signal)>>,
    sent: Mutex<Vec<(u32, Signal)>>,
}

impl ProcessTable for FakeProcs {
    fn comm(&self, pid: u32) -> Option<String> {
        lock(&self.table).get(&pid).map(|p| p.comm.clone())
    }

    fn tty(&self, pid: u32) -> Option<String> {
        lock(&self.table).get(&pid).and_then(|p| p.tty.clone())
    }

    fn ppid(&self, pid: u32) -> Option<u32> {
        lock(&self.table).get(&pid).map(|p| p.ppid)
    }

    fn signal(&self, pid: u32, sig: Signal) -> bool {
        lock(&self.sent).push((pid, sig));
        let mut table = lock(&self.table);
        if !table.contains_key(&pid) {
            return false;
        }
        if lock(&self.ignored).contains(&(pid, sig)) {
            return true;
        }
        let mut gone = vec![pid];
        while let Some(p) = gone.pop() {
            table.remove(&p);
            gone.extend(table.iter().filter(|(_, c)| c.ppid == p).map(|(k, _)| *k));
        }
        true
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A directory removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> io::Result<Self> {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yo-test-{}-{n}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        // Resolved, so paths compare equal to what canonicalize gives
        // (macOS's /var is a link to /private/var).
        Ok(TempDir(dir.canonicalize()?))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fake Codex: what the real one leaves under `~/.codex` (checked
/// against Codex 0.172.0), for the Codex harness's tests.
pub struct FakeCodex<'a> {
    pub sys: &'a FakeSys,
}

impl FakeCodex<'_> {
    /// Session `id`'s rollout, as the TUI (`source: "cli"`, or what's
    /// given) writes it: filed under `day` (`YYYY/MM/DD`), starting `at` (a
    /// UTC timestamp like `2026-11-27T03:53:30.908Z`) in `cwd`.
    pub fn rollout(&self, day: &str, id: &str, cwd: &Path, at: &str, source: &str) -> PathBuf {
        let dir = self.sys.home().join(".codex/sessions").join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(format!("rollout-{}-{id}.jsonl", at.replace(':', "-")));
        let meta = serde_json::json!({
            "timestamp": at, "ordinal": 0, "type": "session_meta",
            "payload": {
                "session_id": id, "id": id, "timestamp": at,
                "cwd": cwd.to_string_lossy(), "originator": "codex-tui",
                "cli_version": "0.172.0", "source": source,
                "base_instructions": {"text": "You are Codex..."},
            },
        });
        let event =
            r#"{"timestamp":"x","ordinal":1,"type":"event_msg","payload":{"type":"task_started"}}"#;
        std::fs::write(&f, format!("{meta}\n{event}\n")).unwrap();
        f
    }

    /// Sessions `(id, pid, cwd)` running: their thread locks, their
    /// processes, and what `lsof` says about them. Give every running
    /// session in one call, after any [`FakeCodex::stale_lock`]: `lsof` is
    /// asked about every lock at once.
    pub fn running(&self, live: &[(&str, u32, &Path)]) {
        let locks = self.sys.home().join(".codex/thread-writer-locks");
        std::fs::create_dir_all(&locks).unwrap();
        std::fs::write(locks.join(".coordination.lock"), "").unwrap();
        let mut held = String::new();
        let mut cwds = String::new();
        let mut ids: Vec<&(&str, u32, &Path)> = live.iter().collect();
        ids.sort();
        for (id, pid, cwd) in &ids {
            let f = locks.join(format!("{id}.lock"));
            std::fs::write(&f, "").unwrap();
            self.sys.add_process(*pid, "codex", Some("ttys020"));
            held.push_str(&format!("p{pid}\nf46\nn{}\n", f.display()));
            cwds.push_str(&format!("p{pid}\nfcwd\nn{}\n", cwd.display()));
        }
        // Every lock, stale ones too, in the harness's order: by id.
        let mut files: Vec<String> = std::fs::read_dir(&locks)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".lock") && !n.starts_with('.'))
            .collect();
        files.sort();
        let files: Vec<String> = files
            .iter()
            .map(|n| locks.join(n).to_string_lossy().into_owned())
            .collect();
        let mut args = vec!["-F", "pn", "--"];
        args.extend(files.iter().map(String::as_str));
        self.sys.answer_ok("lsof", &args, &held);
        let mut pids: Vec<u32> = ids.iter().map(|(_, p, _)| *p).collect();
        pids.sort();
        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
        let pids = pids.join(",");
        self.sys
            .answer_ok("lsof", &["-a", "-d", "cwd", "-F", "pn", "-p", &pids], &cwds);
    }

    /// A lock a crashed session left behind: no process holds it.
    pub fn stale_lock(&self, id: &str) {
        let locks = self.sys.home().join(".codex/thread-writer-locks");
        std::fs::create_dir_all(&locks).unwrap();
        std::fs::write(locks.join(format!("{id}.lock")), "").unwrap();
    }
}
