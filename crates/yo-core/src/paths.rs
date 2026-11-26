//! Where yo's files live, from `HOME` and the XDG variables, as bash has it.

use std::path::{Path, PathBuf};

use crate::sys::Sys;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The config file: `$YO_CONFIG`, else `$XDG_CONFIG_HOME/yo/config.json`
    /// (`~/.config` when unset). It may not exist.
    pub config: PathBuf,
    /// yo's state directory: `$XDG_STATE_HOME/yo` (`~/.local/state`).
    pub state: PathBuf,
}

impl Paths {
    pub fn new(sys: &dyn Sys) -> Self {
        let home = sys.home();
        let config = sys.env_nonempty("YO_CONFIG").map_or_else(
            || xdg(sys, &home, "XDG_CONFIG_HOME", ".config").join("yo/config.json"),
            PathBuf::from,
        );
        let state = xdg(sys, &home, "XDG_STATE_HOME", ".local/state").join("yo");
        Paths { config, state }
    }

    /// The ledger: one JSON file per session.
    pub fn sessions(&self) -> PathBuf {
        self.state.join("sessions")
    }

    /// Composed per-session settings layers.
    pub fn settings(&self) -> PathBuf {
        self.state.join("settings")
    }

    pub fn machine_id(&self) -> PathBuf {
        self.state.join("machine-id")
    }
}

fn xdg(sys: &dyn Sys, home: &Path, var: &str, fallback: &str) -> PathBuf {
    sys.env_nonempty(var)
        .map_or_else(|| home.join(fallback), PathBuf::from)
}

/// A leading `~` or `~/` becomes `HOME` (a `~/` written in the JSON config
/// never met a shell).
pub fn expand_home(home: &Path, p: &str) -> PathBuf {
    if p == "~" {
        home.to_path_buf()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// `p` without trailing slashes (but `/` stays `/`).
pub fn strip_trailing_slash(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    if t.is_empty() && p.starts_with('/') {
        "/"
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSys;

    #[test]
    fn defaults_under_home() {
        let sys = FakeSys::new("/h");
        let p = Paths::new(&sys);
        assert_eq!(p.config, PathBuf::from("/h/.config/yo/config.json"));
        assert_eq!(p.sessions(), PathBuf::from("/h/.local/state/yo/sessions"));
    }

    #[test]
    fn yo_config_beats_xdg_and_empty_is_unset() {
        let sys = FakeSys::new("/h");
        sys.set_env("XDG_CONFIG_HOME", "/x");
        sys.set_env("XDG_STATE_HOME", "");
        assert_eq!(Paths::new(&sys).config, PathBuf::from("/x/yo/config.json"));
        assert_eq!(Paths::new(&sys).state, PathBuf::from("/h/.local/state/yo"));
        sys.set_env("YO_CONFIG", "/c.json");
        assert_eq!(Paths::new(&sys).config, PathBuf::from("/c.json"));
    }

    #[test]
    fn tilde() {
        let h = Path::new("/h");
        assert_eq!(expand_home(h, "~"), PathBuf::from("/h"));
        assert_eq!(expand_home(h, "~/a"), PathBuf::from("/h/a"));
        assert_eq!(expand_home(h, "~a/b"), PathBuf::from("~a/b"));
        assert_eq!(expand_home(h, "/a"), PathBuf::from("/a"));
    }
}
