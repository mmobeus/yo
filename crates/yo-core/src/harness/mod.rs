//! Harnesses: the coding agents' CLIs yo runs (`claude`, later `codex`).
//! What's specific to one lives in its module; the core asks through
//! [`Harness`].
//!
//! The trait holds what's ported so far. Launching and resuming
//! (`tui_command`, `resume_command`), preparing a session and the rest of
//! the design's trait arrive with the commands that use them.

use std::path::PathBuf;

use crate::ledger::HookEvent;
use crate::sys::Sys;

pub mod claude;

pub trait Harness: Send + Sync {
    /// `"claude"`, `"codex"`: what a ledger record's `harness` says.
    fn name(&self) -> &str;
    /// Its sessions that are running now.
    fn live(&self, sys: &dyn Sys) -> Vec<LiveSession>;
    fn capabilities(&self) -> Capabilities;
    /// Its ledger hook's payload as a start or an end, for a harness with
    /// `ledger_hooks`. `None` for anything it doesn't recognize.
    fn hook_event(&self, _payload: &str) -> Option<HookEvent> {
        None
    }
}

/// What a harness has beyond the required parts. Anything it lacks shows
/// up as "not available", never as a failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// A settings layer composed per session (Claude: `--settings`).
    pub settings_layer: bool,
    /// Shared memory (Claude: `autoMemoryDirectory`).
    pub memory: bool,
    /// Personas: yo's `agents` (Claude: `--agent`).
    pub personas: bool,
    /// Hooks that keep the ledger current (Claude: `SessionStart`,
    /// `SessionEnd` calling `yo _session-hook`).
    pub ledger_hooks: bool,
    pub remote_control: bool,
    /// `--model`
    pub models: bool,
}

/// A session a harness says is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSession {
    /// The harness's name for itself.
    pub harness: String,
    /// The harness's session id.
    pub id: String,
    pub pid: u32,
    pub cwd: PathBuf,
    /// The harness's name for the session, if it has one.
    pub name: Option<String>,
    /// When the harness last updated it, in its own units (Claude: unix ms).
    /// Only compared with others from the same harness.
    pub updated_at: i64,
}

/// Every harness yo has.
pub fn all() -> Vec<Box<dyn Harness>> {
    vec![Box::new(claude::Claude)]
}
