//! yo's gates. `cargo xtask ci` runs every default gate, in the order CI
//! runs them; each gate also has its own subcommand. CI only calls these.
//!
//! The default gates need no secrets and nothing from the network beyond
//! fetching crates. Anything else is its own subcommand. Configuration comes
//! from the environment (and a gitignored `.env`, if present); see
//! `docs/dev/env.md`.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::OnceLock;

/// Variables from `.env`, passed to every command a gate runs.
static DOTENV: OnceLock<Vec<(String, String)>> = OnceLock::new();

const USAGE: &str = "\
usage: cargo xtask <gate>

default gates (cargo xtask ci runs these, in this order):
  lint        cargo fmt --check, cargo clippy -D warnings
  arch        architecture rules over cargo metadata
  deny        cargo-deny: licenses, bans (duplicates, allowed crates), sources
  test        unit and integration tests (cargo nextest), doctests

other gates (need the network):
  advisories  cargo-deny: the RustSec advisory database
";

type Gate = fn(&Path) -> Result<(), String>;

const DEFAULT_GATES: &[(&str, Gate)] = &[
    ("lint", lint),
    ("arch", arch),
    ("deny", deny),
    ("test", test),
];

const OTHER_GATES: &[(&str, Gate)] = &[("advisories", advisories)];

#[allow(clippy::print_stdout, clippy::print_stderr)]
fn main() -> ExitCode {
    let root = workspace_root();
    match load_dotenv(&root.join(".env")) {
        Ok(vars) => DOTENV.get_or_init(|| vars),
        Err(e) => {
            eprintln!("xtask: .env: {e}");
            return ExitCode::FAILURE;
        }
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let gates: Vec<(&str, Gate)> = match args.as_slice() {
        [] => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [cmd] if cmd == "help" || cmd == "--help" || cmd == "-h" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [cmd] if cmd == "ci" => DEFAULT_GATES.to_vec(),
        [cmd] => match DEFAULT_GATES
            .iter()
            .chain(OTHER_GATES)
            .find(|(name, _)| name == cmd)
        {
            Some(gate) => vec![*gate],
            None => {
                eprint!("xtask: unknown gate {cmd:?}\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprint!("xtask: expected one gate\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    for (name, gate) in gates {
        eprintln!("==> {name}");
        if let Err(e) = gate(&root) {
            eprintln!("xtask: {name} failed: {e}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn lint(root: &Path) -> Result<(), String> {
    run(root, "cargo", &["fmt", "--all", "--check"])?;
    run(
        root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )
}

fn test(root: &Path) -> Result<(), String> {
    need_tool(root, "nextest", "cargo install --locked cargo-nextest")?;
    run(
        root,
        "cargo",
        &[
            "nextest",
            "run",
            "--workspace",
            "--locked",
            "--no-tests=warn",
        ],
    )?;
    run(root, "cargo", &["test", "--workspace", "--locked", "--doc"])
}

fn deny(root: &Path) -> Result<(), String> {
    need_tool(root, "deny", "cargo install --locked cargo-deny")?;
    run(
        root,
        "cargo",
        &["deny", "--locked", "check", "licenses", "bans", "sources"],
    )
}

fn advisories(root: &Path) -> Result<(), String> {
    need_tool(root, "deny", "cargo install --locked cargo-deny")?;
    run(root, "cargo", &["deny", "--locked", "check", "advisories"])
}

/// Crates that must never be in a crate's (non-dev) dependency tree.
/// See "Architecture rules" in docs/design/core.md.
const FORBIDDEN: &[(&str, &[&str], &str)] = &[
    (
        "yo-core",
        &["tokio", "axum", "agent-client-protocol", "ratatui", "tauri"],
        "yo-core stays synchronous and light",
    ),
    (
        "yo-app",
        &["yo-core", "yo-local", "yo-daemon"],
        "the app holds no yo logic; it uses only yo-client and yo-proto",
    ),
];

fn arch(root: &Path) -> Result<(), String> {
    let out = command(root, "cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;

    let names: BTreeMap<&str, &str> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    // Package id -> the ids it depends on, other than as a dev-dependency.
    let deps: BTreeMap<&str, Vec<&str>> = meta["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let id = node["id"].as_str()?;
            let normal = node["deps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| {
                    d["dep_kinds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|k| k["kind"].as_str() != Some("dev"))
                })
                .filter_map(|d| d["pkg"].as_str())
                .collect();
            Some((id, normal))
        })
        .collect();

    let mut errors = Vec::new();
    for (krate, forbidden, rule) in FORBIDDEN {
        let Some(start) = names.iter().find(|(_, n)| *n == krate).map(|(id, _)| *id) else {
            errors.push(format!("{krate}: not in the workspace"));
            continue;
        };
        // Breadth-first, remembering how each crate was reached.
        let mut via: BTreeMap<&str, &str> = BTreeMap::new();
        let mut queue = VecDeque::from([start]);
        while let Some(id) = queue.pop_front() {
            for &dep in deps.get(id).into_iter().flatten() {
                if dep != start && !via.contains_key(dep) {
                    via.insert(dep, id);
                    queue.push_back(dep);
                }
            }
        }
        for &id in via.keys() {
            let name = names.get(id).copied().unwrap_or(id);
            if forbidden.contains(&name) {
                let mut path = vec![name];
                let mut at = id;
                while let Some(&from) = via.get(at) {
                    path.push(names.get(from).copied().unwrap_or(from));
                    at = from;
                }
                path.push(krate);
                path.dedup();
                path.reverse();
                errors.push(format!("{} ({rule})", path.join(" -> ")));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "forbidden dependencies:\n  {}",
            errors.join("\n  ")
        ))
    }
}

#[allow(clippy::print_stderr)]
fn run(root: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    eprintln!("$ {program} {}", args.join(" "));
    let status = command(root, program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {}: {status}", args.join(" ")))
    }
}

fn need_tool(root: &Path, sub: &str, install: &str) -> Result<(), String> {
    let found = command(root, "cargo")
        .args([sub, "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if found {
        Ok(())
    } else {
        Err(format!(
            "cargo-{sub} is not installed; install it with: {install}"
        ))
    }
}

/// A command run from the workspace root, with `.env`'s variables.
fn command(root: &Path, program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(root)
        .envs(DOTENV.get().into_iter().flatten().cloned());
    cmd
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}

/// Reads `KEY=VALUE` lines. Blank lines and `#` comments are skipped, and a
/// variable already set in the environment wins over the file.
fn load_dotenv(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut vars = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected KEY=VALUE", n + 1));
        };
        let key = key.trim();
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        if std::env::var_os(key).is_none() {
            vars.push((key.to_string(), value.to_string()));
        }
    }
    Ok(vars)
}
