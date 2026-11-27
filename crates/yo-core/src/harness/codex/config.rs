//! Codex's `config.toml`, read for the few tables yo looks at, and
//! appended to. yo never rewrites what's there (comments and all), and
//! leaves a file it can't follow alone.
//!
//! This isn't a TOML parser. It knows table headers (`[a."b"]`,
//! `[[a]]`), `key = value` lines with string values, and skips multi-line
//! strings and arrays. Anything else marks the file as one yo doesn't
//! append to. A misreading costs at most a question asked when it needn't
//! be, never Codex's own reading of the file.

use std::path::Path;

/// One key the file defines: its full path (the table's, then the key's),
/// and its value when that's a plain string.
#[derive(Debug)]
struct Def {
    path: Vec<String>,
    value: Option<String>,
    /// A table header rather than a key.
    table: bool,
}

#[derive(Debug)]
pub struct Config {
    defs: Vec<Def>,
    /// Every line was one this module follows.
    followed: bool,
}

/// Reads `text`. Never fails: what isn't followed is noted, not refused.
pub fn read(text: &str) -> Config {
    let mut defs = Vec::new();
    let mut followed = true;
    let mut table: Vec<String> = Vec::new();
    // Inside a multi-line string, its closing quotes; inside a multi-line
    // array or inline table, how deep.
    let mut closer: Option<&str> = None;
    let mut depth = 0usize;
    for line in text.lines() {
        if let Some(c) = closer {
            if line.contains(c) {
                closer = None;
            }
            continue;
        }
        if depth > 0 {
            depth = nest(line, depth);
            continue;
        }
        let l = line.trim_start();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("[[").or_else(|| l.strip_prefix('[')) {
            let close = if l.starts_with("[[") { "]]" } else { "]" };
            match key(rest) {
                Some((path, after)) if after.trim_start().starts_with(close) => {
                    table = path.clone();
                    defs.push(Def {
                        path,
                        value: None,
                        table: true,
                    });
                }
                _ => {
                    followed = false;
                    table.clear();
                }
            }
            continue;
        }
        let Some((k, after)) = key(l) else {
            followed = false;
            continue;
        };
        let Some(v) = after.trim_start().strip_prefix('=') else {
            followed = false;
            continue;
        };
        let v = v.trim_start();
        let mut path = table.clone();
        path.extend(k);
        let mut value = None;
        if let Some(q) = ["\"\"\"", "'''"].into_iter().find(|q| v.starts_with(q)) {
            if !v[3..].contains(q) {
                closer = Some(q);
            }
        } else if v.starts_with('"') || v.starts_with('\'') {
            value = string(v).map(|(s, _)| s);
        } else if v.starts_with('[') || v.starts_with('{') {
            depth = nest(v, 0);
        }
        defs.push(Def {
            path,
            value,
            table: false,
        });
    }
    if closer.is_some() || depth > 0 {
        followed = false;
    }
    Config { defs, followed }
}

impl Config {
    fn has(&self, prefix: &[&str]) -> bool {
        self.defs
            .iter()
            .any(|d| d.path.len() >= prefix.len() && d.path.iter().zip(prefix).all(|(a, b)| a == b))
    }

    /// Whether Codex trusts `dir`: the trust level of the nearest entry
    /// for it or a directory above it, `trusted` or not. `None`: no entry,
    /// so Codex asks. (Codex trusts a worktree through its main clone's
    /// entry, so `dir` is the clone.)
    pub fn trust(&self, dir: &Path) -> Option<bool> {
        self.defs
            .iter()
            .filter_map(|d| match (d.path.as_slice(), &d.value) {
                ([projects, p, level], Some(v))
                    if projects == "projects" && level == "trust_level" =>
                {
                    let p = Path::new(p);
                    dir.starts_with(p)
                        .then(|| (p.components().count(), v == "trusted"))
                }
                _ => None,
            })
            .max_by_key(|(n, _)| *n)
            .map(|(_, t)| t)
    }

    /// Whether the file sets Codex's status line (`tui.status_line`, or a
    /// `tui = { ... }` that might).
    pub fn sets_status_line(&self) -> bool {
        self.has(&["tui", "status_line"])
            || self
                .defs
                .iter()
                .any(|d| !d.table && d.path.len() == 1 && d.path[0] == "tui")
    }

    /// Whether the hook Codex keys as `key` (`<hooks.json>:session_end:0:0`)
    /// has been approved in its `/hooks` screen.
    pub fn hook_approved(&self, key: &str) -> bool {
        self.has(&["hooks", "state", key, "trusted_hash"])
    }
}

/// `text` with `dir` trusted: one table appended. `None` when `dir`
/// already has an entry (of whatever level), or the file isn't one this
/// module follows well enough to append to.
pub fn with_trust(text: &str, dir: &Path) -> Option<String> {
    let c = read(text);
    let d = dir.to_str()?;
    // `projects = { ... }` can't be added to with a table.
    let inline = c
        .defs
        .iter()
        .any(|d| !d.table && d.path.len() == 1 && d.path[0] == "projects");
    if !c.followed || inline || c.has(&["projects", d]) {
        return None;
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!(
        "[projects.{}]\ntrust_level = \"trusted\"\n",
        super::toml_string(d)
    ));
    (read(&out).trust(dir) == Some(true)).then_some(out)
}

/// A dotted key at the start of `s`, and what follows it.
fn key(s: &str) -> Option<(Vec<String>, &str)> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        rest = rest.trim_start();
        let (seg, after) = if rest.starts_with('"') || rest.starts_with('\'') {
            string(rest)?
        } else {
            let n = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(rest.len());
            if n == 0 {
                return None;
            }
            (rest[..n].to_string(), &rest[n..])
        };
        out.push(seg);
        let after_ws = after.trim_start();
        match after_ws.strip_prefix('.') {
            Some(r) => rest = r,
            None => return Some((out, after)),
        }
    }
}

/// A one-line string at the start of `s` (basic or literal), unescaped,
/// and what follows it.
fn string(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    let (_, q) = chars.next()?;
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            _ if c == q => return Some((out, &s[i + 1..])),
            '\\' if q == '"' => {
                let (_, e) = chars.next()?;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' | 'U' => {
                        let n = if e == 'u' { 4 } else { 8 };
                        let hex: String = (0..n)
                            .filter_map(|_| chars.next().map(|(_, c)| c))
                            .collect();
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    e => out.push(e),
                }
            }
            c => out.push(c),
        }
    }
    None
}

/// The bracket depth after `line`, starting at `depth`, skipping strings
/// and comments.
fn nest(line: &str, mut depth: usize) -> usize {
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        match c {
            '"' | '\'' => match string(rest) {
                Some((_, after)) => {
                    rest = after;
                    continue;
                }
                None => return depth,
            },
            '#' => return depth,
            '[' | '{' => depth += 1,
            ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        rest = &rest[c.len_utf8()..];
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX: &str = r#"# mine
model = "gpt-6"
developer_instructions = """
[projects."/fake"]
trust_level = "trusted"
"""
sandbox = [
  "a", # [projects."/also-fake"]
  "b",
]

[tui]
screen_reader_detection_done = true

[projects."/w/repo"]
trust_level = "trusted"

[projects.'/w/other']
trust_level = "untrusted"

[hooks.state."/h/.codex/hooks.json:session_end:0:0"]
trusted_hash = "sha256:ab"
"#;

    #[test]
    fn trust_is_the_nearest_entry_at_or_above() {
        let c = read(CODEX);
        assert!(c.followed);
        assert_eq!(c.trust(Path::new("/w/repo")), Some(true));
        assert_eq!(c.trust(Path::new("/w/repo/sub")), Some(true));
        assert_eq!(c.trust(Path::new("/w/other")), Some(false));
        assert_eq!(
            c.trust(Path::new("/w/repo2")),
            None,
            "a sibling, not a child"
        );
        assert_eq!(
            c.trust(Path::new("/fake")),
            None,
            "inside a multi-line string"
        );
        assert_eq!(c.trust(Path::new("/also-fake")), None, "inside an array");
    }

    #[test]
    fn hook_approval_is_its_state_entry() {
        let c = read(CODEX);
        assert!(c.hook_approved("/h/.codex/hooks.json:session_end:0:0"));
        assert!(!c.hook_approved("/h/.codex/hooks.json:session_end:1:0"));
    }

    #[test]
    fn a_status_line_is_the_persons_in_any_spelling() {
        assert!(!read(CODEX).sets_status_line(), "[tui] without one");
        assert!(!read("[tui]\nstatus_line_use_colors = false\n").sets_status_line());
        assert!(read("[tui]\nstatus_line = [\n  \"model\",\n]\n").sets_status_line());
        assert!(read("tui.status_line = [\"model\"]\n").sets_status_line());
        assert!(read("tui = { status_line = [\"model\"] }\n").sets_status_line());
        assert!(!read("[profiles.x]\ntui = 1\n").sets_status_line());
    }

    #[test]
    fn trusting_appends_and_keeps_the_rest() {
        let out = with_trust(CODEX, Path::new("/w/new \"one\"")).unwrap();
        assert!(out.starts_with(CODEX), "what was there stays as it was");
        assert_eq!(
            &out[CODEX.len()..],
            "\n[projects.\"/w/new \\\"one\\\"\"]\ntrust_level = \"trusted\"\n"
        );
        assert_eq!(read(&out).trust(Path::new("/w/new \"one\"")), Some(true));
        // A file without a newline at the end, and no file.
        assert!(
            with_trust("a = 1", Path::new("/x"))
                .unwrap()
                .starts_with("a = 1\n\n[projects")
        );
        assert_eq!(
            with_trust("", Path::new("/x")).unwrap(),
            "[projects.\"/x\"]\ntrust_level = \"trusted\"\n"
        );
    }

    #[test]
    fn nothing_is_appended_to_an_entry_or_a_file_it_cant_follow() {
        assert_eq!(
            with_trust(CODEX, Path::new("/w/repo")),
            None,
            "already trusted"
        );
        assert_eq!(
            with_trust(CODEX, Path::new("/w/other")),
            None,
            "the person said no"
        );
        let dotted = "projects.\"/x\".trust_level = \"untrusted\"\n";
        assert_eq!(with_trust(dotted, Path::new("/x")), None);
        let inline = "projects = { \"/y\" = { trust_level = \"trusted\" } }\n";
        assert_eq!(with_trust(inline, Path::new("/x")), None);
        assert_eq!(
            with_trust("s = \"\"\"\nopen\n", Path::new("/x")),
            None,
            "unclosed string"
        );
        assert_eq!(with_trust("what is this\n", Path::new("/x")), None);
    }
}
