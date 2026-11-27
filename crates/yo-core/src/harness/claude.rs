//! The Claude Code harness.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Argv, Bundle, Bundled, BundledKind, Capabilities, GlobalSettings, Harness, LiveSession,
    PrepareAsk, Prepared, SessionSpec, single_quote, valid_session_id,
};
use crate::ledger::HookEvent;
use crate::ojson::OValue;
use crate::paths::Paths;
use crate::plan::Action;
use crate::sys::Sys;
use crate::{Ctx, Error, Result};

#[derive(Debug, Default)]
pub struct Claude;

// What `yo setup` installs: the repo's `claude/` directory, built in.
const SETTINGS: &str = include_str!("../../../../claude/settings.json");
const STATUSLINE: &str = include_str!("../../../../claude/statusline.sh");
const SKILLS: &[(&str, &str)] = &[
    (
        "claude/skills/yo-adopt",
        include_str!("../../../../claude/skills/yo-adopt/SKILL.md"),
    ),
    (
        "claude/skills/yo-down",
        include_str!("../../../../claude/skills/yo-down/SKILL.md"),
    ),
    (
        "claude/skills/yo-title",
        include_str!("../../../../claude/skills/yo-title/SKILL.md"),
    ),
];

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
        flag(
            "--settings",
            &s.settings
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        );
        flag("--model", &s.model);
        flag("--agent", &s.persona);
        flag("--append-system-prompt", &sysprompt);
        // A Remote Control name is the session's name too: otherwise Claude
        // derives one from the directory, and that's what ListAgents,
        // SendMessage and a worker's --from see.
        flag(
            "--name",
            &Some(s.remote_control.clone().unwrap_or_else(|| s.name.clone())),
        );
        flag("--remote-control", &s.remote_control);
        flag("--session-id", &s.id);
        args.extend(message);
        Argv {
            env: Vec::new(),
            command: s.command.clone(),
            args,
        }
    }

    /// Its session file (`~/.claude/sessions/<pid>.json`, live or not),
    /// else the first `cwd` in its transcript (`~/.claude/projects/*/<id>.jsonl`).
    fn session_cwd(&self, sys: &dyn Sys, id: &str) -> Option<PathBuf> {
        if !valid_session_id(id) {
            return None;
        }
        let home = sys.home();
        let files = |dir: PathBuf| -> Vec<PathBuf> {
            fs::read_dir(dir)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
                .unwrap_or_default()
        };
        for f in files(home.join(".claude/sessions")) {
            let Some(d) = fs::read_to_string(&f)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            else {
                continue;
            };
            if d.get("sessionId").and_then(Value::as_str) == Some(id)
                && let Some(c) = d
                    .get("cwd")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty())
            {
                return Some(PathBuf::from(c));
            }
        }
        for proj in files(home.join(".claude/projects")) {
            let t = proj.join(format!("{id}.jsonl"));
            let Ok(text) = fs::read_to_string(&t) else {
                continue;
            };
            for line in text.lines() {
                if let Some(c) = serde_json::from_str::<Value>(line)
                    .ok()
                    .and_then(|v| v.get("cwd").and_then(Value::as_str).map(str::to_string))
                    .filter(|c| !c.is_empty())
                {
                    return Some(PathBuf::from(c));
                }
            }
        }
        None
    }

    /// `<command> [--settings F] [--model M] [--agent A] --name NAME
    /// [--remote-control NAME] --resume ID [MESSAGE]`. The agent and Remote
    /// Control name come from the ledger: nothing says `claude --resume`
    /// restores either on its own.
    fn resume_command(&self, id: &str, s: &SessionSpec) -> Argv {
        let mut args = Vec::new();
        let mut flag = |f: &str, v: Option<String>| {
            if let Some(v) = v {
                args.push(f.to_string());
                args.push(v);
            }
        };
        flag(
            "--settings",
            s.settings
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        );
        flag("--model", s.model.clone());
        flag("--agent", s.persona.clone());
        let name = s.remote_control.clone().unwrap_or_else(|| s.name.clone());
        flag("--name", Some(name));
        flag("--remote-control", s.remote_control.clone());
        flag("--resume", Some(id.to_string()));
        args.extend(s.first_message.clone().filter(|m| !m.is_empty()));
        Argv {
            env: Vec::new(),
            command: s.command.clone(),
            args,
        }
    }

    /// The settings layer (yo's base layer, the profile's settings, the
    /// ledger hooks, the memory directory), the memory directory itself, and
    /// for a trusted worktree: trust in `~/.claude.json` and the clone's
    /// `.claude/settings.local.json` with the repo's MCP servers.
    fn prepare(&self, ctx: &Ctx, p: &PrepareAsk<'_>) -> Result<Prepared> {
        let reg = &ctx.registry;
        let mut out = Prepared::default();
        if let Some(clone) = p.trusted_clone {
            out.actions
                .push(Action::Prepare(super::Prep::Claude(Prep::Trust {
                    worktree: p.dir.to_path_buf(),
                    clone: clone.to_path_buf(),
                })));
            out.actions
                .push(Action::Prepare(super::Prep::Claude(Prep::LocalSettings {
                    worktree: p.dir.to_path_buf(),
                    clone: clone.to_path_buf(),
                    mcp: p.repo.map(|r| r.mcp.clone()).unwrap_or_default(),
                })));
        }

        let memory = reg.memory_dir(p.repo, p.picked);
        if let Some(m) = &memory {
            out.actions.push(Action::MakeDir {
                path: m.clone(),
                what: "memory".into(),
            });
        }
        let profile = reg.profile_of(p.repo, p.picked);
        let extra = profile.and_then(|pr| pr.settings.clone());
        if let (Some(f), Some(pr)) = (&extra, profile)
            && !f.is_file()
        {
            return Err(Error::Refused(format!(
                "profile '{}': no settings file at {}",
                pr.name,
                f.display()
            )));
        }
        let base = reg
            .defaults
            .settings
            .clone()
            .or_else(|| {
                ctx.paths
                    .config
                    .parent()
                    .map(|d| d.join("claude-settings.json"))
            })
            .filter(|f| f.is_file());
        let hook = format!(
            "{} _session-hook",
            single_quote(&ctx.sys.exe().to_string_lossy())
        );
        let contents = settings_layer(
            &[base.as_deref(), extra.as_deref()],
            &hook,
            memory.as_deref(),
        )?;
        let name = match profile {
            Some(pr) if pr.name != p.label => format!("{}.{}", pr.name, p.label),
            _ => p.label.to_string(),
        };
        let path = ctx.paths.settings().join(format!("{name}.json"));
        out.actions.push(Action::WriteFile {
            path: path.clone(),
            contents,
        });
        out.settings = Some(path);
        Ok(out)
    }

    /// The settings layer and the statusline it names, beside yo's config
    /// (nothing of Claude's own is replaced); the skills in
    /// `~/.claude/skills`, where Claude finds personal skills, each its own
    /// directory. `--merge-settings` merges the layer into
    /// `~/.claude/settings.json`.
    fn bundle(&self, sys: &dyn Sys, paths: &Paths) -> Bundle {
        let claude = sys.home().join(".claude");
        let mut items = vec![
            Bundled {
                rel: "claude/settings.json",
                dest: paths.config_dir.join("claude-settings.json"),
                kind: BundledKind::File {
                    text: SETTINGS,
                    exec: false,
                },
            },
            Bundled {
                rel: "claude/statusline.sh",
                dest: paths.config_dir.join("statusline.sh"),
                kind: BundledKind::File {
                    text: STATUSLINE,
                    exec: true,
                },
            },
        ];
        for (rel, text) in SKILLS {
            let name = rel.rsplit('/').next().unwrap_or(rel);
            items.push(Bundled {
                rel,
                dest: claude.join("skills").join(name),
                kind: BundledKind::Dir(vec![("SKILL.md", text)]),
            });
        }
        Bundle {
            items,
            global: Some(GlobalSettings {
                path: claude.join("settings.json"),
                layer: SETTINGS,
            }),
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
/// A step of Claude's setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prep {
    /// Marks a new worktree trusted in `~/.claude.json`, with the main
    /// clone's approval of external `CLAUDE.md` includes, so it adds no
    /// prompt of its own.
    Trust { worktree: PathBuf, clone: PathBuf },
    /// The clone's `.claude/settings.local.json` (gitignored, so not in a
    /// fresh checkout), where MCP approvals live, with the repo's `mcp`
    /// servers added to `enabledMcpjsonServers`.
    LocalSettings {
        worktree: PathBuf,
        clone: PathBuf,
        mcp: Vec<String>,
    },
}

impl Prep {
    pub fn run(&self, sys: &dyn Sys) -> Result<()> {
        match self {
            Prep::Trust { worktree, clone } => {
                trust(&sys.home().join(".claude.json"), worktree, clone)
            }
            Prep::LocalSettings {
                worktree,
                clone,
                mcp,
            } => local_settings(worktree, clone, mcp),
        }
    }

    pub fn failed(&self) -> &'static str {
        match self {
            Prep::Trust { .. } => {
                "trust-seed skipped (couldn't update ~/.claude.json; Claude will prompt)"
            }
            Prep::LocalSettings { .. } => {
                "couldn't write .claude/settings.local.json (Claude may prompt for MCP servers)"
            }
        }
    }
}

fn read_json(path: &Path) -> Result<Option<OValue>> {
    match fs::read_to_string(path) {
        Ok(t) => OValue::parse(&t)
            .map(Some)
            .map_err(|e| Error::Failed(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn s(v: &str) -> OValue {
    OValue::String(v.to_string())
}

fn trust(claude_json: &Path, worktree: &Path, clone: &Path) -> Result<()> {
    let bad = || {
        Error::Failed(format!(
            "{}: not the shape Claude writes",
            claude_json.display()
        ))
    };
    let mut d = read_json(claude_json)?.unwrap_or_else(OValue::object);
    let projects = d.entry_object("projects").ok_or_else(bad)?;
    let base = projects.get(&clone.to_string_lossy()).cloned();
    let ent = projects
        .entry_object(&worktree.to_string_lossy())
        .ok_or_else(bad)?;
    for k in [
        "hasClaudeMdExternalIncludesApproved",
        "hasClaudeMdExternalIncludesWarningShown",
    ] {
        if let Some(v) = base.as_ref().and_then(|b| b.get(k))
            && ent.get(k).is_none()
        {
            ent.insert(k, v.clone());
        }
    }
    ent.insert("hasTrustDialogAccepted", OValue::Bool(true));
    // json.dump wrote no final newline here.
    let text = d.to_pretty();
    crate::write_atomic(claude_json, text.trim_end().as_bytes())
}

fn local_settings(worktree: &Path, clone: &Path, mcp: &[String]) -> Result<()> {
    let rel = ".claude/settings.local.json";
    let (src, dst) = (clone.join(rel), worktree.join(rel));
    let dir = worktree.join(".claude");
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    if src.is_file() {
        fs::copy(&src, &dst).map_err(|e| Error::io(&dst, e))?;
    }
    // A file that isn't valid JSON is started over, as bash did.
    let mut d = read_json(&dst)
        .ok()
        .flatten()
        .filter(|v| matches!(v, OValue::Object(_)))
        .unwrap_or_else(OValue::object);
    let have: Vec<OValue> = d
        .get("enabledMcpjsonServers")
        .and_then(OValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut merged = have.clone();
    for m in mcp {
        if !merged.contains(&s(m)) {
            merged.push(s(m));
        }
    }
    if merged != have {
        d.insert("enabledMcpjsonServers", OValue::Array(merged));
        crate::write_atomic(&dst, d.to_pretty().as_bytes())?;
    }
    Ok(())
}

/// A session's settings layer: each of `layers` that's given, merged in
/// order (objects key by key, lists appended to without repeats, anything
/// else replaced), then the ledger hooks running `hook`, then the memory
/// directory.
pub fn settings_layer(
    layers: &[Option<&Path>],
    hook: &str,
    memory: Option<&Path>,
) -> Result<String> {
    let mut d = OValue::object();
    for path in layers.iter().flatten() {
        let v = read_json(path)?
            .ok_or_else(|| Error::Failed(format!("{}: missing", path.display())))?;
        merge(&mut d, v);
    }
    let h = OValue::Array(vec![OValue::Object(vec![(
        "hooks".into(),
        OValue::Array(vec![OValue::Object(vec![
            ("type".into(), s("command")),
            ("command".into(), s(hook)),
        ])]),
    )])]);
    merge(
        &mut d,
        OValue::Object(vec![(
            "hooks".into(),
            OValue::Object(vec![
                ("SessionStart".into(), h.clone()),
                ("SessionEnd".into(), h),
            ]),
        )]),
    );
    if let Some(m) = memory {
        d.insert("autoMemoryDirectory", s(&m.to_string_lossy()));
    }
    Ok(d.to_pretty())
}

fn merge(a: &mut OValue, b: OValue) {
    let OValue::Object(b) = b else { return };
    for (k, v) in b {
        match (a.get_mut(&k), v) {
            (Some(x @ OValue::Object(_)), v @ OValue::Object(_)) => merge(x, v),
            (Some(OValue::Array(x)), OValue::Array(v)) => {
                for item in v {
                    if !x.contains(&item) {
                        x.push(item);
                    }
                }
            }
            (_, v) => {
                a.insert(&k, v);
            }
        }
    }
}

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
    use serde_json::json;

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
            settings: None,
            remote_control: None,
        }
    }

    #[test]
    fn settings_layers_merge_in_order() {
        let t = TempDir::new().unwrap();
        let (a, b) = (t.path().join("a.json"), t.path().join("b.json"));
        fs::write(&a, r#"{"permissions": {"allow": ["x"], "deny": ["d"]}, "model": "m", "hooks": {"Stop": [1]}}"#).unwrap();
        fs::write(&b, r#"{"permissions": {"allow": ["x", "y"]}, "model": "n", "hooks": {"SessionStart": [0]}}"#).unwrap();
        let text = settings_layer(
            &[Some(&a), None, Some(&b)],
            "'/opt/yo' _session-hook",
            Some(Path::new("/m")),
        )
        .unwrap();
        assert!(text.ends_with("}\n"));
        let v: Value = serde_json::from_str(&text).unwrap();
        let hook = json!({"hooks": [{"type": "command", "command": "'/opt/yo' _session-hook"}]});
        assert_eq!(
            v,
            json!({
                "permissions": {"allow": ["x", "y"], "deny": ["d"]},
                "model": "n",
                "hooks": {"Stop": [1], "SessionStart": [0, hook.clone()], "SessionEnd": [hook]},
                "autoMemoryDirectory": "/m",
            })
        );
        // A layer that isn't JSON is an error, not silently dropped.
        fs::write(&a, "nope").unwrap();
        assert!(settings_layer(&[Some(&a)], "h", None).is_err());
    }

    #[test]
    fn trust_seeds_the_worktree_and_keeps_the_rest() {
        let t = TempDir::new().unwrap();
        let cj = t.path().join(".claude.json");
        // No file yet: one is made.
        trust(&cj, Path::new("/w"), Path::new("/c")).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&cj).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"projects": {"/w": {"hasTrustDialogAccepted": true}}})
        );

        fs::write(
            &cj,
            r#"{"keep": [1], "projects": {"/c": {"hasClaudeMdExternalIncludesApproved": true, "other": 1},
                                         "/w2": {"hasClaudeMdExternalIncludesApproved": false}}}"#,
        )
        .unwrap();
        trust(&cj, Path::new("/w"), Path::new("/c")).unwrap();
        trust(&cj, Path::new("/w2"), Path::new("/c")).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&cj).unwrap()).unwrap();
        assert_eq!(v["keep"], json!([1]));
        assert_eq!(
            v["projects"]["/w"],
            json!({"hasClaudeMdExternalIncludesApproved": true, "hasTrustDialogAccepted": true})
        );
        // What the worktree already had stays.
        assert_eq!(
            v["projects"]["/w2"]["hasClaudeMdExternalIncludesApproved"],
            json!(false)
        );
        // A broken file is left alone.
        fs::write(&cj, "{broken").unwrap();
        assert!(trust(&cj, Path::new("/w"), Path::new("/c")).is_err());
        assert_eq!(fs::read_to_string(&cj).unwrap(), "{broken");
    }

    #[test]
    fn local_settings_copy_and_approve_mcp() {
        let t = TempDir::new().unwrap();
        let (c, w) = (t.path().join("c"), t.path().join("w"));
        fs::create_dir_all(c.join(".claude")).unwrap();
        fs::create_dir_all(&w).unwrap();
        let read = || -> Value {
            serde_json::from_str(
                &fs::read_to_string(w.join(".claude/settings.local.json")).unwrap(),
            )
            .unwrap()
        };
        fs::write(
            c.join(".claude/settings.local.json"),
            r#"{"enabledMcpjsonServers": ["a"], "x": 1}"#,
        )
        .unwrap();
        local_settings(&w, &c, &["b".into(), "a".into()]).unwrap();
        assert_eq!(read(), json!({"enabledMcpjsonServers": ["a", "b"], "x": 1}));
        // Nothing in the clone, nothing to add: no file.
        let w2 = t.path().join("w2");
        local_settings(&w2, &t.path().join("none"), &[]).unwrap();
        assert!(!w2.join(".claude/settings.local.json").exists());
        assert!(w2.join(".claude").is_dir());
    }

    #[test]
    fn resume_commands() {
        let mut s = spec(Some("carry on"));
        s.persona = Some("cindy".into());
        let a = Claude.resume_command("abc", &s);
        assert_eq!(
            a.args,
            [
                "--agent", "cindy", "--name", "issue_7", "--resume", "abc", "carry on"
            ]
        );
        s.remote_control = Some("cindy-home".into());
        s.first_message = None;
        let a = Claude.resume_command("abc", &s);
        assert_eq!(
            a.args[2..],
            [
                "--name",
                "cindy-home",
                "--remote-control",
                "cindy-home",
                "--resume",
                "abc"
            ]
        );
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

        let mut s = spec(None);
        s.settings = Some("/s/a b.json".into());
        let a = Claude.tui_command(&s);
        assert_eq!(a.args[..2], ["--settings", "/s/a b.json"]);
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
