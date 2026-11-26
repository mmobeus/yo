//! Runners: how a session runs (an iTerm tab; later ACP, another machine).
//!
//! So far: the kind, for ledger records, and launching, by [`iterm::Iterm`]
//! or [`DryLaunch`] (`YO_DRY_LAUNCH`), which says what it would run and runs
//! nothing. The rest of the trait (locate, deliver, focus, close, retitle)
//! arrives with the commands that use them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::harness::Argv;
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
    /// types `message` into it, if given. `Focused::NotFound` when no tab
    /// of this runner's has that terminal.
    fn focus(&self, sys: &dyn Sys, tty: &str, message: Option<&str>) -> Result<Focused>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focused {
    Yes,
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

    fn focus(&self, _sys: &dyn Sys, tty: &str, _message: Option<&str>) -> Result<Focused> {
        Ok(Focused::Dry {
            tty: tty.to_string(),
        })
    }
}
