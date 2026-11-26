//! Runners: how a session runs (an iTerm tab; later ACP, another machine).
//!
//! So far: the kind, for ledger records, and launching. [`DryLaunch`] stands
//! in for iTerm (`YO_DRY_LAUNCH`): it says what it would run and runs
//! nothing. The iTerm runner and the rest of the trait (locate, deliver,
//! focus, close, retitle) arrive with the commands that use them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::harness::Argv;
use crate::sys::Sys;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    Iterm,
}

pub trait Runner {
    fn kind(&self) -> RunnerKind;
    fn launch(&self, sys: &dyn Sys, spec: &LaunchSpec) -> Result<Launched>;
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
}
