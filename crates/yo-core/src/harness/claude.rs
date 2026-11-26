//! The Claude Code harness.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::{Argv, Capabilities, Harness, LiveSession, SessionSpec, valid_session_id};
use crate::ledger::HookEvent;
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

    /// `<command> [--model M] [--agent A] [--append-system-prompt P]
    /// --name NAME --session-id ID [MESSAGE]`. The codename goes at the end
    /// of the first message, or into the system prompt when the message is a
    /// slash command (whose arguments it would otherwise join).
    fn tui_command(&self, s: &SessionSpec) -> Argv {
        let (message, sysprompt) = first_message(s.first_message.as_deref(), &s.codename);
        let mut args = Vec::new();
        let mut flag = |f: &str, v: &Option<String>| {
            if let Some(v) = v {
                args.push(f.to_string());
                args.push(v.clone());
            }
        };
        flag("--model", &s.model);
        flag("--agent", &s.persona);
        flag("--append-system-prompt", &sysprompt);
        flag("--name", &Some(s.name.clone()));
        flag("--session-id", &s.id);
        args.extend(message);
        Argv {
            env: Vec::new(),
            command: s.command.clone(),
            args,
        }
    }

    /// `SessionStart` and `SessionEnd` payloads (a missing event name is a
    /// start, as bash takes it). Only `/exit` and ctrl-d
    /// (`prompt_input_exit`) end a session deliberately; a closed tab
    /// reports `other`, and a crash reports nothing.
    fn hook_event(&self, payload: &str) -> Option<HookEvent> {
        let d: Value = serde_json::from_str(payload).ok()?;
        let s = |k: &str| d.get(k).and_then(Value::as_str);
        let id = s("session_id").filter(|s| valid_session_id(s))?.to_string();
        let cwd = PathBuf::from(s("cwd").unwrap_or(""));
        match s("hook_event_name").unwrap_or("SessionStart") {
            "SessionEnd" => Some(HookEvent::End {
                id,
                cwd,
                deliberate: s("reason") == Some("prompt_input_exit"),
            }),
            "SessionStart" => Some(HookEvent::Start { id, cwd }),
            _ => None,
        }
    }
}

/// One session file, if it's an interactive session with an id and a pid.
/// The first message and the system-prompt addition that tell a session its
/// codename.
fn first_message(brief: Option<&str>, codename: &str) -> (Option<String>, Option<String>) {
    let use_it = "use it whenever you identify this session, e.g. in notes, PR comments you're asked to post, or messages.";
    match brief.filter(|b| !b.is_empty()) {
        Some(b) if b.starts_with('/') => (
            Some(b.to_string()),
            Some(format!(
                "Your codename for this session is \"{codename}\". Introduce yourself with it, and {use_it}"
            )),
        ),
        Some(b) => (
            Some(format!(
                "{b} (FYI, your codename for this session is \"{codename}\" — introduce yourself with it and {use_it})"
            )),
            None,
        ),
        None => (
            Some(format!(
                "Your codename for this session is \"{codename}\". Briefly introduce yourself by codename, then wait for instructions."
            )),
            None,
        ),
    }
}

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
    let id = d["sessionId"].as_str().filter(|s| valid_session_id(s))?;
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
        // an id that isn't one
        write(
            t.path(),
            18,
            r#"{"pid":18,"sessionId":"x y'z","cwd":"/w/h"}"#,
        );
        // a pid written as a string
        write(
            t.path(),
            17,
            r#"{"pid":"17","sessionId":"s17","cwd":"/w/g"}"#,
        );
        fs::write(t.path().join(".claude/sessions/16.json"), "not json").unwrap();
        for pid in [10, 11, 12, 15, 17, 18] {
            sys.add_process(pid, "claude", None);
        }
        sys.add_process(14, "vim", None);
        let live = Claude.live(&sys);
        let ids: Vec<&str> = live.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s10", "s15", "s17"]);
        assert_eq!(live[0].name.as_deref(), Some("a-1"));
        assert_eq!(live[0].cwd, PathBuf::from("/w/a"));
        assert_eq!(live[0].updated_at, 5);
    }

    #[test]
    fn hook_payloads() {
        let ev = |p: &str| Claude.hook_event(p);
        assert_eq!(
            ev(r#"{"hook_event_name":"SessionStart","session_id":"s","cwd":"/w"}"#),
            Some(HookEvent::Start {
                id: "s".into(),
                cwd: "/w".into()
            })
        );
        assert_eq!(
            ev(r#"{"session_id":"s","cwd":"/w"}"#),
            Some(HookEvent::Start {
                id: "s".into(),
                cwd: "/w".into()
            })
        );
        for (reason, deliberate) in [("prompt_input_exit", true), ("other", false), ("", false)] {
            let p = format!(
                r#"{{"hook_event_name":"SessionEnd","session_id":"s","cwd":"/w","reason":"{reason}"}}"#
            );
            assert_eq!(
                ev(&p),
                Some(HookEvent::End {
                    id: "s".into(),
                    cwd: "/w".into(),
                    deliberate
                })
            );
        }
    }

    #[test]
    fn bad_hook_payloads_are_ignored() {
        for p in [
            "",
            "null",
            "[]",
            "{",
            "\u{0}",
            r#"{"session_id": 5}"#,
            r#"{"session_id": ""}"#,
            r#"{"hook_event_name":"Stop","session_id":"s"}"#,
            r#"{"session_id":"s; rm -rf ~"}"#,
            r#"{"session_id":"$(touch x)"}"#,
        ] {
            assert_eq!(Claude.hook_event(p), None, "{p}");
        }
        // A cwd that isn't a string is no cwd.
        assert_eq!(
            Claude.hook_event(r#"{"session_id":"s","cwd":null}"#),
            Some(HookEvent::Start {
                id: "s".into(),
                cwd: "".into()
            })
        );
    }

    fn spec(msg: Option<&str>) -> SessionSpec {
        SessionSpec {
            command: "claude --permission-mode auto".into(),
            dir: "/w".into(),
            name: "issue_7".into(),
            id: Some("abc".into()),
            model: None,
            persona: None,
            codename: "Plucky Gary".into(),
            first_message: msg.map(str::to_string),
        }
    }

    #[test]
    fn tui_commands() {
        let a = Claude.tui_command(&spec(Some("fix it")));
        assert_eq!(a.command, "claude --permission-mode auto");
        assert_eq!(a.args[..4], ["--name", "issue_7", "--session-id", "abc"]);
        assert!(
            a.args[4].starts_with("fix it (FYI, your codename for this session is \"Plucky Gary\"")
        );

        let mut s = spec(Some("/code-review https://x"));
        s.model = Some("opus".into());
        s.persona = Some("cindy".into());
        let a = Claude.tui_command(&s);
        assert_eq!(a.args[..4], ["--model", "opus", "--agent", "cindy"]);
        assert_eq!(a.args[4], "--append-system-prompt");
        assert!(a.args[5].starts_with("Your codename for this session is \"Plucky Gary\"."));
        assert_eq!(a.args.last().unwrap(), "/code-review https://x");

        let a = Claude.tui_command(&spec(None));
        assert!(
            a.args
                .last()
                .unwrap()
                .ends_with("then wait for instructions.")
        );
    }

    #[test]
    fn no_sessions_dir() {
        let t = TempDir::new().unwrap();
        assert!(Claude.live(&FakeSys::new(t.path())).is_empty());
    }
}
