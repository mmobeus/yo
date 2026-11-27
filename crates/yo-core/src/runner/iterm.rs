//! The iTerm2 runner: a session in a tab (or a window), with a shell beside
//! it. bash's `launch_session`, `ensure_iterm_profile` and
//! `open_iterm_session`.
//!
//! Its AppleScript and JavaScript are files beside this one, embedded, and
//! run through [`Sys::cmd`] with every value as an argument.
//!
//! # Closing a tab
//!
//! Closing a tab normally ends what runs in it: iTerm keeps the closed
//! session for its undo window ("undo can revive a session that has been
//! closed for up to 5 seconds", a profile setting), then hangs up its
//! terminal, and the shell and claude exit. That holds for a tab closed by
//! script, by `⌘W` or by the window's close button.
//!
//! **Except a workgroup tab closed while it shows CLI.** Claude Code's
//! workgroup puts a session's chat and a shell (its CLI peer) in one tab,
//! one shown at a time. Closed while CLI shows, iTerm ends only the CLI
//! session; the chat session is never hung up, isn't buried (Session ›
//! Buried Sessions stays empty), and runs on with no tab at all: iTerm and
//! its session server (`iTermServer`) both keep its terminal open, so its
//! login shell and claude stay alive until something signals them. Running
//! `yo down` from the CLI pane is the common way in (found in #35, iTerm
//! 3.7.9).
//!
//! iTerm's AppleScript shows the same thing from the other side: it lists
//! only the member of a workgroup tab that's showing. So yo records each
//! session's tab (`tab.window.id` and `tab.id`, which every member of a tab
//! shares) when it opens it, and looks for that as well as the terminal;
//! and `down` ends its session itself ([`crate::end`]) rather than trust
//! the close to.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::{Focused, LaunchSpec, Launched, Runner, RunnerKind, Tabs, WindowChoice, valid_tab};
use crate::harness::shell_word;
use crate::ojson::OValue;
use crate::registry::{Defaults, Layout, WindowMode};
use crate::sys::{CommandSpec, Sys};
use crate::{Error, Result};

const PREFS: &str = include_str!("iterm/prefs.js");
const OPEN: &str = include_str!("iterm/open.applescript");
const RETITLE: &str = include_str!("iterm/retitle.applescript");
const FOCUS: &str = include_str!("iterm/focus.applescript");
const TITLE: &str = include_str!("iterm/title.applescript");
const OWN_TITLE: &str = include_str!("iterm/own_title.applescript");
const RESYNC: &str = include_str!("iterm/resync.applescript");
const CLOSE: &str = include_str!("iterm/close.applescript");
const TABS: &str = include_str!("iterm/tabs.applescript");
const OWN_TAB: &str = include_str!("iterm/own_tab.applescript");

/// `osascript -` with `script` on stdin and `args` after it.
fn script(script: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        stdin: Some(script.as_bytes().to_vec()),
        ..CommandSpec::new("osascript")
            .arg("-")
            .args(args.iter().copied())
    }
}

/// The session's unique id out of `ITERM_SESSION_ID` (`w0t1p0:<uuid>`).
pub fn own_session(iterm_session_id: &str) -> &str {
    iterm_session_id
        .split_once(':')
        .map_or(iterm_session_id, |(_, id)| id)
}

/// Titles, for `yo title` and `yo adopt`: these work on the tab yo runs
/// in, which iTerm names in `ITERM_SESSION_ID`.
impl Iterm {
    /// Retitles every pane of the tab holding session `id`. `Ok(false)`:
    /// no such tab.
    pub fn retitle_own(sys: &dyn Sys, id: &str, title: &str) -> Result<bool> {
        let out = sys
            .cmd()
            .run(&script(TITLE, &[own_session(id), title]))
            .map_err(|e| Error::Failed(format!("osascript: {e}")))?;
        Ok(out.success() && out.text() == "ok")
    }

    /// Puts `title` back as the session's name when Claude Code's workgroup
    /// renames it, for a while, detached.
    pub fn hold_title(sys: &dyn Sys, title: &str) {
        let _ = sys.cmd().spawn(&script(RETITLE, &[title]));
    }

    /// The title yo gave session `id`'s tab, if any.
    pub fn own_title(sys: &dyn Sys, id: &str) -> Option<String> {
        let out = sys.cmd().run(&script(OWN_TITLE, &[own_session(id)])).ok()?;
        out.success().then(|| out.text()).filter(|t| !t.is_empty())
    }

    /// The tab of session `id` (`ITERM_SESSION_ID`), if iTerm shows it.
    pub fn own_tab(sys: &dyn Sys, id: &str) -> Option<String> {
        let out = sys.cmd().run(&script(OWN_TAB, &[own_session(id)])).ok()?;
        out.success().then(|| out.text()).filter(|t| valid_tab(t))
    }

    /// Every session yo titled gets its title back as its name.
    pub fn resync_titles(sys: &dyn Sys) {
        let _ = sys.cmd().run(&script(RESYNC, &[]));
    }
}

/// The settings it runs with, from the registry's defaults.
#[derive(Clone, Debug)]
pub struct Iterm {
    /// The title-locking profile yo keeps (`YO_ITERM_PROFILE`).
    pub profile: String,
    pub layout: Layout,
    pub window_mode: WindowMode,
    pub width: String,
    pub height: String,
}

impl Iterm {
    pub fn new(d: &Defaults) -> Iterm {
        Iterm {
            profile: d.iterm_profile.clone(),
            layout: d.layout,
            window_mode: d.window_mode,
            width: d.win_width.clone(),
            height: d.win_height.clone(),
        }
    }
}

/// What iTerm's preferences say, for a launch.
#[derive(Debug, Default)]
struct Prefs {
    default: Option<Value>,
    peer: bool,
    /// The workgroups, as iTerm keeps them (JSON).
    workgroups: Option<String>,
    screen: Option<(u64, u64)>,
}

fn prefs(sys: &dyn Sys) -> Prefs {
    let spec = CommandSpec {
        stdin: Some(PREFS.as_bytes().to_vec()),
        ..CommandSpec::new("osascript").args(["-l", "JavaScript", "-"])
    };
    let Some(v) = sys
        .cmd()
        .run(&spec)
        .ok()
        .filter(|o| o.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
    else {
        return Prefs::default();
    };
    let screen = v
        .get("screen")
        .and_then(Value::as_array)
        .and_then(|a| Some((a.first()?.as_u64()?, a.get(1)?.as_u64()?)));
    Prefs {
        default: v.get("default").filter(|d| d.is_object()).cloned(),
        peer: v.get("peer").and_then(Value::as_bool).unwrap_or(false),
        workgroups: v
            .get("workgroups")
            .and_then(Value::as_str)
            .map(str::to_string),
        screen,
    }
}

/// Claude Code's workgroup in iTerm2, and whether it has a terminal peer:
/// a shell beside every chat, which lets yo open claude alone rather than
/// split a pane for the shell (`YO_LAYOUT=workgroup`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Peer {
    Present,
    /// No such workgroup (the integration isn't set up, or iTerm2 isn't
    /// there), so nothing to add a peer to.
    NoGroup,
    /// No peer yet: the workgroups as they are, and with one named `name`.
    Missing {
        old: String,
        new: String,
    },
}

const CLAUDE_GROUP: &str = "builtin.claudeCode";

pub fn peer(sys: &dyn Sys, name: &str) -> Peer {
    let Some(raw) = prefs(sys).workgroups else {
        return Peer::NoGroup;
    };
    let Ok(mut groups) = OValue::parse(&raw) else {
        return Peer::NoGroup;
    };
    let OValue::Array(list) = &mut groups else {
        return Peer::NoGroup;
    };
    let id = |g: &OValue| match g.get("uniqueIdentifier") {
        Some(OValue::String(s)) => s == CLAUDE_GROUP,
        _ => false,
    };
    let Some(group) = list.iter_mut().find(|g| id(g)) else {
        return Peer::NoGroup;
    };
    if group.get("sessions").is_none() {
        group.insert("sessions", OValue::Array(Vec::new()));
    }
    let Some(OValue::Array(sessions)) = group.get_mut("sessions") else {
        return Peer::NoGroup;
    };
    let is_peer = |s: &OValue| {
        matches!(
            s.get("kind").and_then(|k| k.get("kind")),
            Some(OValue::String(k)) if k == "peer"
        )
    };
    if sessions.iter().any(is_peer) {
        return Peer::Present;
    }
    let s = |v: &str| OValue::String(v.to_string());
    sessions.push(OValue::Object(vec![
        ("mode".into(), s("regular")),
        (
            "kind".into(),
            OValue::Object(vec![("kind".into(), s("peer"))]),
        ),
        ("command".into(), s("")),
        ("perFileCommand".into(), s("")),
        (
            "toolbarItems".into(),
            OValue::Array(vec![OValue::Object(vec![(
                "kind".into(),
                s("modeSwitcher"),
            )])]),
        ),
        ("displayName".into(), s(name)),
        (
            "uniqueIdentifier".into(),
            s(&sys.new_session_id().to_uppercase()),
        ),
        ("urlString".into(), s("")),
        ("parentID".into(), s("builtin.claudeCode.main")),
    ]));
    Peer::Missing {
        old: raw,
        new: groups.to_compact(),
    }
}

/// Writes iTerm2's workgroups. iTerm reads them at launch, so a running
/// iTerm2 sees them after its next restart (and writes its own copy back
/// if they're edited in its settings before then).
pub fn write_workgroups(sys: &dyn Sys, json: &str) -> Result<()> {
    let hex: String = json.bytes().map(|b| format!("{b:02x}")).collect();
    let spec = CommandSpec::new("defaults").args([
        "write",
        "com.googlecode.iterm2",
        "Workgroups",
        "-data",
        &hex,
    ]);
    match sys.cmd().run(&spec) {
        Ok(o) if o.success() => Ok(()),
        Ok(o) => Err(Error::Failed(format!(
            "defaults write failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ))),
        Err(e) => Err(Error::Failed(format!("couldn't run defaults: {e}"))),
    }
}

/// Whether iTerm2 is running. `-a`: BSD pgrep leaves out its own
/// ancestors, and run in an iTerm tab, iTerm2 is one of them.
pub fn running(sys: &dyn Sys) -> bool {
    sys.cmd()
        .run(&CommandSpec::new("pgrep").args(["-axq", "iTerm2"]))
        .is_ok_and(|o| o.success())
}

/// The dynamic profile: a copy of the Default profile that ignores titles
/// apps set (claude sets one unconditionally), with the tab title pinned to
/// `user.yoTitle`. Keys sorted and two-space indents, as bash wrote it.
fn profile_json(default: &Value, name: &str) -> String {
    let mut p = default.clone();
    if let Some(o) = p.as_object_mut() {
        o.insert("Name".into(), json!(name));
        o.insert("Guid".into(), json!(format!("{name}-dynamic-profile")));
        o.insert("Allow Title Setting".into(), json!(false));
        o.insert("Title Components".into(), json!(1));
        o.insert("Use Custom Tab Title".into(), json!(true));
        o.insert(
            "Custom Tab Title".into(),
            json!("\\(currentSession.user.yoTitle)"),
        );
    }
    serde_json::to_string_pretty(&json!({ "Profiles": [p] })).unwrap_or_default()
}

impl Iterm {
    fn profile_path(&self, sys: &dyn Sys) -> PathBuf {
        sys.home()
            .join("Library/Application Support/iTerm2/DynamicProfiles")
            .join(format!("{}.json", self.profile))
    }

    /// Writes the profile when it changed, and gives iTerm a moment to
    /// load it. `None`: no Default profile to copy, so the default profile
    /// is used (and claude may overwrite the title).
    fn ensure_profile(&self, sys: &dyn Sys, p: &Prefs) -> Option<String> {
        let content = profile_json(p.default.as_ref()?, &self.profile);
        let path = self.profile_path(sys);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
            crate::write_atomic(&path, content.as_bytes()).ok()?;
            sys.sleep(Duration::from_millis(1500));
        }
        Some(self.profile.clone())
    }
}

impl Runner for Iterm {
    fn kind(&self) -> RunnerKind {
        RunnerKind::Iterm
    }

    fn launch(&self, sys: &dyn Sys, spec: &LaunchSpec) -> Result<Launched> {
        let p = prefs(sys);
        let mut warnings = Vec::new();
        let profile = self.ensure_profile(sys, &p).unwrap_or_else(|| {
            warnings.push(format!(
                "couldn't build the '{}' profile; using default (title may be overwritten by claude)",
                self.profile
            ));
            String::new()
        });
        let new_window = match spec.window {
            WindowChoice::Window => true,
            WindowChoice::Tab => false,
            WindowChoice::Config => self.window_mode == WindowMode::Window,
        };
        let split = match self.layout {
            Layout::Split => true,
            Layout::Workgroup => false,
            Layout::Auto => !p.peer,
        };
        let (w, h) = p.screen.unwrap_or((0, 0));
        let setup = format!("cd {}", shell_word(&spec.dir.to_string_lossy()));
        let marker = format!("yo:{}", spec.group);
        let args = [
            "-".to_string(),
            spec.title.clone(),
            profile,
            setup,
            spec.argv.line(),
            self.width.clone(),
            self.height.clone(),
            w.to_string(),
            h.to_string(),
            marker,
            u8::from(new_window).to_string(),
            u8::from(split).to_string(),
        ];
        let open = CommandSpec {
            stdin: Some(OPEN.as_bytes().to_vec()),
            ..CommandSpec::new("osascript").args(args)
        };
        let out = sys
            .cmd()
            .run(&open)
            .map_err(|e| Error::Failed(format!("osascript: {e}")))?;
        if !out.success() {
            return Err(Error::Failed(format!(
                "couldn't open the iTerm2 session: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let retitle = CommandSpec {
            stdin: Some(RETITLE.as_bytes().to_vec()),
            ..CommandSpec::new("osascript").args(["-".to_string(), spec.title.clone()])
        };
        // Best effort: without it, Session Status may read "Chat".
        let _ = sys.cmd().spawn(&retitle);
        let tab = Some(out.text()).filter(|t| valid_tab(t));
        Ok(Launched::Opened {
            new_window,
            split,
            warnings,
            tab,
        })
    }

    fn focus(
        &self,
        sys: &dyn Sys,
        tty: &str,
        tab: Option<&str>,
        message: Option<&str>,
    ) -> Result<Focused> {
        let spec = CommandSpec {
            stdin: Some(FOCUS.as_bytes().to_vec()),
            ..CommandSpec::new("osascript").args([
                "-".to_string(),
                tty.to_string(),
                message.unwrap_or("").to_string(),
                tab.unwrap_or("").to_string(),
            ])
        };
        let said = sys
            .cmd()
            .run(&spec)
            .ok()
            .filter(|o| o.success())
            .map(|o| o.text());
        Ok(match said.as_deref() {
            Some("found") => Focused::Yes,
            Some("tab") => Focused::TabOnly,
            _ => Focused::NotFound,
        })
    }

    /// Errors are ignored: closing the tab also kills osascript itself.
    fn close_own_tab(&self, sys: &dyn Sys, session: &str, tab: Option<&str>) -> bool {
        let _ = sys
            .cmd()
            .run(&script(CLOSE, &[own_session(session), tab.unwrap_or("")]));
        true
    }

    fn own_tab(&self, sys: &dyn Sys, session: &str) -> Option<String> {
        Iterm::own_tab(sys, session)
    }

    /// Only while iTerm runs (asking it would start it); with it not
    /// running, nothing has a tab.
    fn tabs(&self, sys: &dyn Sys) -> Option<Tabs> {
        if !running(sys) {
            return Some(Tabs::default());
        }
        let out = sys.cmd().run(&script(TABS, &[])).ok()?;
        if !out.success() {
            return None;
        }
        let mut t = Tabs::default();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let (tty, tab) = line.split_once('\t').unwrap_or((line, ""));
            if tty.starts_with("/dev/") {
                t.ttys.push(tty.to_string());
            }
            if valid_tab(tab) {
                t.tabs.push(tab.to_string());
            }
        }
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, TempDir, ok};
    use crate::harness::Argv;

    fn iterm(layout: Layout) -> Iterm {
        Iterm {
            profile: "worksesh".into(),
            layout,
            window_mode: WindowMode::Tab,
            width: "1403".into(),
            height: "824".into(),
        }
    }

    fn spec(window: WindowChoice) -> LaunchSpec {
        LaunchSpec {
            dir: "/w/a b".into(),
            title: "[api] it's \"x\"".into(),
            argv: Argv {
                env: vec![],
                command: "claude".into(),
                args: vec!["--name".into(), "n".into()],
            },
            window,
            group: "yo".into(),
            record: None,
        }
    }

    fn answer_prefs(sys: &FakeSys, json: &str) {
        sys.answer("osascript", &["-l", "JavaScript", "-"], ok(json));
    }

    fn open_args(sys: &FakeSys) -> Vec<String> {
        sys.calls()
            .into_iter()
            .find(|c| c.program == "osascript" && c.args.first().is_some_and(|a| a == "-"))
            .map(|c| {
                assert_eq!(c.stdin.as_deref(), Some(OPEN.as_bytes()));
                c.args
            })
            .unwrap()
    }

    #[test]
    fn a_tab_with_the_profile_and_values_as_arguments() {
        let t = TempDir::new().unwrap();
        let sys = FakeSys::new(t.path());
        answer_prefs(
            &sys,
            r#"{"default": {"Name": "Default", "Font": "Menlo"}, "peer": false, "screen": [1512, 982]}"#,
        );
        let s = spec(WindowChoice::Config);
        let open: Vec<String> = [
            "-",
            &s.title,
            "worksesh",
            "cd '/w/a b'",
            "claude --name n",
            "1403",
            "824",
            "1512",
            "982",
            "yo:yo",
            "0",
            "1",
        ]
        .iter()
        .map(|a| a.to_string())
        .collect();
        let open_strs: Vec<&str> = open.iter().map(String::as_str).collect();
        sys.answer("osascript", &open_strs, ok("pty-W1/40\n"));
        let l = iterm(Layout::Auto).launch(&sys, &s).unwrap();
        // The script prints the tab it opened, for the record.
        assert_eq!(
            l,
            Launched::Opened {
                new_window: false,
                split: true,
                warnings: vec![],
                tab: Some("pty-W1/40".into()),
            }
        );
        assert_eq!(open_args(&sys), open);
        // The profile was written, and iTerm given time to load it.
        let written = std::fs::read_to_string(
            t.path()
                .join("Library/Application Support/iTerm2/DynamicProfiles/worksesh.json"),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&written).unwrap();
        let p = &v["Profiles"][0];
        assert_eq!(p["Name"], "worksesh");
        assert_eq!(p["Font"], "Menlo");
        assert_eq!(p["Allow Title Setting"], false);
        assert_eq!(p["Custom Tab Title"], "\\(currentSession.user.yoTitle)");
        assert_eq!(sys.slept().len(), 1);
        // The title is put back, detached, as an argument too.
        let sp = sys.spawned();
        assert_eq!(sp.len(), 1);
        assert_eq!(sp[0].args, ["-", s.title.as_str()]);

        // Unchanged next time: not rewritten, no wait.
        let _ = iterm(Layout::Auto).launch(&sys, &s);
        assert_eq!(sys.slept().len(), 1);
    }

    #[test]
    fn focus_passes_the_tty_and_message_as_arguments() {
        let sys = FakeSys::new("/h");
        let it = iterm(Layout::Auto);
        assert_eq!(
            it.focus(&sys, "/dev/ttys003", None, Some("hi 'there'"))
                .unwrap(),
            Focused::NotFound
        );
        sys.answer("osascript", &["-", "/dev/ttys003", "", ""], ok("found\n"));
        assert_eq!(
            it.focus(&sys, "/dev/ttys003", None, None).unwrap(),
            Focused::Yes
        );
        let c = &sys.calls()[0];
        assert_eq!(c.args, ["-", "/dev/ttys003", "hi 'there'", ""]);
        assert_eq!(c.stdin.as_deref(), Some(FOCUS.as_bytes()));
        // Its terminal not shown, its tab is: a workgroup showing CLI.
        sys.answer(
            "osascript",
            &["-", "/dev/ttys004", "", "pty-W/40"],
            ok("tab\n"),
        );
        assert_eq!(
            it.focus(&sys, "/dev/ttys004", Some("pty-W/40"), None)
                .unwrap(),
            Focused::TabOnly
        );
    }

    #[test]
    fn closing_by_session_then_by_tab() {
        let sys = FakeSys::new("/h");
        let it = iterm(Layout::Auto);
        assert!(it.close_own_tab(&sys, "w0t1p0:UUID", Some("pty-W/40")));
        assert!(it.close_own_tab(&sys, "w0t1p0:UUID", None));
        let c = sys.calls();
        assert_eq!(c[0].args, ["-", "UUID", "pty-W/40"]);
        assert_eq!(c[1].args, ["-", "UUID", ""]);
        assert_eq!(c[0].stdin.as_deref(), Some(CLOSE.as_bytes()));
    }

    #[test]
    fn tabs_from_the_listing_only_while_iterm_runs() {
        let sys = FakeSys::new("/h");
        let it = iterm(Layout::Auto);
        // Not running: nothing has a tab (and iTerm isn't started to ask).
        assert_eq!(it.tabs(&sys), Some(Tabs::default()));
        assert!(!sys.calls().iter().any(|c| c.program == "osascript"));
        sys.answer("pgrep", &["-axq", "iTerm2"], ok(""));
        // Running, but the listing failed: it can't say.
        assert_eq!(it.tabs(&sys), None);
        sys.answer(
            "osascript",
            &["-"],
            ok("/dev/ttys003\tpty-W/27\n/dev/ttys007\t/\njunk\n"),
        );
        assert_eq!(
            it.tabs(&sys),
            Some(Tabs {
                ttys: vec!["/dev/ttys003".into(), "/dev/ttys007".into()],
                tabs: vec!["pty-W/27".into()],
            })
        );
    }

    #[test]
    fn windows_layouts_and_no_profile() {
        let sys = FakeSys::new("/nonexistent-home");
        // A workgroup peer is the shell, so no split; no prefs, no profile.
        answer_prefs(&sys, r#"{"peer": true}"#);
        let r = iterm(Layout::Auto).launch(&sys, &spec(WindowChoice::Window));
        // The open itself failed (no answer), which is an error.
        assert!(matches!(r, Err(Error::Failed(_))));
        let a = open_args(&sys);
        assert_eq!(
            (a[2].as_str(), a[10].as_str(), a[11].as_str()),
            ("", "1", "0")
        );
        assert_eq!((a[7].as_str(), a[8].as_str()), ("0", "0"));

        let sys = FakeSys::new("/nonexistent-home");
        sys.answer("osascript", &[], ok(""));
        let _ = iterm(Layout::Split).launch(&sys, &spec(WindowChoice::Tab));
        let a = open_args(&sys);
        assert_eq!((a[10].as_str(), a[11].as_str()), ("0", "1"));
    }
}
