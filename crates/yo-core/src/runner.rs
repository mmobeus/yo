//! Runners: how a session runs (an iTerm tab; later ACP, another machine).
//!
//! Only the kind is here so far, for ledger records. The trait's methods
//! (launch, locate, deliver, focus, close, retitle) and the iTerm runner
//! arrive with the commands that use them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    Iterm,
}

pub trait Runner {
    fn kind(&self) -> RunnerKind;
}
