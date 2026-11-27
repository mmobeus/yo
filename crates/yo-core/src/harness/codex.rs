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
//! - **Not used, or not offered:** a settings layer, shared memory,
//!   personas, Remote Control, and ledger hooks. Codex does have hooks
//!   (`SessionStart`, `SessionEnd`, ...), but each needs approving in its
//!   `/hooks` screen before it runs, so yo learns ids from the rollouts
//!   instead. It has no `--name`: names come from `/rename` only.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Argv, Capabilities, Harness, IdPolicy, Launch, LiveSession, SessionSpec, valid_session_id,
};
use crate::sys::{CommandSpec, Sys};

/// Its name, in config and the ledger.
pub const NAME: &str = "codex";

#[derive(Debug, Default)]
pub struct Codex;

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

    /// Only `--model`.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            models: true,
            ..Capabilities::default()
        }
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
