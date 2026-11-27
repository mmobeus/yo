//! `yo setup`'s plans, and running them, under a temporary `HOME`.

use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;
use crate::fake::{FakeSys, TempDir, ok};

struct T {
    _dir: TempDir,
    home: PathBuf,
    sys: FakeSys,
}

impl T {
    fn new() -> T {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let sys = FakeSys::new(&home);
        for tool in ["git", "jq", "gh"] {
            sys.answer_ok(tool, &["--version"], "1");
        }
        T {
            _dir: dir,
            home,
            sys,
        }
    }

    fn paths(&self) -> Paths {
        Paths::new(&self.sys)
    }

    fn plan(&self, opts: &SetupOpts) -> SetupPlan {
        plan(&self.sys, &self.paths(), opts).unwrap()
    }

    fn run(&self, opts: &SetupOpts) -> SetupPlan {
        let p = self.plan(opts);
        execute(&self.sys, &p, &mut |_| {}).unwrap();
        p
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.p(rel)).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.p(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn link(&self, rel: &str, to: &str) {
        let p = self.p(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        symlink(to, p).unwrap();
    }

    /// A yo checkout's `claude/` files, under `rel`.
    fn checkout(&self, rel: &str) -> PathBuf {
        for f in [
            "claude/settings.json",
            "claude/statusline.sh",
            "claude/skills/yo-adopt/SKILL.md",
            "claude/skills/yo-down/SKILL.md",
            "claude/skills/yo-title/SKILL.md",
        ] {
            self.write(&format!("{rel}/{f}"), "x");
        }
        self.p(rel)
    }

    /// iTerm's preferences, as the prefs script prints them.
    fn workgroups(&self, groups: &str) {
        let v = serde_json::json!({ "peer": false, "workgroups": groups });
        self.sys
            .answer_ok("osascript", &["-l", "JavaScript", "-"], &v.to_string());
    }
}

fn copy() -> SetupOpts {
    SetupOpts::default()
}

const DOWN: &str = ".claude/skills/yo-down";
const LAYER: &str = ".config/yo/claude-settings.json";
const STATUS: &str = ".config/yo/statusline.sh";

/// Each step, in short: what it does, and to which path under HOME.
fn said(t: &T, p: &SetupPlan) -> Vec<String> {
    let rel = |p: &Path| {
        p.strip_prefix(&t.home)
            .map_or_else(|_| p.display().to_string(), |r| r.display().to_string())
    };
    p.steps
        .iter()
        .map(|s| match s {
            Step::Keep { path, why } => format!("keep {} {why:?}", rel(path)),
            Step::Backup { path, copy, .. } => format!("backup {} copy={copy}", rel(path)),
            Step::Remove { path, why } => format!(
                "remove {} {}",
                rel(path),
                match why {
                    Removed::Link { .. } => "link",
                    Removed::Forced { .. } => "forced",
                    Removed::Copy => "copy",
                    Removed::Dangling { .. } => "dangling",
                    Removed::OldYo { .. } => "old-yo",
                }
            ),
            Step::Copy(b) => format!("copy {}", rel(&b.dest)),
            Step::Link { path, to } => format!("link {} -> {}", rel(path), rel(to)),
            Step::Write { path, what, .. } => format!(
                "write {} {}",
                rel(path),
                match what {
                    Wrote::Config { .. } => "config",
                    Wrote::Settings => "settings",
                    Wrote::Merged { .. } => "merged",
                    Wrote::Unmerged { .. } => "unmerged",
                    Wrote::Hook { .. } => "hook",
                    Wrote::Unhook { .. } => "unhook",
                }
            ),
            Step::AddPeer { .. } => "add-peer".into(),
            Step::PeerPresent => "peer-present".into(),
            Step::Manifest { contents, .. } => format!("manifest {}", contents.is_some()),
        })
        .collect()
}

#[test]
fn a_fresh_install_copies_everything_and_a_second_run_does_nothing() {
    let t = T::new();
    let p = t.run(&copy());
    assert!(p.notes.is_empty(), "{:?}", p.notes);
    assert_eq!(
        said(&t, &p),
        [
            "copy .config/yo/claude-settings.json",
            "copy .config/yo/statusline.sh",
            "copy .claude/skills/yo-adopt",
            "copy .claude/skills/yo-down",
            "copy .claude/skills/yo-title",
            "write .config/yo/config.json config",
            "manifest true",
        ]
    );
    assert!(t.read(LAYER).contains("statusLine"));
    assert!(t.read(&format!("{DOWN}/SKILL.md")).contains("yo-down"));
    let mode = fs::metadata(t.p(STATUS)).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    // No checkout to name: no repos yet.
    assert_eq!(
        OValue::parse(&t.read(".config/yo/config.json")).unwrap(),
        OValue::parse(r#"{"repos": {}}"#).unwrap()
    );

    let again = t.plan(&copy());
    assert_eq!(
        said(&t, &again),
        [
            "keep .config/yo/claude-settings.json Installed",
            "keep .config/yo/statusline.sh Installed",
            "keep .claude/skills/yo-adopt Installed",
            "keep .claude/skills/yo-down Installed",
            "keep .claude/skills/yo-title Installed",
            "keep .config/yo/config.json Config",
        ]
    );
    assert!(!stale(&t.sys, &t.paths()));
}

#[test]
fn planning_changes_nothing() {
    let t = T::new();
    t.link(".local/bin/yo", "/src/yo/target/release/yo");
    t.workgroups(r#"[{"uniqueIdentifier": "builtin.claudeCode", "sessions": []}]"#);
    let opts = SetupOpts {
        merge_settings: true,
        iterm: true,
        ..copy()
    };
    let p = t.plan(&opts);
    assert!(said(&t, &p).contains(&"add-peer".to_string()));
    let mut left: Vec<_> = walk(&t.home);
    left.sort();
    assert_eq!(
        left,
        [t.p(".local"), t.p(".local/bin"), t.p(".local/bin/yo")]
    );
    // Only questions: tool versions, iTerm's preferences, whether it runs.
    for c in t.sys.calls() {
        assert!(
            ["git", "jq", "gh", "osascript", "pgrep", "date"].contains(&c.program.as_str()),
            "{c:?}"
        );
        assert_ne!(c.program, "defaults");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        out.push(p.clone());
        if fs::symlink_metadata(&p).unwrap().is_dir() {
            out.extend(walk(&p));
        }
    }
    out
}

#[test]
fn an_older_copy_is_replaced_and_an_edited_one_backed_up_first() {
    let t = T::new();
    t.run(&copy());
    // An older yo's copy: on disk as the manifest recorded it.
    t.write(LAYER, "{\"old\": true}\n");
    let m = t.paths().setup_manifest();
    let text = fs::read_to_string(&m).unwrap();
    let mut man: Manifest = serde_json::from_str(&text).unwrap();
    let old = fnv(&[b"{\"old\": true}\n"]);
    man.files.insert(key(&t.p(LAYER)), old);
    fs::write(&m, serde_json::to_string(&man).unwrap()).unwrap();
    assert!(stale(&t.sys, &t.paths()));
    // An edited skill: not what yo wrote.
    t.write(&format!("{DOWN}/SKILL.md"), "mine");

    let p = t.run(&copy());
    let s = said(&t, &p);
    assert_eq!(s[0], "remove .config/yo/claude-settings.json copy");
    assert_eq!(s[1], "copy .config/yo/claude-settings.json");
    assert!(s.contains(&"backup .claude/skills/yo-down copy=false".to_string()));
    assert!(t.read(LAYER).contains("statusLine"));
    let bak = t.p(".claude/skills").read_dir().unwrap().find_map(|e| {
        let p = e.unwrap().path();
        p.to_string_lossy().contains("yo-down.bak-").then_some(p)
    });
    assert_eq!(
        fs::read_to_string(bak.unwrap().join("SKILL.md")).unwrap(),
        "mine"
    );
    assert!(!stale(&t.sys, &t.paths()));
}

#[test]
fn links_from_an_older_install_are_replaced_and_others_need_force() {
    let t = T::new();
    t.link(DOWN, "/src/yo/claude/skills/yo-down");
    t.link(LAYER, "/elsewhere/settings.json");
    let p = t.run(&copy());
    let s = said(&t, &p);
    assert!(s.contains(&format!("remove {DOWN} link")));
    assert!(s.contains(&format!("copy {DOWN}")));
    assert!(
        s.iter()
            .any(|l| l.starts_with(&format!("keep {LAYER} Foreign")))
    );
    assert!(fs::symlink_metadata(t.p(DOWN)).unwrap().is_dir());
    assert!(fs::symlink_metadata(t.p(LAYER)).unwrap().is_symlink());
    // Left alone, so it's no reason to say setup is out of date.
    assert!(!stale(&t.sys, &t.paths()));

    let p = t.run(&SetupOpts {
        force: true,
        ..copy()
    });
    let s = said(&t, &p);
    assert_eq!(s[0], format!("remove {LAYER} forced"));
    assert!(fs::symlink_metadata(t.p(LAYER)).unwrap().is_file());
}

#[test]
fn link_mode_links_into_the_checkout_and_names_it_in_the_config() {
    let t = T::new();
    let c = t.checkout("src/yo");
    let opts = SetupOpts {
        link: Some(c.clone()),
        ..copy()
    };
    let p = t.run(&opts);
    let s = said(&t, &p);
    assert_eq!(s[0], format!("link {LAYER} -> src/yo/claude/settings.json"));
    assert_eq!(s[3], format!("link {DOWN} -> src/yo/claude/skills/yo-down"));
    assert_eq!(
        fs::read_link(t.p(DOWN)).unwrap(),
        c.join("claude/skills/yo-down")
    );
    let cfg = OValue::parse(&t.read(".config/yo/config.json")).unwrap();
    let path = cfg
        .get("repos")
        .and_then(|r| r.get("yo"))
        .and_then(|y| y.get("path"));
    assert_eq!(
        path,
        Some(&OValue::String(c.to_string_lossy().into_owned()))
    );
    assert!(
        said(&t, &t.plan(&opts))
            .iter()
            .all(|l| l.starts_with("keep"))
    );

    // Back to copies: the links are yo's, so they're replaced.
    let p = t.run(&copy());
    assert_eq!(said(&t, &p)[0], format!("remove {LAYER} link"));
    assert!(fs::symlink_metadata(t.p(LAYER)).unwrap().is_file());
}

#[test]
fn link_mode_needs_a_yo_checkout() {
    let t = T::new();
    t.write("src/other/README", "x");
    let err = plan(
        &t.sys,
        &t.paths(),
        &SetupOpts {
            link: Some(t.p("src/other")),
            ..copy()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("isn't a yo checkout"), "{err}");
    assert!(
        plan(
            &t.sys,
            &t.paths(),
            &SetupOpts {
                link: Some(t.p("nowhere")),
                ..copy()
            }
        )
        .is_err()
    );
}

#[test]
fn uninstall_takes_out_only_what_yo_put_in() {
    let t = T::new();
    t.run(&copy());
    t.write(&format!("{DOWN}/SKILL.md"), "mine");
    fs::remove_dir_all(t.p(".claude/skills/yo-title")).unwrap();
    t.link(".claude/skills/yo-title", "/src/yo/claude/skills/yo-title");
    let opts = SetupOpts {
        uninstall: true,
        ..copy()
    };
    let p = t.run(&opts);
    assert_eq!(
        said(&t, &p),
        [
            "remove .config/yo/claude-settings.json copy",
            "remove .config/yo/statusline.sh copy",
            "remove .claude/skills/yo-adopt copy",
            "keep .claude/skills/yo-down Edited",
            "remove .claude/skills/yo-title link",
            "keep .config/yo/config.json Config",
            "manifest false",
        ]
    );
    assert!(!t.p(LAYER).exists());
    assert!(t.p(DOWN).exists());
    assert!(t.p(".config/yo/config.json").exists());
    assert!(!t.paths().setup_manifest().exists());
    assert!(!stale(&t.sys, &t.paths()));
}

#[test]
fn a_dangling_config_link_is_replaced() {
    let t = T::new();
    t.link(".config/yo/config.json", "/gone/config.json");
    let s = said(&t, &t.run(&copy()));
    assert!(s.contains(&"remove .config/yo/config.json dangling".to_string()));
    assert!(
        fs::symlink_metadata(t.p(".config/yo/config.json"))
            .unwrap()
            .is_file()
    );
}

#[test]
fn merge_settings_keeps_the_rest_and_takes_only_its_keys_back() {
    let t = T::new();
    let settings = ".claude/settings.json";
    t.write(
        settings,
        r#"{"model": "opus", "statusLine": {"type": "x", "keep": 1}}"#,
    );
    let merge = SetupOpts {
        merge_settings: true,
        ..copy()
    };
    let s = said(&t, &t.run(&merge));
    assert!(s.contains(&format!("backup {settings} copy=true")));
    assert!(s.contains(&format!("write {settings} merged")));
    let v = OValue::parse(&t.read(settings)).unwrap();
    assert_eq!(v.get("model"), Some(&OValue::String("opus".into())));
    let sl = v.get("statusLine").unwrap();
    assert_eq!(sl.get("type"), Some(&OValue::String("command".into())));
    assert!(sl.get("keep").is_some(), "objects merge key by key");
    assert!(said(&t, &t.plan(&merge)).contains(&format!("keep {settings} Merged")));

    let s = said(
        &t,
        &t.run(&SetupOpts {
            uninstall: true,
            ..merge.clone()
        }),
    );
    assert!(s.contains(&format!("write {settings} unmerged")));
    let v = OValue::parse(&t.read(settings)).unwrap();
    assert!(v.get("statusLine").is_none());
    assert!(v.get("model").is_some());

    t.write(settings, "not json");
    assert!(plan(&t.sys, &t.paths(), &merge).is_err());
}

#[test]
fn merge_settings_creates_the_file_when_there_is_none() {
    let t = T::new();
    let s = said(
        &t,
        &t.run(&SetupOpts {
            merge_settings: true,
            ..copy()
        }),
    );
    assert!(s.contains(&"write .claude/settings.json settings".to_string()));
    assert!(t.read(".claude/settings.json").contains("statusLine"));
}

#[test]
fn the_iterm_peer_is_added_once() {
    let t = T::new();
    t.sys.push_ids(["abcd-1234"]);
    let old = r#"[{"uniqueIdentifier":"other"},{"uniqueIdentifier":"builtin.claudeCode","sessions":[{"kind":{"kind":"chat"}}]}]"#;
    t.workgroups(old);
    t.sys.answer("pgrep", &["-axq", "iTerm2"], ok(""));
    let iterm = SetupOpts {
        iterm: true,
        ..copy()
    };
    let p = t.plan(&iterm);
    let Some(Step::AddPeer {
        new,
        backup,
        running,
        ..
    }) = p.steps.iter().find(|s| matches!(s, Step::AddPeer { .. }))
    else {
        panic!("{:?}", p.steps)
    };
    assert!(running);
    let groups = OValue::parse(new).unwrap();
    let OValue::Array(gs) = &groups else { panic!() };
    let peer = gs[1].get("sessions").and_then(OValue::as_array).unwrap()[1].clone();
    assert_eq!(peer.get("displayName"), Some(&OValue::String("CLI".into())));
    assert_eq!(
        peer.get("uniqueIdentifier"),
        Some(&OValue::String("ABCD-1234".into()))
    );
    assert_eq!(
        gs[0],
        OValue::parse(r#"{"uniqueIdentifier":"other"}"#).unwrap()
    );

    let hex: String = new.bytes().map(|b| format!("{b:02x}")).collect();
    let args = [
        "write",
        "com.googlecode.iterm2",
        "Workgroups",
        "-data",
        hex.as_str(),
    ];
    t.sys.answer("defaults", &args, ok(""));
    execute(&t.sys, &p, &mut |_| {}).unwrap();
    assert_eq!(fs::read_to_string(backup).unwrap(), old);
    assert!(
        t.sys
            .calls()
            .iter()
            .any(|c| c.program == "defaults" && c.args == args)
    );

    t.workgroups(&groups.to_compact());
    assert!(said(&t, &t.plan(&iterm)).contains(&"peer-present".to_string()));
}

#[test]
fn no_workgroup_is_a_warning_not_a_failure() {
    let t = T::new();
    t.workgroups(r#"[{"uniqueIdentifier":"other"}]"#);
    let p = t.plan(&SetupOpts {
        iterm: true,
        ..copy()
    });
    assert!(matches!(&p.notes[..], [Note::Warn(w)] if w.contains("workgroup")));
    assert!(!said(&t, &p).iter().any(|s| s.contains("peer")));
}

#[test]
fn missing_tools_are_warned_about() {
    let t = T::new();
    t.sys.answer("jq", &["--version"], crate::fake::failed(127));
    let p = t.plan(&copy());
    assert!(matches!(&p.notes[..], [Note::Warn(w)] if w.starts_with("jq not found")));
}

#[test]
fn an_old_link_to_a_checkouts_yo_goes_unless_it_is_running() {
    let t = T::new();
    t.link(".local/bin/yo", "/src/yo/target/release/yo");
    let s = said(&t, &t.run(&copy()));
    assert!(s.contains(&"remove .local/bin/yo old-yo".to_string()));
    assert!(fs::symlink_metadata(t.p(".local/bin/yo")).is_err());

    // Someone else's yo there is left alone.
    t.link(".local/bin/yo", "/opt/other/yo");
    assert!(
        !said(&t, &t.plan(&copy()))
            .iter()
            .any(|l| l.contains(".local/bin"))
    );
}

#[test]
fn stale_needs_a_manifest_and_ignores_links() {
    let t = T::new();
    assert!(!stale(&t.sys, &t.paths()));
    let c = t.checkout("src/yo");
    t.run(&SetupOpts {
        link: Some(c),
        ..copy()
    });
    assert!(!stale(&t.sys, &t.paths()));
    // A file this yo has that the last run didn't install.
    let m = t.paths().setup_manifest();
    let mut man: Manifest = serde_json::from_str(&fs::read_to_string(&m).unwrap()).unwrap();
    man.files.remove(&key(&t.p(DOWN)));
    fs::write(&m, serde_json::to_string(&man).unwrap()).unwrap();
    assert!(stale(&t.sys, &t.paths()));
}

const HOOKS: &str = ".codex/hooks.json";

/// yo's hook in `~/.codex/hooks.json` is added only when asked, kept
/// current, needs approving until Codex has, and comes out on uninstall
/// with the person's own hooks kept.
#[test]
fn codex_hook_only_when_asked() {
    let t = T::new();
    t.sys
        .answer_ok("codex", &["--version"], "codex-cli 0.172.0");
    assert_eq!(
        hook_offers(&t.sys)
            .iter()
            .map(|o| o.harness.as_str())
            .collect::<Vec<_>>(),
        ["codex"]
    );
    let p = t.run(&copy());
    assert!(
        !said(&t, &p).iter().any(|s| s.contains(HOOKS)),
        "not asked: not added"
    );
    assert!(!t.p(HOOKS).exists());

    let theirs = r#"{"hooks":{"SessionEnd":[{"hooks":[{"type":"command","command":"mine"}]}]}}"#;
    t.write(HOOKS, theirs);
    let hooked = SetupOpts {
        hooks: true,
        ..copy()
    };
    let p = t.run(&hooked);
    let s = said(&t, &p);
    assert!(s.contains(&format!("backup {HOOKS} copy=true")), "{s:?}");
    assert!(s.contains(&format!("write {HOOKS} hook")), "{s:?}");
    assert_eq!(p.approve.len(), 1);
    assert_eq!(p.approve[0].command, "codex");
    let cmd = format!("'{}' _session-hook codex", t.sys.exe().display());
    let v = OValue::parse(&t.read(HOOKS)).unwrap();
    let groups = v.get("hooks").and_then(|h| h.get("SessionEnd")).unwrap();
    assert_eq!(
        groups.to_compact(),
        format!(
            r#"[{{"hooks":[{{"type":"command","command":"mine"}}]}},{{"hooks":[{{"type":"command","command":{}}}]}}]"#,
            serde_json::to_string(&cmd).unwrap()
        )
    );
    assert!(hook_offers(&t.sys).is_empty(), "in place: nothing to offer");
    assert!(!hook_approved(&t.sys, "codex"));

    // Once Codex has approved it (under its place in the file), a run
    // without --hooks keeps it and asks nothing.
    t.write(
        ".codex/config.toml",
        &format!(
            "[hooks.state.\"{}:session_end:1:0\"]\ntrusted_hash = \"sha256:x\"\n",
            t.p(HOOKS).display()
        ),
    );
    assert!(hook_approved(&t.sys, "codex"));
    let p = t.run(&copy());
    assert!(said(&t, &p).contains(&format!("keep {HOOKS} Hooked")));
    assert!(p.approve.is_empty());

    let p = t.run(&SetupOpts {
        uninstall: true,
        ..copy()
    });
    assert!(said(&t, &p).contains(&format!("write {HOOKS} unhook")));
    assert_eq!(
        OValue::parse(&t.read(HOOKS)).unwrap(),
        OValue::parse(theirs).unwrap()
    );
}

#[test]
fn codex_hook_isnt_offered_without_codex() {
    let t = T::new();
    t.sys
        .answer("codex", &["--version"], crate::fake::failed(127));
    assert!(hook_offers(&t.sys).is_empty());
}

#[test]
fn a_misshapen_codex_hooks_file_is_warned_about_and_left_alone() {
    let t = T::new();
    t.write(HOOKS, r#"{"hooks":[]}"#);
    let p = t.plan(&SetupOpts {
        hooks: true,
        ..copy()
    });
    assert!(p.notes.iter().any(|n| matches!(n,
        Note::Warn(s) if s.ends_with("isn't shaped like a hooks file, so yo's hook wasn't added; fix or move it first"))));
    assert!(
        !said(&t, &p).iter().any(|s| s.contains(HOOKS)),
        "{:?}",
        said(&t, &p)
    );
    assert!(p.approve.is_empty());
}
