//! Runners: how a session runs (an iTerm tab; later ACP, another machine).
//!
//! So far: the kind, for ledger records; launching, focusing and closing,
//! by [`iterm::Iterm`] or [`DryLaunch`] (`YO_DRY_LAUNCH`), which says what
//! it would do and does nothing; and the tabs a runner shows, for telling a
//! running session with a tab from one whose tab is gone ([`tab_of`]).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::harness::Argv;
use crate::ledger::Key;
use crate::sys::Sys;

pub mod iterm;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    Iterm,
}

pub trait Runner {
    fn kind(&self) -> RunnerKind;
    fn launch(&self, sys: &dyn Sys, spec: &LaunchSpec) -> Result<Launched>;
    /// Brings forward the session on terminal `tty` (`/dev/ttys003`) and
    /// types `message` into it, if given; else the tab `tab` (as
    /// [`Launched::Opened`] gave it), if given, with nothing typed.
    /// `Focused::NotFound` when it finds neither.
    fn focus(
        &self,
        sys: &dyn Sys,
        tty: &str,
        tab: Option<&str>,
        message: Option<&str>,
    ) -> Result<Focused>;
    /// Closes the tab holding `session`, the one yo runs in (which may end
    /// yo too), or else the tab `tab`. `false`: nothing was closed (a dry
    /// run).
    fn close_own_tab(&self, sys: &dyn Sys, session: &str, tab: Option<&str>) -> bool;
    /// The terminals and tabs it shows, for [`tab_of`]. `None`: it can't
    /// say.
    fn tabs(&self, _sys: &dyn Sys) -> Option<Tabs> {
        None
    }
    /// The tab holding `session`, the one yo runs in, as [`Launched::Opened`]
    /// gives tabs. `None`: it can't say.
    fn own_tab(&self, _sys: &dyn Sys, _session: &str) -> Option<String> {
        None
    }
    /// Whether it really ends sessions (a dry runner says it would).
    fn ends_sessions(&self) -> bool {
        true
    }
}

/// What a runner shows: the terminals of its sessions, and their tabs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tabs {
    /// `/dev/ttys003`.
    pub ttys: Vec<String>,
    pub tabs: Vec<String>,
}

/// Whether a running session still has a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Held,
    /// The runner shows neither its terminal nor its tab: closing the tab
    /// left it running.
    Gone,
    /// The runner can't say, or the session's tab was never recorded and
    /// its terminal isn't shown (it may be a hidden member of its tab).
    Unknown,
}

/// Whether the session running as `pid`, whose tab was recorded as `tab`,
/// still has one, by what the runner shows.
pub fn tab_of(sys: &dyn Sys, tab: Option<&str>, pid: u32, shown: Option<&Tabs>) -> Tab {
    let Some(shown) = shown else {
        return Tab::Unknown;
    };
    let tty = sys.procs().tty(pid).map(|t| {
        if t.starts_with("/dev/") {
            t
        } else {
            format!("/dev/{t}")
        }
    });
    if tty.is_some_and(|t| shown.ttys.contains(&t))
        || tab.is_some_and(|t| shown.tabs.iter().any(|s| s == t))
    {
        Tab::Held
    } else if tab.is_some() {
        Tab::Gone
    } else {
        Tab::Unknown
    }
}

/// Whether `t` looks like a tab a runner gave: `<window id>/<tab id>`, of
/// letters, digits and `-`. It reaches osascript only as an argument, but
/// a record edited by hand to hold something else counts as no tab.
pub fn valid_tab(t: &str) -> bool {
    t.len() <= 128
        && t.split_once('/').is_some_and(|(w, n)| {
            !w.is_empty()
                && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit())
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focused {
    Yes,
    /// Its tab, but not the session itself: its tab shows something else
    /// (a workgroup's CLI), so nothing was typed.
    TabOnly,
    NotFound,
    /// Nothing ran: what would have.
    Dry {
        tty: String,
    },
}

/// A session to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec {
    pub dir: PathBuf,
    pub title: String,
    pub argv: Argv,
    pub window: WindowChoice,
    /// The shared window it joins, unless it gets a window of its own.
    pub group: String,
    /// The ledger record that gets the new tab, once it's open.
    pub record: Option<Key>,
}

/// Where a session's tab goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowChoice {
    /// A tab in the shared window (`--tab`).
    Tab,
    /// A window of its own (`--new-window`).
    Window,
    /// What `YO_WINDOW_MODE` says.
    #[default]
    Config,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launched {
    /// Nothing ran: the line that would have.
    Dry { line: String },
    /// In a new window or a tab; split with a shell beside it, or not.
    Opened {
        new_window: bool,
        split: bool,
        warnings: Vec<String>,
        /// The tab it opened in, if the runner could say.
        tab: Option<String>,
    },
}

/// `YO_DRY_LAUNCH`: says what it would launch, and launches nothing.
#[derive(Debug, Default)]
pub struct DryLaunch;

impl Runner for DryLaunch {
    fn kind(&self) -> RunnerKind {
        RunnerKind::Iterm
    }

    fn launch(&self, _sys: &dyn Sys, spec: &LaunchSpec) -> Result<Launched> {
        Ok(Launched::Dry {
            line: spec.argv.line(),
        })
    }

    fn focus(
        &self,
        _sys: &dyn Sys,
        tty: &str,
        _tab: Option<&str>,
        _message: Option<&str>,
    ) -> Result<Focused> {
        Ok(Focused::Dry {
            tty: tty.to_string(),
        })
    }

    fn close_own_tab(&self, _sys: &dyn Sys, _session: &str, _tab: Option<&str>) -> bool {
        false
    }

    fn ends_sessions(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSys;

    #[test]
    fn a_session_has_a_tab_by_its_terminal_or_its_recorded_tab() {
        let sys = FakeSys::new("/h");
        sys.add_process(30, "claude", Some("ttys009"));
        sys.add_process(31, "claude", Some("ttys010"));
        let shown = Tabs {
            ttys: vec!["/dev/ttys009".into(), "/dev/ttys011".into()],
            tabs: vec!["pty-W/40".into(), "pty-W/41".into()],
        };
        let s = Some(&shown);
        assert_eq!(tab_of(&sys, None, 30, s), Tab::Held);
        // Its terminal isn't shown, but its tab is (a workgroup showing CLI).
        assert_eq!(tab_of(&sys, Some("pty-W/41"), 31, s), Tab::Held);
        assert_eq!(tab_of(&sys, Some("pty-W/7"), 31, s), Tab::Gone);
        // No tab recorded: it may be hidden in one, so yo can't say.
        assert_eq!(tab_of(&sys, None, 31, s), Tab::Unknown);
        assert_eq!(tab_of(&sys, Some("pty-W/7"), 31, None), Tab::Unknown);
    }

    #[test]
    fn tabs_are_checked() {
        assert!(valid_tab("pty-D1673866-7462-4209-819C-1416E93F1CD2/31"));
        for bad in ["", "w", "/3", "w/", "w/x", "w x/3", "w/3\n", "a'b/1"] {
            assert!(!valid_tab(bad), "{bad}");
        }
    }
}
