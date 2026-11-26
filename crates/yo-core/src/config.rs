//! The config file: `config.json`, as bash reads it. Unknown keys are
//! errors. Maps keep the file's order, because order matters: the first repo
//! registered is the default one.
//!
//! These are the file's shapes only. [`crate::registry`] resolves them.

use std::fmt;
use std::fs;
use std::marker::PhantomData;
use std::path::Path;

use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::error::{Error, Result};
use crate::ojson::OValue;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub defaults: DefaultsCfg,
    #[serde(default)]
    pub profiles: Ordered<ProfileCfg>,
    #[serde(default)]
    pub repos: Ordered<RepoCfg>,
    #[serde(default, rename = "reposIn")]
    pub repos_in: Vec<Text>,
    #[serde(default)]
    pub agents: Ordered<AgentCfg>,
}

/// `defaults`: global settings. Each has an environment variable that, when
/// set, beats the file (see [`crate::registry::Defaults`]).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DefaultsCfg {
    pub branch_prefix: Option<Text>,
    pub mode: Option<Text>,
    pub claude: Option<Text>,
    pub memories: Option<Text>,
    pub memory: Option<Text>,
    pub settings: Option<Text>,
    pub layout: Option<Text>,
    pub window_mode: Option<Text>,
    pub window_group: Option<Text>,
    pub win_width: Option<Text>,
    pub win_height: Option<Text>,
    pub iterm_profile: Option<Text>,
    pub review_cmd: Option<Text>,
    pub default_repo: Option<Text>,
    pub clone_root: Option<Text>,
    pub clone_protocol: Option<Text>,
    pub guest_profile: Option<Text>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProfileCfg {
    pub mode: Option<Text>,
    pub memory: Option<Text>,
    pub branch_prefix: Option<Text>,
    pub claude: Option<Text>,
    pub settings: Option<Text>,
    pub dir: Option<Text>,
    pub window_group: Option<Text>,
    /// GitHub owners: every clone at `<cloneRoot>/<owner>/*` joins.
    pub owners: Option<Words>,
    /// Only `false` turns it off; anything else leaves worktrees trusted.
    pub trust: Option<serde_json::Value>,
    #[serde(default)]
    pub repos: Ordered<RepoCfg>,
    #[serde(default)]
    pub repos_in: Vec<Text>,
    #[serde(default)]
    pub agents: Ordered<AgentCfg>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RepoCfg {
    pub path: Text,
    pub base: Option<Text>,
    pub github: Option<Text>,
    pub mcp: Option<Words>,
    pub mirror: Option<Words>,
    pub title_prefix: Option<Text>,
    pub memory: Option<Text>,
    pub branch_prefix: Option<Text>,
    pub mode: Option<Text>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCfg {
    pub repo: Text,
    pub title: Option<Text>,
}

impl Config {
    /// The config at `path`, or `None` when there's no file there.
    pub fn load(path: &Path) -> Result<Option<Config>> {
        if !path.is_file() {
            return Ok(None);
        }
        let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Error::Config {
                path: path.to_path_buf(),
                msg: e.to_string(),
            })
    }
}

/// Writing the config: each change keeps the file's order and everything
/// else in it, and is checked to still load before it's written.
impl Config {
    /// Registers repo `key` at `repo_path` (under `profile`, if given), with
    /// its GitHub `owner/repo`. A path under `home` is written as `~/...`.
    pub fn add_repo(
        path: &Path,
        home: &Path,
        profile: Option<&str>,
        key: &str,
        repo_path: &Path,
        github: Option<&str>,
    ) -> Result<()> {
        let bad = |m: String| Error::Config {
            path: path.to_path_buf(),
            msg: m,
        };
        for k in std::iter::once(key).chain(profile) {
            if !crate::registry::valid_key(k) {
                return Err(bad(format!(
                    "'{k}': names may only contain letters, digits, . _ -"
                )));
            }
        }
        if let Some(g) = github
            && crate::github::owner_repo_from_url(&format!("https://github.com/{g}")).as_deref()
                != Some(g)
        {
            return Err(bad(format!("'{g}' isn't a GitHub owner/repo")));
        }
        let mut d = read_ordered(path)?;
        let shape = || bad("not the shape of a yo config".into());
        let target = match profile {
            Some(p) => d
                .entry_object("profiles")
                .and_then(|ps| ps.entry_object(p))
                .and_then(|p| p.entry_object("repos")),
            None => d.entry_object("repos"),
        }
        .ok_or_else(shape)?;
        let mut entry = vec![("path".to_string(), OValue::String(tilde(home, repo_path)))];
        if let Some(g) = github {
            entry.push(("github".into(), OValue::String(g.to_string())));
        }
        target.insert(key, OValue::Object(entry));
        write_checked(path, &d)
    }

    /// Adds profile `name`, claiming GitHub owner `owner`.
    pub fn add_profile(path: &Path, name: &str, owner: &str) -> Result<()> {
        let bad = |m: String| Error::Config {
            path: path.to_path_buf(),
            msg: m,
        };
        if !crate::registry::valid_key(name) || !crate::registry::valid_key(owner) {
            return Err(bad(format!(
                "'{name}': names may only contain letters, digits, . _ -"
            )));
        }
        let mut d = read_ordered(path)?;
        let profiles = d
            .entry_object("profiles")
            .ok_or_else(|| bad("not the shape of a yo config".into()))?;
        if profiles.get(name).is_some() {
            return Err(bad(format!("profile '{name}' already exists")));
        }
        profiles.insert(
            name,
            OValue::Object(vec![(
                "owners".into(),
                OValue::Array(vec![OValue::String(owner.to_string())]),
            )]),
        );
        write_checked(path, &d)
    }
}

fn read_ordered(path: &Path) -> Result<OValue> {
    match fs::read_to_string(path) {
        Ok(t) => OValue::parse(&t).map_err(|e| Error::Config {
            path: path.to_path_buf(),
            msg: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(OValue::object()),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Writes `d` only if it still loads as a config.
fn write_checked(path: &Path, d: &OValue) -> Result<()> {
    let text = d.to_pretty();
    serde_json::from_str::<Config>(&text).map_err(|e| Error::Config {
        path: path.to_path_buf(),
        msg: format!("the change wouldn't load ({e}); not written"),
    })?;
    crate::write_atomic(path, text.as_bytes())
}

/// `~/rest` for a path under `home`.
fn tilde(home: &Path, p: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() && home != Path::new("/") => {
            if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            }
        }
        _ => p.display().to_string(),
    }
}

/// A scalar setting: a string, or a number or boolean taken as its text (as
/// bash takes it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text(pub String);

impl Text {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => Ok(Text(s)),
            serde_json::Value::Number(n) => Ok(Text(n.to_string())),
            serde_json::Value::Bool(b) => Ok(Text(if b { "True" } else { "False" }.into())),
            other => Err(de::Error::custom(format!("expected a string, got {other}"))),
        }
    }
}

/// A list of words, written as a JSON list or one space-separated string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Words(pub Vec<String>);

impl<'de> Deserialize<'de> for Words {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            List(Vec<Text>),
            One(Text),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::List(v) => Words(v.into_iter().map(|t| t.0).collect()),
            Raw::One(t) => Words(t.0.split_whitespace().map(str::to_string).collect()),
        })
    }
}

/// A JSON object that keeps its keys in the file's order. A key given twice
/// keeps its first place and its last value, as Python's `json` does.
#[derive(Debug)]
pub struct Ordered<T>(pub Vec<(String, T)>);

impl<T> Default for Ordered<T> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<T> Ordered<T> {
    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Self::Value, A::Error> {
                let mut out: Vec<(String, T)> = Vec::new();
                while let Some((k, v)) = m.next_entry::<String, T>()? {
                    match out.iter_mut().find(|(e, _)| *e == k) {
                        Some(slot) => slot.1 = v,
                        None => out.push((k, v)),
                    }
                }
                Ok(Ordered(out))
            }
            // Not an object: say so rather than accept a list.
            fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
                Err(de::Error::custom("expected an object"))
            }
        }
        d.deserialize_map(V(PhantomData))
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use crate::fake::TempDir;

    #[test]
    fn add_repo_and_profile_keep_the_file() {
        let t = TempDir::new().unwrap();
        let f = t.path().join("c.json");
        let h = Path::new("/h");
        fs::write(&f, r#"{"repos": {"z": {"path": "/z"}, "a": {"path": "/a"}}, "defaults": {"mode": "here"}}"#).unwrap();
        Config::add_repo(&f, h, None, "m", Path::new("/h/src/m"), Some("o/m")).unwrap();
        Config::add_profile(&f, "o", "o").unwrap();
        Config::add_repo(&f, h, Some("o"), "n", Path::new("/x/n"), None).unwrap();
        assert_eq!(
            fs::read_to_string(&f).unwrap(),
            r#"{
  "repos": {
    "z": {
      "path": "/z"
    },
    "a": {
      "path": "/a"
    },
    "m": {
      "path": "~/src/m",
      "github": "o/m"
    }
  },
  "defaults": {
    "mode": "here"
  },
  "profiles": {
    "o": {
      "owners": [
        "o"
      ],
      "repos": {
        "n": {
          "path": "/x/n"
        }
      }
    }
  }
}
"#
        );
        assert!(Config::add_profile(&f, "o", "o").is_err());
        // Bad keys, and a change that wouldn't load, are refused and not written.
        let before = fs::read_to_string(&f).unwrap();
        assert!(Config::add_repo(&f, h, None, "..", Path::new("/x"), None).is_err());
        assert!(Config::add_repo(&f, h, Some("a b"), "k", Path::new("/x"), None).is_err());
        assert!(Config::add_repo(&f, h, None, "k", Path::new("/x"), Some("a/b/c")).is_err());
        fs::write(&f, r#"{"repos": []}"#).unwrap();
        assert!(Config::add_repo(&f, h, None, "k", Path::new("/x"), None).is_err());
        fs::write(&f, &before).unwrap();
        // No file yet: one is made.
        let g = t.path().join("new/c.json");
        Config::add_repo(&g, h, None, "k", Path::new("/k"), None).unwrap();
        assert!(Config::load(&g).unwrap().is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> std::result::Result<Config, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    #[test]
    fn empty_and_missing() {
        let c = parse("{}").unwrap();
        assert!(c.repos.0.is_empty());
        assert_eq!(
            Config::load(Path::new("/no/such/config.json"))
                .unwrap()
                .map(|_| ()),
            None
        );
    }

    #[test]
    fn unknown_keys_are_errors_at_every_level() {
        for bad in [
            r#"{"repo": {}}"#,
            r#"{"defaults": {"modes": "here"}}"#,
            r#"{"repos": {"a": {"path": "/a", "bse": "main"}}}"#,
            r#"{"profiles": {"p": {"colour": "red"}}}"#,
            r#"{"agents": {"x": {"repo": "a", "name": "y"}}}"#,
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains("unknown field"), "{bad}: {e}");
        }
    }

    #[test]
    fn required_keys() {
        assert!(
            parse(r#"{"repos": {"a": {}}}"#)
                .unwrap_err()
                .contains("path")
        );
        assert!(
            parse(r#"{"agents": {"x": {}}}"#)
                .unwrap_err()
                .contains("repo")
        );
    }

    #[test]
    fn order_is_kept() {
        let c = parse(
            r#"{"repos": {"zed": {"path": "/z"}, "abc": {"path": "/a"}, "zed": {"path": "/z2"}}}"#,
        )
        .unwrap();
        let keys: Vec<&str> = c.repos.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["zed", "abc"]);
        assert_eq!(c.repos.0[0].1.path.as_str(), "/z2");
    }

    #[test]
    fn load_a_file() {
        let t = crate::fake::TempDir::new().unwrap();
        let good = t.path().join("good.json");
        std::fs::write(&good, r#"{"repos": {"a": {"path": "/a"}}}"#).unwrap();
        assert_eq!(Config::load(&good).unwrap().unwrap().repos.0.len(), 1);
        for (name, text, want) in [
            ("bad.json", "{", "EOF"),
            ("unknown.json", r#"{"repo": {}}"#, "unknown field `repo`"),
        ] {
            let p = t.path().join(name);
            std::fs::write(&p, text).unwrap();
            let e = Config::load(&p).unwrap_err().to_string();
            assert!(e.starts_with(&format!("{}: ", p.display())), "{e}");
            assert!(e.contains(want), "{e}");
        }
    }

    #[test]
    fn booleans_as_bash_takes_them_and_nulls_refused() {
        let c = parse(r#"{"defaults": {"mode": true, "claude": false}}"#).unwrap();
        assert_eq!(c.defaults.mode, Some(Text("True".into())));
        assert_eq!(c.defaults.claude, Some(Text("False".into())));
        assert!(parse(r#"{"repos": {"a": {"path": null}}}"#).is_err());
        assert!(parse(r#"{"repos": {"a": {"path": ["/a"]}}}"#).is_err());
    }

    #[test]
    fn scalars_and_words() {
        let c = parse(
            r#"{"defaults": {"winWidth": 1200},
                "repos": {"a": {"path": "/a", "mcp": ["x", "y"], "mirror": ".env .envrc"}}}"#,
        )
        .unwrap();
        assert_eq!(c.defaults.win_width, Some(Text("1200".into())));
        let r = &c.repos.0[0].1;
        assert_eq!(r.mcp, Some(Words(vec!["x".into(), "y".into()])));
        assert_eq!(r.mirror, Some(Words(vec![".env".into(), ".envrc".into()])));
    }
}
