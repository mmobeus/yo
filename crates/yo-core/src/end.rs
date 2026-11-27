//! Ending a running session the way a closed terminal would: a hangup to
//! the shell it runs under (which passes it on, then exits), then to the
//! session itself, then SIGTERM.
//!
//! `down` needs this because closing a tab doesn't always end what runs in
//! it: see "Closing a tab" in [`crate::runner::iterm`].
//!
//! Before each signal it checks that the harness still lists the session
//! with that pid, so a pid that was reused, or a session that already
//! ended, is never signalled.

use std::time::Duration;

use crate::Ctx;
use crate::sys::Signal;

/// How long `yo _end-session` gives a session to end with its tab before
/// ending it: past iTerm's undo window (5 seconds by default), in which a
/// closed tab's session still runs, and may be brought back.
pub const AFTER_CLOSE: Duration = Duration::from_secs(8);

/// How long each signal gets to work before the next.
const GRACE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(250);

/// What [`end_session`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// It wasn't running (or ended while we waited): nothing was sent.
    NotRunning,
    /// It ended after this signal.
    By(Signal),
    /// It was still running after SIGTERM.
    Survived,
}

/// Ends session `id` (process `pid`), after first giving it `wait` to end
/// by itself.
pub fn end_session(ctx: &Ctx, id: &str, pid: u32, wait: Duration) -> Ended {
    if !running_within(ctx, id, pid, wait) {
        return Ended::NotRunning;
    }
    let procs = ctx.sys.procs();
    // The shell above it, when it's on the same terminal: hanging that up
    // is what closing the terminal does, and it takes the shell with it.
    let tty = procs.tty(pid);
    let shell = procs
        .ppid(pid)
        .filter(|&s| s > 1 && tty.is_some() && procs.tty(s) == tty);
    let steps = [
        (shell, Signal::Hup),
        (Some(pid), Signal::Hup),
        (Some(pid), Signal::Term),
    ];
    for (target, sig) in steps {
        let Some(target) = target else { continue };
        if !is_running(ctx, id, pid) {
            return Ended::NotRunning;
        }
        // The shell is checked again: still this session's parent.
        if target != pid && procs.ppid(pid) != Some(target) {
            continue;
        }
        procs.signal(target, sig);
        if !running_within(ctx, id, pid, GRACE) {
            return Ended::By(sig);
        }
    }
    Ended::Survived
}

fn is_running(ctx: &Ctx, id: &str, pid: u32) -> bool {
    ctx.live().iter().any(|l| l.id == id && l.pid == pid)
}

/// Whether it's still running after up to `d`, checked every [`POLL`].
fn running_within(ctx: &Ctx, id: &str, pid: u32, d: Duration) -> bool {
    let mut left = d;
    loop {
        if !is_running(ctx, id, pid) {
            return false;
        }
        if left.is_zero() {
            return true;
        }
        let step = left.min(POLL);
        ctx.sys.sleep(step);
        left -= step;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeSys, TempDir};

    const ID: &str = "s1";

    fn ctx(t: &TempDir) -> (Ctx, Arc<FakeSys>) {
        let sys = Arc::new(FakeSys::new(t.path()));
        let dir = t.path().join(".claude/sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("30.json"),
            format!(r#"{{"pid":30,"sessionId":"{ID}","cwd":"/w","kind":"interactive"}}"#),
        )
        .unwrap();
        // login (10) → zsh (20) → claude (30), on one terminal.
        sys.add_process(10, "login", Some("ttys009"));
        sys.add_child(20, "-zsh", Some("ttys009"), 10);
        sys.add_child(30, "claude", Some("ttys009"), 20);
        let c = Ctx::load(sys.clone(), t.path()).unwrap();
        (c, sys)
    }

    #[test]
    fn hangs_up_its_shell_first() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        assert_eq!(
            end_session(&c, ID, 30, Duration::ZERO),
            Ended::By(Signal::Hup)
        );
        assert_eq!(sys.signals(), [(20, Signal::Hup)]);
    }

    #[test]
    fn then_the_session_then_sigterm() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        sys.ignore_signal(20, Signal::Hup);
        sys.ignore_signal(30, Signal::Hup);
        assert_eq!(
            end_session(&c, ID, 30, Duration::ZERO),
            Ended::By(Signal::Term)
        );
        assert_eq!(
            sys.signals(),
            [(20, Signal::Hup), (30, Signal::Hup), (30, Signal::Term)]
        );
    }

    #[test]
    fn says_when_it_survives() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        sys.ignore_signal(20, Signal::Hup);
        sys.ignore_signal(30, Signal::Hup);
        sys.ignore_signal(30, Signal::Term);
        assert_eq!(end_session(&c, ID, 30, Duration::ZERO), Ended::Survived);
    }

    #[test]
    fn a_shell_on_another_terminal_is_left_alone() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        sys.add_process(20, "-zsh", Some("ttys004"));
        assert_eq!(
            end_session(&c, ID, 30, Duration::ZERO),
            Ended::By(Signal::Hup)
        );
        assert_eq!(sys.signals(), [(30, Signal::Hup)]);
    }

    #[test]
    fn nothing_is_sent_to_a_session_that_ended_or_a_reused_pid() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        // Another session's id on this pid: not ours.
        assert_eq!(
            end_session(&c, "other", 30, Duration::from_secs(8)),
            Ended::NotRunning
        );
        // The pid now runs something that isn't claude.
        sys.add_child(30, "vim", Some("ttys009"), 20);
        assert_eq!(end_session(&c, ID, 30, Duration::ZERO), Ended::NotRunning);
        assert!(sys.signals().is_empty());
    }

    #[test]
    fn waits_first_when_asked() {
        let t = TempDir::new().unwrap();
        let (c, sys) = ctx(&t);
        assert_eq!(
            end_session(&c, ID, 30, Duration::from_secs(8)),
            Ended::By(Signal::Hup)
        );
        let waited: Duration = sys.slept().iter().sum();
        assert_eq!(waited, Duration::from_secs(8));
    }
}
