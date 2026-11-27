//! yo's ledger hook in Codex's `hooks.json` (`~/.codex/hooks.json`): one
//! `SessionEnd` command, `'<yo>' _session-hook codex`, which `yo setup`
//! adds when asked and takes out on `--uninstall`. Everything else in the
//! file stays as it is.
//!
//! Codex runs a hook only once it's been approved in its `/hooks` screen,
//! and keys that approval by the hook's place in the file
//! (`<hooks.json>:session_end:<group>:<index>`), so yo's hook keeps its
//! place: a changed command is changed where it is.

use std::path::Path;

use crate::ojson::OValue;

/// What marks a command as yo's hook, whichever yo it names.
const MARK: &str = " _session-hook codex";

/// The hook's command, for the yo at `exe`.
pub fn command(exe: &Path) -> String {
    format!(
        "{}{MARK}",
        super::super::single_quote(&exe.to_string_lossy())
    )
}

fn is_ours(h: &OValue) -> bool {
    matches!(h.get("command"), Some(OValue::String(c)) if c.ends_with(MARK))
}

fn groups(v: &OValue) -> &[OValue] {
    v.get("hooks")
        .and_then(|h| h.get("SessionEnd"))
        .and_then(OValue::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Where yo's hook is (group, index), and its command.
pub fn find(v: &OValue) -> Option<(usize, usize, String)> {
    groups(v).iter().enumerate().find_map(|(g, group)| {
        let hooks = group.get("hooks").and_then(OValue::as_array)?;
        hooks
            .iter()
            .enumerate()
            .find_map(|(i, h)| match h.get("command") {
                Some(OValue::String(c)) if is_ours(h) => Some((g, i, c.clone())),
                _ => None,
            })
    })
}

/// `v` with yo's hook running `cmd`: changed in place if it's there, else
/// a group of its own at the end. `None` when `v` isn't shaped like a
/// hooks file.
pub fn with_hook(v: &OValue, cmd: &str) -> Option<OValue> {
    let hook = OValue::Object(vec![
        ("type".into(), OValue::String("command".into())),
        ("command".into(), OValue::String(cmd.into())),
    ]);
    let mut next = v.clone();
    let groups = next.entry_object("hooks")?;
    if groups.get("SessionEnd").is_none() {
        groups.insert("SessionEnd", OValue::Array(Vec::new()));
    }
    let OValue::Array(list) = groups.get_mut("SessionEnd")? else {
        return None;
    };
    match find(v) {
        Some((g, i, _)) => {
            let OValue::Array(hooks) = list.get_mut(g)?.get_mut("hooks")? else {
                return None;
            };
            hooks[i] = hook;
        }
        None => list.push(OValue::Object(vec![(
            "hooks".into(),
            OValue::Array(vec![hook]),
        )])),
    }
    Some(next)
}

/// `v` without yo's hook (and without a group or event it leaves empty).
/// `None` when it has none.
pub fn without_hook(v: &OValue) -> Option<OValue> {
    let (g, i, _) = find(v)?;
    let mut next = v.clone();
    let groups = next.get_mut("hooks")?;
    let OValue::Array(list) = groups.get_mut("SessionEnd")? else {
        return None;
    };
    let OValue::Array(hooks) = list[g].get_mut("hooks")? else {
        return None;
    };
    hooks.remove(i);
    if hooks.is_empty() {
        list.remove(g);
    }
    if list.is_empty() {
        groups.remove("SessionEnd");
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> OValue {
        OValue::parse(s).unwrap()
    }

    #[test]
    fn the_hook_is_added_beside_others_and_found() {
        let theirs = p(
            r#"{"x":1,"hooks":{"SessionEnd":[{"hooks":[{"type":"command","command":"mine"}]}],"Stop":[]}}"#,
        );
        let cmd = command(Path::new("/opt/yo's/bin/yo"));
        assert_eq!(cmd, "'/opt/yo'\\''s/bin/yo' _session-hook codex");
        let next = with_hook(&theirs, &cmd).unwrap();
        assert_eq!(find(&next), Some((1, 0, cmd.clone())));
        assert_eq!(
            next.to_compact(),
            format!(
                r#"{{"x":1,"hooks":{{"SessionEnd":[{{"hooks":[{{"type":"command","command":"mine"}}]}},{{"hooks":[{{"type":"command","command":{}}}]}}],"Stop":[]}}}}"#,
                serde_json::to_string(&cmd).unwrap()
            )
        );
        // A new yo changes the command where it is, so Codex's key for it
        // stays the same.
        let moved = with_hook(&next, "'/new/yo' _session-hook codex").unwrap();
        assert_eq!(find(&moved).map(|(g, i, _)| (g, i)), Some((1, 0)));
        assert_eq!(without_hook(&moved), Some(theirs));
    }

    #[test]
    fn a_fresh_file_and_a_misshapen_one() {
        let next = with_hook(&OValue::object(), "'/yo' _session-hook codex").unwrap();
        assert_eq!(
            next.to_compact(),
            r#"{"hooks":{"SessionEnd":[{"hooks":[{"type":"command","command":"'/yo' _session-hook codex"}]}]}}"#
        );
        assert_eq!(without_hook(&next), Some(p(r#"{"hooks":{}}"#)));
        assert_eq!(with_hook(&p(r#"{"hooks":[]}"#), "c"), None);
        assert_eq!(without_hook(&p(r#"{"hooks":{}}"#)), None);
    }
}
