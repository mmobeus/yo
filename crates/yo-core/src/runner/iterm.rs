//! The iTerm2 runner: a session in a tab (or a window), with a shell beside
//! it. bash's `launch_session`, `ensure_iterm_profile` and
//! `open_iterm_session`.
//!
//! Its AppleScript and JavaScript are files beside this one, embedded, and
//! run through [`Sys::cmd`] with every value as an argument.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::{LaunchSpec, Launched, Runner, RunnerKind, WindowChoice};
use crate::harness::shell_word;
use crate::registry::{Defaults, Layout, WindowMode};
use crate::sys::{CommandSpec, Sys};
use crate::{Error, Result};

const PREFS: &str = include_str!("iterm/prefs.js");
const OPEN: &str = include_str!("iterm/open.applescript");
const RETITLE: &str = include_str!("iterm/retitle.applescript");

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
        screen,
    }
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
        Ok(Launched::Opened {
            new_window,
            split,
            warnings,
        })
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
        sys.answer("osascript", &open_strs, ok(""));
        let l = iterm(Layout::Auto).launch(&sys, &s).unwrap();
        assert_eq!(
            l,
            Launched::Opened {
                new_window: false,
                split: true,
                warnings: vec![]
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
