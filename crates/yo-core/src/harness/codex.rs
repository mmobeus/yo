//! The Codex CLI harness (`codex`, checked against 0.172.0).
//!
//! What Codex offers, and what yo uses of it:
//!
//! - **Ids:** Codex picks a session's id (a UUID) and has no flag to take
//!   one, so a record is written without one and learns it later
//!   ([`Harness::reported_id`]), from Codex's own records.
//! - **Its records:** each session's rollout,
//!   `~/.codex/sessions/YYYY/MM/DD/rollout-<local time>-<id>.jsonl`, whose
//!   first line is a `session_meta` with its `id`, `cwd`, UTC `timestamp`
//!   and `source` (`cli` for the TUI). The TUI writes it with the first
//!   message, so yo always gives one.
//! - **Live sessions:** a running TUI holds a lock on
//!   `~/.codex/thread-writer-locks/<id>.lock`, removed when it exits (and
//!   left, unlocked, by a crash). `lsof` names the process holding it, and
//!   that process's directory. Codex has no status command or pid files.
//! - **Inside a session:** Codex sets `CODEX_THREAD_ID` for the commands it
//!   runs.
//! - **The codename** goes in as developer instructions
//!   (`-c developer_instructions=...`), so the first message stays as it is.
//! - **The status line:** yo passes `-c tui.status_line=[...]`
//!   ([`STATUS_LINE`]), unless `config.toml` or the configured command
//!   sets one (a `-c` would beat the person's own).
//! - **Trust:** Codex asks to trust a git repo once, and keeps the answer
//!   in `~/.codex/config.toml` under the main clone (`[projects."<clone>"]`),
//!   which every worktree of it inherits. A `-c projects...` override
//!   doesn't count. For a worktree of a repo yo trusts, yo asks the person
//!   at launch and adds the clone's entry itself; otherwise it says Codex
//!   will ask, so a tab waiting on the prompt isn't a surprise.
//! - **The ledger hook:** Codex has hooks, but only from its own files
//!   (`~/.codex/hooks.json`), and each must be approved in its `/hooks`
//!   screen before it runs. `yo setup` adds one, `SessionEnd` running
//!   `yo _session-hook codex`, when asked. Codex fires it on `/quit` and
//!   ctrl-d, not on a closed tab or a crash, so any `SessionEnd` is a
//!   deliberate end. It fires only in a TUI that runs its own threads: a
//!   `-c` override (yo always passes one) keeps the TUI off Codex's shared
//!   daemon, where `/quit` only disconnects.
//! - **Skills:** Codex finds a person's own in `$CODEX_HOME/skills/<name>`
//!   (`SKILL.md`, the same frontmatter as Claude's), run as `$name`.
//!   `yo setup` installs yo's there, rewritten for Codex: its commands run
//!   in a sandbox that blocks iTerm2 and yo's state, so they ask to run
//!   outside it.
//! - **Not used, or not offered:** a settings layer, shared memory,
//!   personas and Remote Control. It has no `--name`: names come from
//!   `/rename` only.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Argv, Bundle, Bundled, BundledKind, Capabilities, GlobalHook, Harness, IdPolicy, Launch,
    LiveSession, PrepareAsk, Prepared, SessionSpec, valid_session_id,
};
use crate::ledger::HookEvent;
use crate::ojson::OValue;
use crate::paths::Paths;
use crate::plan::{Action, Note};
use crate::registry::Mode;
use crate::sys::{CommandSpec, Sys};
use crate::{Ctx, Error, Result};

mod config;
mod hooks;

/// Its name, in config and the ledger.
pub const NAME: &str = "codex";

/// The status line yo gives a session, as TOML: Claude's (`claude/statusline.sh`)
/// as near as Codex's items go. Codex can't show yo's name: it names a
/// thread only by `/rename`, and the tab's title has it anyway.
const STATUS_LINE: &str =
    r#"["model-with-reasoning","current-dir","git-branch","context-used","five-hour-limit"]"#;

#[derive(Debug, Default)]
pub struct Codex;

// What `yo setup` installs: the repo's `codex/` directory, built in. These
// are yo's Claude skills, written for Codex (`$skill`, its sandbox).
const SKILLS: &[(&str, &str)] = &[
    (
        "codex/skills/yo-adopt",
        include_str!("../../../../codex/skills/yo-adopt/SKILL.md"),
    ),
    (
        "codex/skills/yo-down",
        include_str!("../../../../codex/skills/yo-down/SKILL.md"),
    ),
    (
        "codex/skills/yo-title",
        include_str!("../../../../codex/skills/yo-title/SKILL.md"),
    ),
];

impl Harness for Codex {
    fn name(&self) -> &str {
        NAME
    }

    fn default_command(&self) -> &str {
        "codex"
    }

    fn ids(&self) -> IdPolicy {
        IdPolicy::Reported
    }

    /// Every thread lock a `codex` process holds, with that process's
    /// directory (else its rollout's `cwd`).
    fn live(&self, sys: &dyn Sys) -> Vec<LiveSession> {
        let dir = codex_home(sys).join("thread-writer-locks");
        let Ok(rd) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut locks: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let id = p.file_name()?.to_str()?.strip_suffix(".lock")?.to_string();
                valid_session_id(&id).then_some((id, p))
            })
            .collect();
        if locks.is_empty() {
            return Vec::new();
        }
        locks.sort();
        let held = lsof(
            sys,
            CommandSpec::new("lsof")
                .args(["-F", "pn", "--"])
                .args(locks.iter().map(|(_, p)| p.to_string_lossy().into_owned())),
        );
        let mut out: Vec<LiveSession> = Vec::new();
        for (id, path) in &locks {
            let Some((pid, _)) = held.iter().find(|(_, n)| Path::new(n) == path) else {
                continue;
            };
            if !sys.procs().comm(*pid).is_some_and(|c| c.contains("codex")) {
                continue;
            }
            out.push(LiveSession {
                harness: NAME.to_string(),
                id: id.clone(),
                pid: *pid,
                cwd: PathBuf::new(),
                name: None,
                updated_at: 0,
                // Codex doesn't say.
                idle: None,
            });
        }
        if out.is_empty() {
            return out;
        }
        let mut pids: Vec<u32> = out.iter().map(|l| l.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
        let cwds = lsof(
            sys,
            CommandSpec::new("lsof").args(["-a", "-d", "cwd", "-F", "pn", "-p", &pids.join(",")]),
        );
        for l in &mut out {
            l.cwd = cwds
                .iter()
                .find(|(p, _)| *p == l.pid)
                .map(|(_, c)| PathBuf::from(c))
                .or_else(|| rollout_meta(sys, &l.id).map(|m| m.cwd))
                .unwrap_or_default();
        }
        out.sort_by_key(|s| s.pid);
        out
    }

    /// `--model`, and the ledger hook (once `yo setup` has added it).
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            models: true,
            ledger_hooks: true,
            ..Capabilities::default()
        }
    }

    /// For a session in a git repo Codex hasn't trusted: for a worktree of
    /// a repo yo trusts, a question whether to trust its main clone in
    /// Codex's config; otherwise, a warning that Codex will ask in the
    /// tab. And a warning when yo's hook is in Codex's hooks file but not
    /// approved yet, so Codex will stop at its review first. And yo's status
    /// line, unless Codex's config or the configured command sets one.
    fn prepare(&self, ctx: &Ctx, p: &PrepareAsk<'_>) -> Result<Prepared> {
        let sys = &*ctx.sys;
        let home = codex_home(sys);
        let text = fs::read_to_string(home.join("config.toml")).unwrap_or_default();
        let cfg = config::read(&text);
        let mut out = Prepared::default();

        let root = match (p.mode, p.trusted_clone, p.repo) {
            (_, Some(c), _) => Some(c.to_path_buf()),
            (Mode::Worktree, None, Some(r)) => Some(r.path.clone()),
            _ => crate::git::primary_clone(sys, p.dir),
        };
        if let Some(root) = root {
            let real = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
            if cfg.trust(&root).is_none() && cfg.trust(&real).is_none() {
                let otherwise = format!(
                    "Codex will ask to trust {} in the new tab: accept it there once, and its worktrees inherit that",
                    root.display()
                );
                if p.trusted_clone.is_some() {
                    out.actions.push(Action::Ask {
                        question: format!(
                            "Codex hasn't trusted {} yet, so the new tab would stop at its trust prompt. \
                             Trust it for Codex now (one entry in {}; its worktrees inherit it)?",
                            root.display(),
                            home.join("config.toml").display()
                        ),
                        prep: super::Prep::Codex(Prep::Trust { root: real }),
                        otherwise,
                    });
                } else {
                    out.notes.push(Note::Warn(otherwise));
                }
            }
        }

        // A status line like Claude's, unless the person has their own.
        if !cfg.sets_status_line() && !p.command.contains("tui.status_line") {
            out.args = vec!["-c".into(), format!("tui.status_line={STATUS_LINE}")];
        }

        if let Some(h) = self.global_hook(sys)
            && h.installed.is_some()
            && !h.approved
        {
            out.notes.push(Note::Warn(
                "yo's Codex hook isn't approved yet, so the new tab stops at Codex's hook review \
                 first: approve it there once (or run yo setup)"
                    .into(),
            ));
        }
        Ok(out)
    }

    /// A `SessionEnd`, which Codex sends only for `/quit` and ctrl-d. yo
    /// adds no `SessionStart` hook: Codex's hooks run for every Codex
    /// session, and the id is learned from the rollouts anyway.
    fn hook_event(&self, payload: &str) -> Option<HookEvent> {
        let d: Value = serde_json::from_str(payload).ok()?;
        let s = |k: &str| d.get(k).and_then(Value::as_str);
        let id = s("session_id").filter(|s| valid_session_id(s))?.to_string();
        let cwd = PathBuf::from(s("cwd").unwrap_or(""));
        (s("hook_event_name")? == "SessionEnd").then_some(HookEvent::End {
            id,
            cwd,
            deliberate: true,
        })
    }

    /// yo's skills, in `$CODEX_HOME/skills`, where Codex finds a person's
    /// own. Only when Codex has a home: setup doesn't make one for a Codex
    /// that isn't there (and, once there is, says setup is stale).
    fn bundle(&self, sys: &dyn Sys, _paths: &Paths) -> Bundle {
        let home = codex_home(sys);
        if !home.is_dir() {
            return Bundle::default();
        }
        let items = SKILLS
            .iter()
            .map(|&(rel, text)| Bundled {
                rel,
                dest: home
                    .join("skills")
                    .join(rel.rsplit('/').next().unwrap_or(rel)),
                kind: BundledKind::Dir(vec![("SKILL.md", text)]),
            })
            .collect();
        Bundle {
            items,
            global: None,
        }
    }

    /// `~/.codex/hooks.json`, and whether Codex has approved yo's hook in
    /// it: an approval is kept in its `config.toml`, under the hook's
    /// place in the file.
    fn global_hook(&self, sys: &dyn Sys) -> Option<GlobalHook> {
        let home = codex_home(sys);
        let path = home.join("hooks.json");
        let v = match fs::read_to_string(&path) {
            Ok(t) => OValue::parse(&t)
                .ok()
                .filter(|v| matches!(v, OValue::Object(_))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(OValue::object()),
            // There but unreadable (not UTF-8, say): not a file to replace.
            Err(_) => None,
        };
        let found = v.as_ref().and_then(hooks::find);
        let approved = found.as_ref().is_some_and(|(g, i, _)| {
            let text = fs::read_to_string(home.join("config.toml")).unwrap_or_default();
            let cfg = config::read(&text);
            let real = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            [&path, &real]
                .iter()
                .any(|p| cfg.hook_approved(&format!("{}:session_end:{g}:{i}", p.display())))
        });
        let command = hooks::command(&sys.exe());
        Some(GlobalHook {
            with: v
                .as_ref()
                .and_then(|v| hooks::with_hook(v, &command))
                .map(|v| v.to_pretty()),
            without: v
                .as_ref()
                .and_then(hooks::without_hook)
                .map(|v| v.to_pretty()),
            path,
            command,
            installed: found.map(|(_, _, c)| c),
            approved,
            offer: "yo can add a hook to Codex so that a Codex session quit with /quit shows \
                    as ended. It runs for every Codex session (it only touches yo's ledger), \
                    and Codex runs it once you've approved it in its hook review.",
            approve: "Codex opens on \"Hooks need review\": choose Trust all and continue (or \
                      Review hooks, to trust only yo's), then /quit",
        })
    }

    /// `<command> -c developer_instructions=<codename> [-m M] MESSAGE`. With
    /// no brief, the message asks it to introduce itself: Codex records a
    /// session only once it has a first message.
    fn tui_command(&self, s: &SessionSpec) -> Argv {
        let mut args = flags(s);
        args.push(
            s.first_message
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| {
                    "Briefly introduce yourself by codename, then wait for instructions.".into()
                }),
        );
        Argv {
            env: Vec::new(),
            command: s.command.clone(),
            args,
        }
    }

    /// `<command> resume -c developer_instructions=<codename> [-m M] ID
    /// [MESSAGE]`.
    fn resume_command(&self, id: &str, s: &SessionSpec) -> Argv {
        let mut args = vec!["resume".to_string()];
        args.extend(flags(s));
        args.push(id.to_string());
        args.extend(s.first_message.clone().filter(|m| !m.is_empty()));
        Argv {
            env: Vec::new(),
            command: s.command.clone(),
            args,
        }
    }

    /// Its rollout's `cwd`.
    fn session_cwd(&self, sys: &dyn Sys, id: &str) -> Option<PathBuf> {
        rollout_meta(sys, id).map(|m| m.cwd)
    }

    /// Codex sets `CODEX_THREAD_ID` for the commands it runs.
    fn own_session(&self, sys: &dyn Sys) -> Option<String> {
        sys.env_nonempty("CODEX_THREAD_ID")
            .filter(|id| valid_session_id(id))
    }

    /// The first TUI session Codex recorded in the launch's directory, in
    /// the day from its launch (less a minute, for clocks), whose developer
    /// instructions carry the codename yo gave it, and that isn't `taken`.
    /// The codename is what ties a rollout to this launch: another session
    /// started there later, by yo or not, has a different one.
    fn reported_id(&self, sys: &dyn Sys, l: &Launch<'_>, taken: &[String]) -> Option<String> {
        if l.codename.is_empty() {
            return None;
        }
        let free = |id: &str| !taken.iter().any(|t| t == id);
        let (from, to) = (l.since - 60, l.since + 86_400);
        let told = told(l.codename);
        let mut found: Vec<Meta> = rollouts_since(sys, from)
            .into_iter()
            .filter_map(|p| read_meta(&p).map(|m| (p, m)))
            .filter(|(_, m)| {
                m.interactive
                    && (from..=to).contains(&m.at)
                    && free(&m.id)
                    && same_dir(&m.cwd, l.dir)
            })
            .filter(|(p, _)| mentions(p, &told))
            .map(|(_, m)| m)
            .collect();
        found.sort_by_key(|m| m.at);
        found.into_iter().next().map(|m| m.id)
    }
}

/// A step of Codex's setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prep {
    /// Adds `[projects."<root>"]` with `trust_level = "trusted"` to the end
    /// of Codex's `config.toml`, leaving the rest as it is.
    Trust { root: PathBuf },
}

impl Prep {
    pub fn run(&self, sys: &dyn Sys) -> Result<()> {
        match self {
            Prep::Trust { root } => {
                let path = codex_home(sys).join("config.toml");
                let text = match fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(e) => return Err(Error::io(&path, e)),
                };
                let next = config::with_trust(&text, root).ok_or_else(|| {
                    Error::Failed(format!(
                        "{} has an entry for it already, or a shape yo doesn't add to",
                        path.display()
                    ))
                })?;
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
                }
                crate::write_atomic(&path, next.as_bytes())
            }
        }
    }

    pub fn planned(&self) -> &'static str {
        match self {
            Prep::Trust { .. } => "trust the main clone in Codex's config.toml",
        }
    }

    pub fn doing(&self) -> &'static str {
        match self {
            Prep::Trust { .. } => "trusting the main clone in Codex's config.toml",
        }
    }

    pub fn failed(&self) -> &'static str {
        match self {
            Prep::Trust { .. } => {
                "couldn't trust the main clone in Codex's config.toml (Codex will ask in the tab)"
            }
        }
    }
}

/// What a session is told of its codename: the start of its developer
/// instructions.
fn told(codename: &str) -> String {
    format!("Your codename for this session is \"{codename}\".")
}

/// Whether the rollout at `path` says `text` in its opening lines, where
/// Codex writes the developer instructions (JSON-escaped, as there).
fn mentions(path: &Path, text: &str) -> bool {
    let Ok(f) = fs::File::open(path) else {
        return false;
    };
    let quoted = serde_json::to_string(text).unwrap_or_default();
    let needle = &quoted[1..quoted.len().saturating_sub(1).max(1)];
    BufReader::new(f)
        .lines()
        .take(32)
        .map_while(|l| l.ok())
        .any(|l| l.contains(needle))
}

/// The flags new and resumed sessions share.
fn flags(s: &SessionSpec) -> Vec<String> {
    let mut args = Vec::new();
    if !s.codename.is_empty() {
        args.push("-c".into());
        args.push(format!(
            "developer_instructions={}",
            toml_string(&format!(
                "{} Introduce yourself with it, and use it whenever you identify this session, e.g. in notes, PR comments you're asked to post, or messages.",
                told(&s.codename)
            ))
        ));
    }
    args.extend(s.args.iter().cloned());
    if let Some(m) = &s.model {
        args.push("-m".into());
        args.push(m.clone());
    }
    args
}

/// `s` as a TOML basic string, which `-c` parses as TOML.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `CODEX_HOME`, else `~/.codex`.
fn codex_home(sys: &dyn Sys) -> PathBuf {
    sys.env_nonempty("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| sys.home().join(".codex"))
}

/// `lsof -F pn` output as (pid, name) pairs: a `p` line starts a process,
/// and each `n` line after it is one of its files.
fn lsof(sys: &dyn Sys, spec: CommandSpec) -> Vec<(u32, String)> {
    let Ok(out) = sys.cmd().run(&spec) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pid = None;
    let mut found = Vec::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(n), Some(p)) = (line.strip_prefix('n'), pid) {
            found.push((p, n.to_string()));
        }
    }
    found
}

/// A rollout's first line.
#[derive(Debug, PartialEq, Eq)]
struct Meta {
    id: String,
    cwd: PathBuf,
    /// Unix seconds, from its UTC timestamp.
    at: i64,
    /// From the TUI (`source: cli`), not `codex exec`.
    interactive: bool,
}

fn read_meta(path: &Path) -> Option<Meta> {
    let mut line = String::new();
    BufReader::new(fs::File::open(path).ok()?)
        .read_line(&mut line)
        .ok()?;
    let v: Value = serde_json::from_str(&line).ok()?;
    if v["type"] != "session_meta" {
        return None;
    }
    let p = &v["payload"];
    let id = p["id"].as_str().filter(|id| valid_session_id(id))?;
    Some(Meta {
        id: id.to_string(),
        cwd: PathBuf::from(p["cwd"].as_str()?),
        at: p["timestamp"].as_str().and_then(unix_secs)?,
        interactive: p["source"].as_str() == Some("cli"),
    })
}

/// The rollout of session `id`, wherever it's filed.
fn rollout_meta(sys: &dyn Sys, id: &str) -> Option<Meta> {
    if !valid_session_id(id) {
        return None;
    }
    let suffix = format!("-{id}.jsonl");
    day_dirs(sys, i64::MIN)
        .into_iter()
        .rev()
        .flat_map(files)
        .find(|p| p.to_string_lossy().ends_with(&suffix))
        .and_then(|p| read_meta(&p))
        .filter(|m| m.id == id)
}

/// Rollouts filed on a day from `since`'s (a day early, for time zones).
fn rollouts_since(sys: &dyn Sys, since: i64) -> Vec<PathBuf> {
    day_dirs(sys, since - 86_400)
        .into_iter()
        .flat_map(files)
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned());
            n.is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        })
        .collect()
}

/// `sessions/YYYY/MM/DD` directories for days from `since`'s UTC date on,
/// oldest first.
fn day_dirs(sys: &dyn Sys, since: i64) -> Vec<PathBuf> {
    let from = if since == i64::MIN {
        (0, 0, 0)
    } else {
        civil(since.div_euclid(86_400))
    };
    let root = codex_home(sys).join("sessions");
    let num = |p: &Path| -> Option<i64> { p.file_name()?.to_str()?.parse().ok() };
    let mut out = Vec::new();
    for y in dirs(&root) {
        let Some(yn) = num(&y) else { continue };
        for m in dirs(&y) {
            let Some(mn) = num(&m) else { continue };
            for d in dirs(&m) {
                let Some(dn) = num(&d) else { continue };
                if (yn, mn, dn) >= from {
                    out.push(((yn, mn, dn), d));
                }
            }
        }
    }
    out.sort();
    out.into_iter().map(|(_, d)| d).collect()
}

fn dirs(p: &Path) -> Vec<PathBuf> {
    fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn files(p: PathBuf) -> Vec<PathBuf> {
    fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether two paths name the same directory, as written or resolved
/// (`/tmp` is `/private/tmp`).
fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (fs::canonicalize(a), fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// `2026-11-27T03:53:30.908Z` as unix seconds. UTC only, as Codex writes it.
fn unix_secs(ts: &str) -> Option<i64> {
    let ts = ts.strip_suffix('Z')?;
    let (date, time) = ts.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|s| s.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.split('.').next()?;
    let mut t = time.splitn(3, ':').map(|s| s.parse::<i64>().ok());
    let (h, mi, s) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + s)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01.
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests;
