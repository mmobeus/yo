//! The Claude Code harness.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::{Capabilities, Harness, LiveSession};
use crate::sys::Sys;

#[derive(Debug, Default)]
pub struct Claude;

impl Harness for Claude {
    fn name(&self) -> &str {
        "claude"
    }

    /// Interactive sessions from `~/.claude/sessions/<pid>.json` whose
    /// process is alive and is still `claude` (a pid can be reused after a
    /// reboot). Background sessions (`kind` other than `interactive`) are
    /// left out, as bash leaves them.
    fn live(&self, sys: &dyn Sys) -> Vec<LiveSession> {
        let dir = sys.home().join(".claude/sessions");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(d) = fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            else {
                continue;
            };
            let Some(s) = session(&d) else { continue };
            if sys
                .procs()
                .comm(s.pid)
                .is_some_and(|c| c.contains("claude"))
            {
                out.push(s);
            }
        }
        out.sort_by_key(|s| s.pid);
        out
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            settings_layer: true,
            memory: true,
            personas: true,
            ledger_hooks: true,
            remote_control: true,
            models: true,
        }
    }
}

/// One session file, if it's an interactive session with an id and a pid.
fn session(d: &Value) -> Option<LiveSession> {
    let pid = match &d["pid"] {
        Value::Number(n) => u32::try_from(n.as_u64()?).ok()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    if d.get("kind")
        .and_then(Value::as_str)
        .unwrap_or("interactive")
        != "interactive"
    {
        return None;
    }
    let id = d["sessionId"].as_str().filter(|s| !s.is_empty())?;
    Some(LiveSession {
        harness: "claude".to_string(),
        id: id.to_string(),
        pid,
        cwd: PathBuf::from(d["cwd"].as_str().unwrap_or("")),
        name: d["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        updated_at: d["updatedAt"].as_i64().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, TempDir};

    fn write(home: &std::path::Path, pid: u32, json: &str) {
        let dir = home.join(".claude/sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{pid}.json")), json).unwrap();
    }

    #[test]
    fn live_sessions() {
        let t = TempDir::new().unwrap();
        let sys = FakeSys::new(t.path());
        // As Claude Code 2.1 writes them (trimmed).
        write(
            t.path(),
            10,
            r#"{"pid":10,"sessionId":"s10","cwd":"/w/a","kind":"interactive","name":"a-1","updatedAt":5}"#,
        );
        // background, not interactive
        write(
            t.path(),
            11,
            r#"{"pid":11,"sessionId":"s11","cwd":"/w/b","kind":"background"}"#,
        );
        // no id yet
        write(t.path(), 12, r#"{"pid":12,"cwd":"/w/c"}"#);
        // dead
        write(t.path(), 13, r#"{"pid":13,"sessionId":"s13","cwd":"/w/d"}"#);
        // pid reused by something else
        write(t.path(), 14, r#"{"pid":14,"sessionId":"s14","cwd":"/w/e"}"#);
        // no kind: interactive
        write(t.path(), 15, r#"{"pid":15,"sessionId":"s15","cwd":"/w/f"}"#);
        fs::write(t.path().join(".claude/sessions/16.json"), "not json").unwrap();
        for pid in [10, 11, 12, 15] {
            sys.add_process(pid, "claude", None);
        }
        sys.add_process(14, "vim", None);
        let live = Claude.live(&sys);
        let ids: Vec<&str> = live.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s10", "s15"]);
        assert_eq!(live[0].name.as_deref(), Some("a-1"));
        assert_eq!(live[0].cwd, PathBuf::from("/w/a"));
        assert_eq!(live[0].updated_at, 5);
    }

    #[test]
    fn no_sessions_dir() {
        let t = TempDir::new().unwrap();
        assert!(Claude.live(&FakeSys::new(t.path())).is_empty());
    }
}
