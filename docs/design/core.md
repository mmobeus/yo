# yo core: design

## Status

> **Phase 1 is done:** `yo` is the Rust binary (`yo-cli` over `yo-core`),
> with one harness (Claude) and one runner (iTerm). The bash script it
> replaced, `scripts/yo`, is gone; git history has it. The
> [guide](../guide.md) and [reference](../reference.md) describe the Rust
> `yo`. Nothing from phase 2 on, including harnesses other than Claude, ACP
> sessions, the relay, the daemon, `yo status`, `yo tui` and `yo app`,
> exists until its phase lands.

| phase | what | status |
| --- | --- | --- |
| 0 | this design | done (#8, PR #10) |
| before 1 | the gates: CI, lints, review skills, skill tuning | done (#11, PRs #13, #14, #15, #16) |
| 1 | Rust core with CLI parity | done (#18, #19, #20, #21; PRs #22 to #32, and the switch, PR #33): the parity harness taken out of the plan; `Sys`, config, the registry and the ledger; `yo-cli` on clap; every command ported, area by area; one review round over the whole phase (#21); `install.sh` builds the binary, and `scripts/yo` is retired |
| after 1 | distribution: Homebrew, and `yo setup` replacing `install.sh` | done (#34; PRs #36, #37, and mmobeus/homebrew-tap#1): `yo setup` installs the `claude/` files from copies built into yo (or links them, `--link`), and `install.sh` is retired; a `v*` tag releases both macOS binaries as the tag's version and dispatches to the tap (`cargo xtask release`; from #43 the tag is the version, with `release/X.Y` hotfix branches on demand); `brew install mmobeus/tap/yo` installs them, from v0.1.0 |
| 2 | a second harness: Codex | in progress (#39): the `harness` setting and `--harness`, `harnesses.<name>` in the registry, and what phase 1 left outside `harness/claude.rs` (agents' `.claude/agents`, `CLAUDE_CODE_SESSION_ID`, `CLAUDECODE`, the trust steps' wording) behind `Harness` methods; the Codex harness next |
| 3 | status and events | not started |
| 4 | `yo tui` | not started |
| 5 | the daemon, ACP runner and relay | not started |
| 6 | `yo app` | not started |
| 7 | phone, hub and remote workers | not started |

Each phase updates this table as it lands, and corrects anything below that
turned out different in practice. The [phases](#phases) section says what
each one includes; #9 tracks them.

## About this doc

Phase 0 of moving yo from a bash script to a core with a CLI, a TUI, an app and
pluggable runners. This doc settles the language, sketches the core and its API,
says how yo stays open to coding agents other than Claude Code, and says how
the port is tested. No code comes with it; phase 1 builds against it.

- [Status](#status)
- [Decisions](#decisions)
- [Principles](#principles)
- [Language: Rust](#language-rust)
- [The app: Tauri](#the-app-tauri)
- [Shape](#shape)
- [Crates](#crates)
- [The core's API](#the-cores-api)
- [Harnesses](#harnesses)
- [Runners](#runners)
- [Messages](#messages)
- [Machine ids](#machine-ids)
- [Events and status](#events-and-status)
- [The daemon](#the-daemon)
- [Phases](#phases)
- [Testing the port](#testing-the-port)
- [How it's built](#how-its-built)
- [Open questions](#open-questions)

## Decisions

| question | decision |
| --- | --- |
| language | Rust, for the core, CLI, daemon, TUI and the app's native side |
| coding agents | more than Claude Code. yo calls them **harnesses** (`claude`, `codex`, ...), because "agent" already means a persona like `--agent=cindy`. Claude is the first; Codex is the second, right after the port |
| background sessions | run over [ACP](https://agentclientprotocol.com) (the Agent Client Protocol), which works for many harnesses, not by driving one harness's own stream format |
| messages | by typing into the tab, by ACP, or through a relay session that uses Claude's `SendMessage`; see [messages](#messages) |
| app | a Tauri app (`yo app`): one web UI, wrapped by Tauri, talking to the daemon over its Unix socket. The app holds no yo logic. The same UI can be served over the daemon's opt-in HTTP, for a phone or for UI development. Not Wails |
| compatibility | yo has one user and isn't public yet, so there is no migration and no backward compatibility to keep. Config and ledger formats stay as they are unless there's a reason to change them; the changes planned are new `machine` and `harness` fields in ledger records, and a `harness` setting in config |
| the port | it keeps today's *effects* (ledger, config, git, files written, commands launched); output may change, deliberately. Each command is tested by `yo-core` unit tests on its plan, with a fake `Sys`, and was compared with `scripts/yo` by hand while porting, since bash `yo` went about a day after the Rust one ran. See [testing the port](#testing-the-port) |
| daemon | optional for the CLI and the TUI; started on demand by what needs it (`yo app`, background sessions, the relay). Listens only on a Unix socket unless TCP is turned on. A launchd agent can come later |
| how it's built | gates cheapest first, everything CI runs can run locally (`cargo xtask`), review skills by hand, and only the maintainer loosens a gate; see [how it's built](#how-its-built) |
| "latest focus" | raw: the last prompt and the last tool used. A summary can come later |
| phone | Claude's Remote Control for Claude sessions, for now; for every harness later, the same web UI over the daemon's opt-in HTTP, on a tailnet |
| iTerm automation | keep shelling out to `osascript` |
| distribution | a Homebrew formula (`mmobeus/tap/yo`) from release binaries, or `cargo install` from a checkout; then `yo setup` installs a harness's files (`Harness::bundle`) from copies built into yo, with a manifest so a rerun or `--uninstall` knows its own copies, and every command says when an upgrade left them stale (#34) |
| git, gh | keep shelling out to the `git` and `gh` CLIs, not a git library: same behavior as today, same auth, same config |

## Principles

- **yo coordinates; harnesses do the work.** yo starts, finds, reaches, resumes
  and cleans up sessions. It doesn't show conversations, edit prompts or answer
  tool calls: each harness's own UI does that. Where yo runs a session with no
  UI (over ACP), the way to take over is opening it in the harness's own TUI.
- **Build on what harnesses ship.** Claude Code now has background sessions
  and a status board (`claude agents`), Remote Control and cross-session
  messaging; others will grow similar things. yo uses those through an adapter
  rather than rebuilding them. Each phase starts by checking what's shipped
  since the last one.
- **Claude-specific is fine; Claude-assumed isn't.** A harness may have extras
  (a settings layer, memory, personas, Remote Control). The core never
  requires them: anything a harness lacks shows up as "unknown" or "not
  available", not as a failure.
- **Two of everything that's meant to be general.** An interface with one
  implementation is a guess. Each interface meant to be general (harnesses,
  ACP's harnesses, the yo API) gets a second implementation in the phase that
  introduces it, or soon after.
- **The same bar for every change.** Most of yo will be written by coding
  agents, so every PR clears the same gates whoever wrote it, and everything
  CI runs can run locally. See [how it's built](#how-its-built).

## Language: Rust

Go was the other candidate, and the pragmatic one for a tool made of processes,
events and glue. Rust wins here for reasons specific to how yo is built:

- **Most of the code will be written by Claude Code, not by hand.** That makes
  the compiler the first reviewer. The mistakes an agent makes are mostly
  silent ones: a field it forgot to handle, an optional value treated as
  always present, a shared map touched from two tasks, an error dropped.
  Exhaustive `match`, `Option`, `Result` and the borrow checker turn many of
  those into build errors that must be fixed before anything runs. Go
  compiles them.
- **Types as the contract between phases.** Config, ledger records, events,
  the daemon's API and ACP's messages are all serde types. Later phases,
  often built by separate sessions, build against those types rather than
  against prose.
- **Ceremony is cheap when an agent writes it.** Rust's extra code (error
  enums, ownership plumbing) mostly costs typing, and the typing isn't done by
  hand.
- **Few human readers**, even once public, which takes away Go's main
  remaining advantage: that newcomers can read and contribute easily.
- **ACP's official Rust crate** (`agent-client-protocol`) is at 1.0, and Zed
  uses it. **Tauri**, for the app, is Rust too: the same workspace, the same
  `yo-proto` types, the same gates.

### Keeping Rust's costs down

- **A synchronous core.** Config, the ledger, name resolution, sweep's rules,
  git calls, harnesses and the iTerm runner don't need async. Only the daemon
  (the ACP runner, the relay, the API) uses tokio. The CLI stays plain and
  starts fast, which matters for the ledger hook that runs at every `claude`
  start and exit.
- **Simple ownership.** Owned data, `Arc` where something is shared, and one
  task that owns the daemon's session table and takes messages. No
  long-lived borrowed references in the API.
- **Fast incremental builds:** a workspace of several crates, with the heavy
  dependencies (tokio, axum, ratatui, the ACP crate, Tauri) kept out of
  `yo-core`.
- **One bar to clear:** `cargo xtask ci`, the same in every session's loop as
  in CI; see [how it's built](#how-its-built).

## The app: Tauri

`yo app` is a Tauri app around one web UI (`web/`). Serving that UI from the
daemon to a browser was the alternative. Tauri is the main path because:

- **The daemon needs no open port.** A browser can't talk to a Unix socket, so
  a browser UI means the daemon listens on localhost, where every page in every
  browser on the machine can try to reach it. That needs `Origin` and `Host`
  checks, and a way to hand the browser a token. In the app, the UI calls the
  app's Rust side, which talks to the daemon over its socket (mode `0600`). No
  port is open at all, which removes that class of attack rather than
  defending against it.
- **Tauri's own security model** adds defense in depth: the UI can call only
  the native commands the app grants it.
- **Native features that suit yo:** a menu bar icon with how many sessions
  need you, native notifications for "needs me", a global shortcut to bring yo
  forward, the daemon's token kept in the Keychain, and `yo://` links.

Rules for the app:

- **It holds no yo logic.** Its Rust side forwards requests to the daemon and
  provides the native features above, nothing more. Logic in the app would
  duplicate the daemon, and the CLI, TUI and phone would not have it.
- **Least privilege.** The UI is granted only the Tauri commands it uses.
- **One UI, one transport layer.** The UI reaches the daemon through a small
  adapter: Tauri's bridge to the socket inside the app, HTTP everywhere else.
  Nothing else in the UI knows which it's using.

Where the browser path is still used:

- **A phone, and other machines.** These need the daemon reachable over the
  network (a tailnet) anyway, so the daemon serves the same UI there, over its
  opt-in HTTP. Claude sessions can also use Remote Control.
- **UI development:** a dev server in a browser, against the daemon's HTTP on
  localhost, turned on for that.

Costs, accepted:

- **Signing.** Builds for yourself can be signed ad hoc. Shipping to others
  needs an Apple Developer ID and notarization on each release. The CLI
  doesn't: a Homebrew formula's download isn't quarantined, and arm64
  builds are signed ad hoc by the linker.
- **One engine.** On macOS Tauri uses the system's WebKit, so the UI is tested
  against WebKit, not Chrome.

Wails was considered and rejected.

## Shape

```
     yo (CLI)        yo tui        yo app (Tauri)     phone browser
         \             |                 |                  |
          \      the yo API (trait) -----+                  |
           \      |            \                            |
           \   in process    yo-client (Unix socket)   opt-in HTTP
            \     |               \                         |
             \    |           yo serve (daemon, async) -----+
             \    |           ACP runner · relay · API · event tailer
              \   |                 |
              +------------ yo-core (sync) ---------------------------+
              | config · registry (repos, profiles, agents, homes)    |
              | ledger · resolve (`yo NAME`) · create · resume · down |
              | sweep · events · status                               |
              | harnesses: claude · codex · ...                        |
              | runners: iterm · acp (via the daemon) · ...            |
              | sys: clock, ids, processes, fs roots (fakeable)       |
              +-------------------------------------------------------+
```

- The CLI calls `yo-core` directly and works with no daemon running, as
  today. For what only a daemon has (ACP sessions, the relay), it starts one
  or asks the running one, through `yo-client`.
- The TUI and the app use **the yo API**: one trait, with its types in
  `yo-proto`, and two implementations.
  - **In process,** over `yo-core` and the event log files, with no daemon.
    The TUI uses it from phase 4, before the daemon exists, and whenever
    none is running.
  - **Over `yo-client`,** talking to the daemon on this machine through its
    Unix socket. The app always uses it, and the TUI does once a daemon is
    running, since only the daemon has ACP sessions and the relay.

  Nothing above the trait knows which implementation it has.
- **The daemon is local; sessions needn't be.** The TUI and app talk to this
  machine's daemon. Once the hub exists, that daemon is how they reach every
  machine's sessions, so nothing in the TUI or app assumes a session is on
  this machine.
- **The browser path** (a phone, UI development) doesn't use `yo-client`,
  which is Rust: the web UI talks to the daemon's opt-in HTTP directly.
- There is one binary, `yo`, with `yo serve` and `yo tui` among its
  subcommands. No `--mode=` flag: `mode` already means in place vs worktree.

## Crates

A Cargo workspace:

| crate | kind | what it holds |
| --- | --- | --- |
| `yo-core` | lib, sync | everything that's site logic today, plus harnesses and the iTerm runner; see [the core's API](#the-cores-api) |
| `yo-cli` | bin `yo` | clap commands, flags, output rendering, prompts. Thin over `yo-core` |
| `yo-proto` | lib | the yo API: its trait and types (requests, responses, the event stream), shared by daemon and clients; an OpenAPI spec is generated from them. Types only |
| `yo-local` | lib | the yo API in process, over `yo-core` and the event log files: what the TUI uses with no daemon |
| `yo-client` | lib | the yo API over this machine's daemon's Unix socket, used by the TUI, the app and the CLI. Later, also over HTTPS, for daemons reaching the hub and each other |
| `yo-daemon` | lib, async | `yo serve`: the API, the event tailer, the ACP runner, the relay |
| `yo-tui` | lib | `yo tui` |
| `yo-app` | bin, Tauri | `yo app`: the Tauri shell around `web/`. Forwards to the daemon through `yo-client`; tray, notifications, shortcut, Keychain. No yo logic |
| `xtask` | bin, dev | the gates, run the same way locally and in CI (`cargo xtask ci`); see [how it's built](#how-its-built) |
| `web/` | not Rust | the web UI: inside `yo-app`, and served by the daemon's opt-in HTTP |

Inside `yo-core`:

```
yo-core/src/
  sys.rs        Sys: clock, ids, rng, process table, command runner, env, paths
  fake.rs       the fake Sys, for tests (the `fake` feature); outside sys.rs, so not exempt from its bans
  error.rs      yo-core's errors; the CLI renders them
  paths.rs      XDG dirs, config path, state dir, worktree roots
  machine.rs    this machine's id
  config.rs     the config file: types (serde, deny_unknown_fields, the file's order kept), load, add-repo and add-profile
  ojson.rs      JSON that keeps its key order, for files yo edits in place (the config, ~/.claude.json)
  place.rs      an unregistered URL's repo: a profile's, --add-to's, asked, or a guest; cloned and registered
  registry.rs   resolved repos, profiles, agents, homes; precedence (flag > repo > profile > defaults, env beats file)
  git.rs        the git CLI: worktrees, branches, remotes, status, default branch
  github.rs     the gh CLI: PR and issue titles, PR listings for sweep; URL parsing
  ledger.rs     session records: load, write, drop, liveness, sync, the hook; later put, adopt, retitle, get
  resolve.rs    `yo NAME`: the ordered lookup
  reach.rs      reaching what it found: focus a running session (typing the message), resume a stopped one
  create.rs     plan a new session: INPUT (unnamed, PR, issue, existing or new branch), worktree, branch, title, codename, brief
  session.rs    for the ledger commands: where a directory sits (a yo worktree, a clone), a session by query, recording one
  down.rs       tear down the current session
  end.rs        end a running session the way a closed terminal would (down, when closing its tab doesn't)
  sweep.rs      classify worktrees; remove
  naming.rs     codenames, titles, truncation
  plan.rs       Plan and Action: what a command will do, before it does it
  events.rs     event types, append, read, tail
  status.rs     the status model: a fold over records, liveness and events
  harness/      the Harness trait; claude.rs (live sessions; later settings layer, trust, memory, hooks, agents), codex/
  runner.rs     the Runner trait, the dry runner; iterm.rs (and its scripts in iterm/). The ACP runner's client side lives here; its processes live in the daemon
  message.rs    delivering a message: choosing a method, and the result
```

## The core's API

These signatures are the shape, not the final code: phase 1 may rename things.

### Effects go through `Sys`

Everything that's nondeterministic or touches the outside goes through one
trait object. The real one does what bash does today; tests use a fake
(`yo-core`'s `fake` feature, so `yo-cli`'s tests can use it too). This is
what makes the suite possible.

```rust
pub trait Sys: Send + Sync {
    fn now(&self) -> SystemTime;
    fn new_session_id(&self) -> String;              // a v4 UUID; today: uuidgen
    fn pick(&self, n: usize) -> usize;               // codenames; today: $RANDOM
    fn env(&self, key: &str) -> Option<String>;      // Some("") is set but empty
    fn home(&self) -> PathBuf;
    fn cmd(&self) -> &dyn CommandRunner;             // git, gh, osascript, claude, codex
    fn procs(&self) -> &dyn ProcessTable;            // a pid's comm (None: not running), tty, ppid; and signal(pid, HUP | TERM)
    fn hostname(&self) -> String;
    fn exe(&self) -> PathBuf;                        // this yo, for hooks that call it back
    fn sleep(&self, d: Duration);                    // iTerm loading a profile yo just wrote
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output>;   // program, args, cwd, env, stdin
    fn spawn(&self, cmd: &CommandSpec) -> io::Result<()>;     // left running after yo exits
}
```

Files are touched with plain `std::fs` under paths the `Sys` gives (`home`, and
XDG variables via `env`), so a throwaway `HOME` sandboxes them, as today.

The real `Sys` takes its randomness from `/dev/urandom`, asks `ps` whether a
pid is alive and what it is, and sends signals with `kill` (only `down` does,
to end a session its tab left running), rather than pulling in `uuid`, `rand` or `libc`
(the workspace forbids `unsafe`). The fake lives in `fake.rs`, not under
`sys.rs`, so the banned calls `sys.rs` may make aren't open to it; other
crates use it only as a dev-dependency feature.

### Context and config

```rust
pub struct Ctx {
    pub sys: Arc<dyn Sys>,
    pub paths: Paths,           // config path, state dir, settings dir, ...
    pub registry: Registry,     // config resolved: repos, profiles, agents, global settings
    pub harnesses: Vec<Box<dyn Harness>>,
}

impl Ctx {
    /// Soft load: a missing config is single-repo mode against the cwd, as today.
    pub fn load(sys: Arc<dyn Sys>, cwd: &Path) -> Result<Ctx>;
}

pub struct Config {            // the JSON file, as today, plus harness settings
    pub defaults: DefaultsCfg,
    pub profiles: Ordered<ProfileCfg>,
    pub repos: Ordered<RepoCfg>,
    pub repos_in: Vec<Text>,                       // ~/ expanded when resolved
    pub agents: Ordered<AgentCfg>,
    pub harnesses: Ordered<HarnessCfg>,            // new in phase 2, optional: command, extra args per harness
}
```

`Ordered<T>` is a JSON object that keeps the file's order, because order is
behavior: the first repo registered is the default one, and repos are listed
in that order. `Text` is a scalar taken as its text, as bash takes it (a number
or a boolean, too).

```rust

impl Registry {
    pub fn load(sys: &dyn Sys, config: &Path, cwd: &Path) -> Result<Registry>;  // single-repo mode with no file
    pub fn repos(&self) -> &[Repo];                            // registration order
    pub fn repo(&self, key: &str) -> Option<&Repo>;
    pub fn repo_at(&self, dir: &Path) -> Option<&Repo>;        // the repo whose clone is dir
    pub fn default_repo(&self) -> Option<&Repo>;               // YO_DEFAULT_REPO, else the first
    pub fn repos_of_profile(&self, name: &str) -> impl Iterator<Item = &Repo>;
    pub fn detect_repo(&self, sys: &dyn Sys, dir: &Path) -> Option<&Repo>;  // by remote, else by clone
    pub fn repo_for_url(&self, sys: &dyn Sys, url: &str) -> Option<&Repo>;  // any GitHub URL
    pub fn github_for(&self, sys: &dyn Sys, repo: &Repo) -> Option<String>; // owner/repo
    pub fn base_ref(&self, sys: &dyn Sys, repo: &Repo) -> String;
    pub fn profile(&self, name: &str) -> Option<&Profile>;
    pub fn profile_of(&self, repo: Option<&Repo>, picked: Option<&str>) -> Option<&Profile>;
    pub fn profile_for_owner(&self, owner: &str) -> Option<&Profile>;
    pub fn agent(&self, name: &str) -> Option<&Agent>;
    pub fn home_repo(&self, name: &str) -> Option<&Repo>;      // later: NAME-home
    pub fn layered<T>(&self, flag: Option<T>, repo: Option<&Repo>, picked_profile: Option<&str>,
                      of_repo: impl Fn(&Repo) -> Option<T>, of_profile: impl Fn(&Profile) -> Option<T>,
                      default: T) -> T;
    pub fn mode(&self, flag: Option<Mode>, repo: Option<&Repo>, picked: Option<&str>) -> Mode;
    // branch_prefix, claude_cmd, window_group, memory_dir, title_prefix, ...: each over `layered`
}
```

`Registry::layered` is the one place precedence lives: flag, then repo, then
profile, then `defaults`. Each setting is a named method over it. A global
setting's environment variable beats the file when it's set, even to nothing,
as bash's `${X+x}` does; `Defaults` holds the global settings resolved that
way. Today that's spread over `*_for` functions.

Harnesses are looked up through `Ctx::harnesses` (`Ctx::harness(name)`,
`Ctx::harness_of(record)`); their settings (`harnesses.<name>`: `command`
and `args`) are in the registry's `Defaults`, and `Registry::harness` layers
the `harness` setting like `mode`.

Two small differences from bash, both deliberate: a profile's `owners`
written as one string is split into words (bash scanned it a character at a
time, which no one meant), and keys made only of dots are refused.

The machine id isn't in `Ctx`: it's read (and made, the first time) when a
ledger record is written, so commands that write nothing never create it.

A setting, `harness` (default `claude`, `YO_HARNESS`), sits at every level
like `mode` does, with a `--harness=H` flag (phase 2). The `claude` setting
stays, as the `claude` harness's command: a profile's `claude`, then
`YO_CLAUDE_CMD` or `defaults.claude`, beat `harnesses.claude.command`, which
is the same setting for any harness. The Claude harness does that layering
itself (`Harness::command`), so the registry names no harness.

### The ledger

The record keeps today's fields and adds `machine` and `harness`. The file
layout stays: one JSON file per session in `$XDG_STATE_HOME/yo/sessions/`,
keyed by the worktree path.

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub worktree: PathBuf,
    pub session_id: Option<String>,       // the harness's id; not always a UUID, not always known at launch
    pub name: Option<String>,
    pub title: Option<String>,
    pub repo: Option<String>,
    pub profile: Option<String>,
    pub mode: Option<Mode>,               // worktree | here
    pub guest: Option<String>,            // owner/repo of a guest repo
    pub agent: Option<String>,
    pub home: bool,                       // an agent's home
    pub remote_control: Option<String>,
    pub codename: Option<String>,
    pub brief: Option<String>,
    pub by_id: bool,
    pub harness: Option<String>,          // new; absent = claude
    pub machine: Option<MachineId>,       // new; absent = this machine
    pub runner: Option<RunnerKind>,       // new; absent = iterm
    pub created_at: Option<i64>,          // unix seconds, as bash writes them
    pub updated_at: Option<i64>,
    pub ended_at: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,   // unknown keys survive a rewrite
}

pub struct Ledger { /* sys, paths */ }

impl Ledger {
    pub fn all(&self) -> Vec<Entry>;                          // Entry = key + record
    pub fn write(&self, e: &mut Entry) -> Result<()>;         // back to its own file; stamps updatedAt, machine
    pub fn put(&self, rec: Record) -> Result<Key>;
    pub fn adopt(&self, a: &Adopt) -> Result<Key>;
    pub fn drop(&self, key: &Key) -> Result<bool>;            // false: there was none
    pub fn drop_dir(&self, worktree: &Path) -> Result<bool>;
    pub fn retitle(&self, key: Option<&str>, id: Option<&str>, cwd: &Path, title: &str) -> Result<bool>;
    pub fn learn_id(&self, key: &Key, session_id: &str) -> Result<()>;  // a harness that names its own ids
    pub fn sync(&self, clones: &[SyncClone], live: &[LiveSession]) -> Result<Vec<Adopted>>;
    pub fn hook(&self, ev: &HookEvent);                       // never fails the caller
    pub fn get(&self, q: &str) -> Result<Option<Entry>, Vec<Entry>>; // id or dir, 8+ prefix, name; the harness's transcripts are session::find's fallback
}

pub enum HookEvent {                                          // from Harness::hook_event
    Start { id: String, cwd: PathBuf },
    End { id: String, cwd: PathBuf, deliberate: bool },       // Claude: /exit, ctrl-d
}

pub enum State { Running { pid: u32 }, Ready, Ended, Gone, NoId }

pub fn state_of(rec: &Record, live: &[LiveSession]) -> State;   // today's status_of

// runner.rs: whether a running session still has a tab, by its terminal or
// the tab recorded at launch (Record::tab), against what the runner shows
pub enum Tab { Held, Gone, Unknown }
pub fn tab_of(sys: &dyn Sys, tab: Option<&str>, pid: u32, shown: Option<&Tabs>) -> Tab;
```

`running (no tab)` in `yo sessions` is `Running` with `Tab::Gone`: the
harness still lists the session, but its tab is gone (#35). `Unknown` (the
runner can't say, or no tab was recorded and its terminal isn't shown) reads
as plain `running`.

Writes go to a temporary file followed by a rename, as today, with sorted
keys and two-space indents. One difference: Python escaped non-ASCII text
(`\u00e9`), and serde writes it as UTF-8; both read back the same. A record
goes back to the file it came from, even when its session id has changed
(after a `/clear`). `sync` returns what it adopted, for the CLI to report,
rather than printing it.

**Session ids known late.** yo picks a Claude session's id before launching it
(`--session-id`), which is how the ledger knows a session before it has
started, and how `resume-all` brings back one that crashed early. Other
harnesses may only report an id once they're running. For those, the record is
written with no id, and `learn_id` fills it in when the harness reports one: by
its hook, by its list of live sessions, or from ACP's `session/new`. Until then
the record is `no-id`, as a record without an id is today.

### Commands as plans

Every command that changes something first builds a `Plan`, then runs it.
`--dry-run` prints the plan; tests assert on it. Sweep, `down`, `resume-all`
and creating a session all get dry runs from one mechanism.

```rust
pub struct Plan { pub notes: Vec<Note>, pub actions: Vec<Action> }

pub enum Action {
    Clone { url: String, into: PathBuf },
    RegisterRepo { profile: Option<String>, key: String, path: PathBuf },
    Fetch { repo: PathBuf, refspec: String },
    AddWorktree { repo: PathBuf, path: PathBuf, branch: BranchSpec },
    RemoveWorktree { repo: PathBuf, path: PathBuf },
    DeleteBranch { repo: PathBuf, branch: String },
    MirrorFiles { from: PathBuf, to: PathBuf, files: Vec<PathBuf> },
    Prepare(HarnessPrep),            // a harness's setup: Claude's settings layer, trust, local settings, memory
    PutRecord(Record),
    DropRecord(Key),
    Launch(LaunchSpec),
    Deliver(Delivery),               // a message; see [messages](#messages)
    CloseOwnTab,
}

pub fn plan_create(ctx: &Ctx, req: CreateRequest) -> Result<Plan>;
pub fn plan_reach(ctx: &Ctx, e: &Entry, st: State, label: &str, o: &ResumeOpts) -> Result<Plan>; // what resolve found
pub fn plan_resume(ctx: &Ctx, q: &str, opts: ResumeOpts) -> Result<Plan>;
pub fn plan_resume_all(ctx: &Ctx, opts: ResumeOpts) -> Result<Plan>;
pub fn plan_down(ctx: &Ctx, here: &Path, opts: DownOpts) -> Result<Plan>;
pub fn plan_sweep(ctx: &Ctx, opts: SweepOpts) -> Result<(Vec<Verdict>, Plan)>;

pub fn execute(ctx: &Ctx, plan: &Plan, runners: &Runners, out: &mut dyn Report) -> Result<()>;
```

`Report` is where output goes: the CLI renders it as today's text; the daemon
turns it into events. The core never prints.

As built in phase 1, not every command that changes something has a plan
yet. `resume-all` plans each session with `plan_resume`, but choosing which
ones (only `ready`, and not those running without a tab, which it lists
instead) happens in `yo-cli`, and its `--dry-run` lists them rather than
printing a plan; `resume` refuses running and gone sessions in `yo-cli` too.
What a running session without a tab is, and the wait for those still
exiting, are `yo-core`'s (`reach::tabless`, `reach::await_tabless`). `adopt`, `forget` and `title` write the ledger directly, and
`title` and `adopt` call the iTerm runner's own methods rather than going
through `Runner`. `config edit` runs `$EDITOR` outside `Sys`. These move
into `yo-core` plans (`plan_resume_all` and small ones for the ledger
commands) when the daemon or the TUI needs them, no later than phase 4.

### `yo NAME`

Resolution is a pure function over the registry, the ledger and the live
sessions. It returns what NAME means; the caller acts on it.

```rust
pub enum Resolved {
    New,                                    // a flag, a repo or profile (positionally), or nothing matched
    Agent(String),                          // a registered agent
    Live(LiveSession),                      // by the harness's own name for it
    Ledger(Box<Entry>, State),              // by yo name, id, 8+ prefix; running, ready or ended
    Home { repo: String, target: HomeTarget }, // NAME-home: running, else live there, else latest ready, else new in place
}

pub fn resolve(reg: &Registry, entries: &[Entry], live: &[LiveSession],
               args: &[String], has_flags: bool) -> Result<Resolved, Ambiguous>;
```

Today this order is spread over `main`, `cmd_reach` and `cmd_home_reach`.
Having it in one function, tested case by case, is one of the main things the
port buys.

## Harnesses

A harness is a coding agent's CLI: `claude`, `codex`, and later others. Most of
what's Claude-specific in yo today is in how it prepares, launches and watches
a Claude session, not in runners or sessions. All of that moves behind one
trait.

Like the rest of the core's API, the trait grows with the commands that use
it: phase 1's foundations have `name`, `live`, `capabilities` and `hook_event`, create added
`tui_command` and `prepare` (Claude's steps are `Action::Prepare(Prep::Claude(..))`; the
settings layer and memory directory are plain `WriteFile` and `MakeDir` actions), and
reaching and the ledger commands added `resume_command` and `session_cwd` (where a
session ran, from the harness's own records: Claude's session files, else its transcript).
Phase 2 added `default_command` and `command` (its configured command), `ids`,
`inside` (yo runs inside one of its sessions: Claude's `CLAUDECODE`) and
`persona_dir` (where its agents are defined: Claude's `.claude/agents`, in a
checkout and under `HOME`).

```rust
pub trait Harness {
    fn name(&self) -> &str;                                       // "claude", "codex"

    // required
    fn tui_command(&self, s: &SessionSpec) -> Argv;              // start its TUI in s.dir, with s.name and s.first_message
    fn resume_command(&self, id: &str, s: &SessionSpec) -> Argv; // reopen a session in its TUI
    fn ids(&self) -> IdPolicy;                                    // yo picks the id | the harness reports it
    fn live(&self, sys: &dyn Sys) -> Vec<LiveSession>;            // what's running now: id, pid, cwd, name
    fn acp(&self) -> Option<Argv>;                                // how to run it over ACP, if it can be

    // optional: what a harness has, it says here
    fn capabilities(&self) -> Capabilities;
    fn hook_event(&self, payload: &str) -> Option<HookEvent> { None }  // its ledger hook's payload, made neutral
    fn prepare(&self, ctx: &Ctx, s: &SessionSpec) -> Result<Vec<HarnessPrep>> { Ok(vec![]) }
    fn status(&self, sys: &dyn Sys) -> Vec<(SessionId, HarnessStatus)> { vec![] }
    fn relay(&self) -> Option<&dyn Relay> { None }
}

pub struct Capabilities {
    pub settings_layer: bool,     // Claude: --settings, composed per session
    pub memory: bool,             // Claude: autoMemoryDirectory
    pub personas: bool,           // Claude: --agent; yo's `agents`
    pub ledger_hooks: bool,       // Claude: SessionStart / SessionEnd call yo (`_session-hook`, then `_hook`)
    pub remote_control: bool,
    pub models: bool,             // --model
}

pub struct SessionSpec {
    pub dir: PathBuf,
    pub name: String,             // yo's name; passed to the harness so others can address it
    pub id: Option<String>,       // when the harness lets yo pick it
    pub model: Option<String>,
    pub persona: Option<String>,
    pub codename: String,         // "Plucky Gary": yo picks it; the harness decides how the session is told
    pub first_message: Option<String>, // the brief, without the codename
    pub permission_mode: Option<String>,
}
```

- **The Claude harness** owns what's Claude-specific in yo today:
  `--session-id`, `--settings` and the composed settings layer, the trust
  seeding in `~/.claude.json`, each worktree's `.claude/settings.local.json`
  (MCP pre-approval), shared memory, `--agent` and agents' homes, the ledger
  hooks, Remote Control names, live sessions from `~/.claude/sessions`, status
  from `claude agents --json`. It tells the session its codename at the end
  of the first message, or with `--append-system-prompt` when the message is
  a slash command, whose arguments it would otherwise join. It also starts
  passing `--name <yo name>`, so other sessions (and the relay) can address
  it by yo's name. Another harness tells the session its codename however it
  can.
- **The Codex harness** (phase 2) is the second implementation: launch, resume,
  live sessions, a ledger record with an id learned after launch, and
  messages typed into its tab. Its ACP command (`codex-acp`) is used from
  phase 5. Everything else is "not available" until Codex offers it. Its details (how it names sessions, what
  it reports while running, whether it has hooks) are checked when that phase
  starts, against what Codex ships then.
- **Flags that don't apply** to a session's harness (`--agent` on Codex, say)
  are an error that names the harness, not silently ignored.
- **Status** comes from whatever the harness offers. For Claude that's
  `claude agents --json` (a research preview; the docs say to use it rather
  than read `~/.claude/jobs`, so yo reads it through one adapter and tolerates
  fields changing). A harness with nothing reports `unknown`, which the status
  model treats as normal.

## Runners

A runner is how a session runs. A session is a record in yo; its runner and
harness are fields on it.

As built in phase 1, the trait has `kind`, for ledger records; `launch`,
which also returns the tab it opened, for the record; `focus` (by terminal,
else by recorded tab); `close_own_tab`; `tabs`, what the runner shows, for
`tab_of`; and `ends_sessions`, false for the dry runner (`YO_DRY_LAUNCH`), which
says what it would do and does none of it. `locate`, `deliver` and `retitle`
below are the shape for later. A launch carries the harness's command already
built (`Argv`), so a plan shows the exact command line.

```rust
pub trait Runner {
    fn kind(&self) -> RunnerKind;                                  // iterm | acp | ...
    fn launch(&self, h: &dyn Harness, spec: &LaunchSpec) -> Result<Launched>;
    fn locate(&self, live: &LiveSession) -> Option<Location>;      // where it is, if this runner can tell
    fn deliver(&self, at: &Location, message: &str) -> Result<Delivered>;
    fn focus(&self, at: &Location) -> Result<()>;
    fn close(&self, at: &Location) -> Result<()>;
    fn retitle(&self, at: &Location, title: &str) -> Result<()>;
}

pub struct LaunchSpec {
    pub session: SessionSpec,
    pub title: String,
    pub window: WindowChoice,        // tab | window | config
    pub group: String,
}
```

- **Liveness isn't a runner's business.** Each harness says what's running;
  a runner only answers where a live session is and how to reach it.
- **iTerm:** today's runner. It runs the harness's TUI command in a tab
  (argv, turned into a single-quoted command line as today), with today's
  AppleScript kept as `.applescript` files embedded with `include_str!` and
  run through `CommandRunner`, so tests see them. A session is located by its
  tty, or else by the tab recorded when it was opened: AppleScript lists
  only the member of a Claude Code workgroup tab that's showing, so a chat
  behind CLI has no listed tty. Works for any harness with a TUI.
- **Closing a tab doesn't always end its session.** A workgroup tab closed
  while it shows CLI leaves the chat running, with no tab (#35; the details
  are in the iTerm runner's docs). So `down` ends its session itself
  (`end.rs`): a hangup to the shell above it, then to the session, then
  SIGTERM, each sent only while the harness still lists that session on
  that pid. From inside the session, a detached `yo _end-session` does it
  after iTerm's undo window.
- **ACP** (phase 5): for sessions nobody is watching. The daemon runs the
  harness's ACP command (`claude-agent-acp`, `codex-acp`, ...) as a child
  process and talks JSON-RPC to it over stdin and stdout, through the
  `agent-client-protocol` crate:
  - `session/new` in the worktree starts it, and its `sessionId` goes into
    the ledger.
  - `session/prompt` sends messages.
  - `session/update` notifications become events. ACP v2's `state_update`
    (running, idle, requires_action) gives "needs me" for every harness. v2 is
    a preview; on v1, yo infers it from turns in flight and pending
    permission requests (see [open questions](#open-questions)).
  - `session/request_permission` becomes a "needs me" with the question in
    it. Answering it from the TUI or app is possible, but yo's first answer
    is the principle above: open the session in the harness's TUI
    (`resume_command`) and answer it there.
  - After a daemon restart, `session/resume` reattaches, where the harness
    supports it.

  ACP replaces two earlier ideas: driving `claude --print` over its own
  stream-json format (one harness only), and Claude's `claude --bg` (one
  harness only, and its process isn't yo's to manage). It's built and tested
  against two harnesses at once: Claude and Codex.
- **Moving between runners** is the harness's `resume_command` in a tab: a
  session started over ACP is opened in its TUI when you want to take over.
  Whether a harness lets its TUI and an ACP client share a live session isn't
  defined by ACP; until a harness supports it, taking over means the daemon
  closes its side first.
- **Remote workers** (phase 7) are another machine's daemon, reached through a
  hub. The trait makes no assumption of a tab, a tty or a local process.

## Messages

`yo NAME 'message'` reaches a session and gives it the message. There are three
ways to deliver one, and they aren't the same thing:

| method | the session sees | reaches | harnesses |
| --- | --- | --- | --- |
| type into the tab | **you**: your input, with your authority | a session in an iTerm tab yo can find | any with a TUI |
| ACP `session/prompt` | **you** | sessions on yo's ACP runner | any ACP harness |
| the relay | **another session**: a peer's message | any Claude session the relay can address: on this machine, on others over Remote Control, in the cloud | Claude |

### The relay

The relay is a Claude session the daemon keeps running, on the ACP runner,
whose only tools are Claude Code's `ListAgents` and `SendMessage`. To send, yo
prompts it to send exact text to a named session. yo doesn't trust the model's
summary of what it did: it reads the relay's tool calls from the
`session/update` stream and checks that `SendMessage` went to the right session
with exactly the right text. Anything else is a failed delivery, and is
reported as one.

- **To verify first, in phase 5:** that a session run through
  `claude-agent-acp` (which is built on the Claude Agent SDK) opens an inbox
  and has `ListAgents` and `SendMessage`. Claude's docs say `--bare` sessions
  get no inbox, and don't say what an SDK session gets. If it doesn't, the
  relay is a plain `claude -p` run per message, with the same tool limits and
  the same check of its output, instead of a long-running ACP session.
- **Reaching other machines** needs the relay itself connected to Remote
  Control, and signed in with a claude.ai account rather than an API key.
- **It runs on a cheap model** (Haiku), and costs one short turn per message.
- **Replies come back.** A session answers a peer with `SendMessage` to the
  sender's address, so replies arrive at the relay. The daemon turns them into
  events: yo gets an inbox, which today doesn't exist ("messages are
  invisible", in #9).
- **Addresses are names.** That's why the Claude harness passes
  `--name <yo name>` at launch.

### Why typing stays

A peer's message isn't the user speaking. Claude Code says cross-session
messages never count as user consent, and depending on the receiving
session's settings (`crossSessionInbound`) they can be held for approval or
refused. "Go ahead and push", sent through the relay, is a teammate's request,
not your go-ahead. That's right for coordination: yo's own notices, or passing
on another session's report. It's wrong when you mean "this is me".

So `yo NAME 'message'` delivers in your voice where it can:

1. ACP `session/prompt`, for a session on the ACP runner.
2. Typing into the tab, for a session in iTerm.
3. Otherwise, the relay, for a Claude session neither can reach: one started
   outside yo (with `claude --bg`, say), one on another machine, or one with
   no tab yo can find. Today that case
   prints "your message was not delivered". yo says when it used the relay.

`--via=tab|acp|relay` picks one explicitly, and a `messages` setting can make
the relay the default. Typing keeps today's type, pause, then Enter sequence.

### What's not used

- **The inbox socket** each Claude session opens
  (`CLAUDE_CODE_MESSAGING_SOCKET`) is documented only for a session's own
  hooks and scripts. yo could post to it, but its format isn't specified and
  may change. Revisit if it's documented.
- **Channels** (an MCP server pushing into a session) need a flag at launch,
  plus a development flag for custom ones, and are a research preview.

## Machine ids

Design for several machines now, build it in phase 7.

- **`MachineId`**: a UUID created once and stored at
  `$XDG_STATE_HOME/yo/machine-id`, with the hostname as its display name.
  Not the hostname itself, which changes and collides.
- **What carries it from the start:**
  - ledger records (`machine`, written on every put)
  - every event
  - every session as the daemon's API returns it, and every request that
    targets a session (`SessionRef { machine, key }`)
  - the daemon's own identity (`GET /v1/info`)
- **A session's global identity** is `(machine, key)`. The ledger key is
  derived from the worktree path, so two machines can have the same key; the
  session id alone isn't always known (`no-id`, and ids learned late).
- **What doesn't need it:** config. Each machine has its own config file;
  the hub will learn which machine can run which repo and harness from each
  daemon's registry.

## Events and status

The types live in `yo-core` from phase 1 so the ledger hook already writes
through them.

### The event log

JSON lines in `$XDG_STATE_HOME/yo/events/YYYY-MM-DD.jsonl`, one file per day,
so old days can be pruned. Writers append with `O_APPEND` under an advisory
lock (`flock`), one line per event, kept small. Hooks, the daemon (ACP
sessions, the relay) and the CLI all write here.

```rust
#[derive(Serialize, Deserialize)]
pub struct Event {
    pub v: u32,                      // schema version
    pub ts: i64,                     // unix ms
    pub machine: MachineId,
    pub harness: Option<String>,
    pub session: Option<String>,     // the harness's session id
    pub key: Option<Key>,            // ledger key, when known (YO_SESSION_KEY)
    #[serde(flatten)]
    pub kind: EventKind,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    SessionStart { cwd: PathBuf, source: String },
    SessionEnd { reason: String },
    Prompt { text: String },                     // UserPromptSubmit, or an ACP prompt; truncated
    ToolUsed { tool: String, summary: String },  // PostToolUse, or an ACP tool_call
    NeedsInput { why: String },                  // Notification, ACP requires_action or request_permission
    Idle,                                        // Stop, or ACP idle
    Usage { cost_usd: f64, context_pct: f32 },   // from the statusline, or ACP usage_update
    Message { from: String, to: String, text: String, via: Via },  // sent, or a reply to the relay
    Yo { action: String, detail: Value },        // created, resumed, reached, down, swept
}
```

Phase 1 keeps today's hook entry point, `yo _session-hook`, for
`SessionStart` and `SessionEnd` (today's ledger updates), so sessions started by
either version call the same hook. Phase 3 renames it `yo _hook`,
reading the event name from the payload, and adds Claude's
`UserPromptSubmit`, `PostToolUse`, `Notification` and `Stop` to yo's settings
layer. Events from ACP sessions use
the same kinds, so nothing downstream cares which harness or runner a session
is on.

### Session status

Status is derived, never stored: a fold over ledger records, the harness's
live sessions and status, and events.

```rust
pub struct SessionStatus {
    pub r#ref: SessionRef,
    pub name: String,
    pub codename: Option<String>,
    pub task: Task,                      // title + brief
    pub state: State,                    // running | ready | ended | gone | no-id
    pub activity: Activity,              // working | idle | waiting | unknown
    pub needs_me: Option<NeedsMe>,       // permission | input | turn done; since when
    pub focus: Option<Focus>,            // last prompt, last tool
    pub harness: String,
    pub runner: RunnerKind,
    pub repo: Option<String>, pub profile: Option<String>, pub agent: Option<String>,
    pub branch: Option<String>, pub pr: Option<u64>,
    pub usage: Option<Usage>,
    pub last_activity: Option<i64>,
}

impl SessionStatus {
    pub fn apply(&mut self, ev: &Event);              // pure; the TUI, app and `yo status` all use it
    pub fn observe(&mut self, s: &HarnessStatus);     // the harness's own report, e.g. claude agents --json
}
```

`needs_me` is set by `NeedsInput` and `Idle`, and cleared by the next
`Prompt` or `ToolUsed`. A harness's own report, when it has one, wins over
what's inferred from events: Claude's `waitingFor` (a permission prompt, input
needed, a dialog) says more than a hook can.

## The daemon

`yo serve`, from phase 5. It exists because the ACP runner and the relay need
a long-running process; everything else works without it. It stays small.

- **Started on demand** by whatever needs it, and left running. One per
  machine: a pid file and a lock in the state directory.
- **What it holds:** ACP child processes and their sessions, the relay, the
  event tailer, and the API.
- **API:** versioned JSON over HTTP semantics (`/v1/...`), with server-sent
  events for the event stream, and an OpenAPI spec generated from `yo-proto`.
  Mostly the core's plan and execute functions, made remote.
- **Listening:** on a Unix socket (mode `0600`) only, by default. The CLI, the
  TUI and the app all use it. TCP is opt-in, for a phone, the hub or UI
  development, and bound to localhost or a tailnet address, never all
  interfaces.
- **Security:**
  - a bearer token in `$XDG_STATE_HOME/yo/daemon-token` (mode `0600`) on
    every request, even on the socket
  - over TCP, additionally: the `Origin` and `Host` headers checked on every
    request, so a web page can't call the daemon from your browser
  - remote access (a phone, the hub) needs real authentication on top
- **The web UI** is the same files the app wraps. Over opt-in TCP the daemon
  serves them too, for a phone or a browser.

## Phases

This replaces the phase list in #9. Each phase is useful on its own.

0. **Design:** this doc.
   Then, before any Rust is written: **the gates** (#11), as described in
   [how it's built](#how-its-built).
1. **Core with CLI parity:** port yo behind the same commands and effects, with
   tests ([below](#testing-the-port)). One harness (Claude) and one runner
   (iTerm), but both already behind their traits. `--name` is passed at launch.
2. **A second harness:** Codex, in iTerm tabs. Launch, resume, the ledger
   with ids learned late, `--harness`, messages by typing. This is what proves
   the `Harness` trait. It also moves what phase 1 left outside
   `harness/claude/`: looking agents up in `.claude/agents` (`agent.rs`,
   `git.rs`, `create.rs`), and `yo-cli` reading `CLAUDE_CODE_SESSION_ID`
   and `CLAUDECODE`, which become `Harness` methods.
3. **Status and events:** the event log, Claude's extra hooks, Claude's status
   from `claude agents --json`, `yo status`, `yo watch`.
4. **`yo tui`:** session list with live status, a "needs me" filter, focus,
   send, start (with an editor for the brief), resume, sweep with checkboxes.
   Built on the yo API's in-process implementation (`yo-local`), since there's
   no daemon yet.
5. **The daemon, the ACP runner and the relay:** background sessions for
   Claude and Codex, opening them in a tab to take over, the relay and its
   inbox. `yo-client` lands here, and the TUI uses it when a daemon is
   running.
6. **`yo app`:** the Tauri app: the web UI, the menu bar icon, notifications.
   Maybe the MCP server.
7. **Reach:** a phone (Remote Control for Claude sessions; for any harness,
   the same UI over the daemon's opt-in HTTP on a tailnet), then the hub and
   remote workers.

## Testing the port

bash `yo` was retired about a day after the Rust `yo` could run (#21, the
switch), so there was no harness running the two side by side: it would have
been scaffolding for that one day. Instead:

- **What a command does is tested in `yo-core`, on its plan.** A test builds
  a registry, a ledger and live sessions, gives a fake `Sys` its answers
  (git, gh, osascript, the process table, the clock, ids), and asserts on the
  plan the command builds. Executing a plan against the fake, under a
  temporary `HOME`, shows the commands it ran and the files it wrote. These
  tests stay as the regression suite after the switch.
- **The port was compared with `scripts/yo` by hand,** command by command as
  it was ported, in a throwaway sandbox: `HOME`, `XDG_STATE_HOME` and
  `YO_CONFIG` pointed at a temporary directory, and `YO_DRY_LAUNCH=1` (the
  README's "Making changes"). Effects (the ledger, config, git, files written,
  commands launched) had to match. Output could change, but only
  deliberately, and the list below says how. Each PR said what it compared.
  With the script gone, the plan tests are what keep the effects.
- **The CLI follows common CLI idioms, not bash's quirks.** Effects are what
  must match; how arguments are parsed and how errors read needn't. The
  Rust `yo` differs from `scripts/yo` in these ways (the
  [reference](../reference.md#changes-from-the-bash-yo) lists the ones a
  user notices):
  - arguments are parsed by clap: `--flag value` works as well as
    `--flag=value`, and usage errors are clap's (with `yo --help` and
    `yo <command> --help`); `yo help` still prints the guide and the
    registry
  - the `-d` and `--resume` spellings of `down` and `resume` are gone;
    `ls`, `rm` and `teardown` stay
  - `--discard` needs `--yes`, and `--here` and `--worktree` can't both be
    given
  - a bad config's error is serde's: the key and where it is in the file
  - creating a session has `--dry-run`, which prints the plan and changes
    nothing; `YO_DRY_LAUNCH` keeps its meaning (everything but opening the tab)
  - create's progress reads a little differently: `==> fetching origin/main`
    without the `[repo]`, git's own chatter isn't shown, and the codename, id
    and brief are listed as `codename:`, `session:` and `brief:` (was
    `persona:` and `seeding claude with:`, the whole first message)
  - a session name with an empty or dot-only part, or a part starting with
    `-`, is refused (bash let `..` climb out of the worktree root)
  - every argument on the launch line is quoted only where it needs to be
  - iTerm's Default profile is read through JXA (`osascript -l
    JavaScript`, with `NSUserDefaults`) rather than `plutil` and Python, so
    no plist parser is needed; the dynamic profile it writes has the same
    keys and values (whole numbers written as `1`, not `1.0`). The progress
    line says `opened iTerm2 tab ...` after the tab opens
  - an unknown URL's repo is placed before planning, since the plan needs
    its clone: with `--dry-run`, yo says where it would go and stops there
  - `yo NAME` takes `--dry-run` too (it doesn't count as a flag, so the
    lookup still runs); a resumed session is passed `--name` like a new one
  - `down --yes` with uncommitted work refuses with the changes in the
    error, rather than after printing the whole summary; under
    `YO_DRY_LAUNCH`, `down` doesn't close the tab it runs in (bash closed it)
  - sweep keeps a worktree whose `git status` can't be read (bash read it
    as clean), and warns about PR listings on stderr
  - an agent's home in an untrusted repo gets no copied
    `settings.local.json` or `mirror` files (bash copied them, trust or
    not); a home's fetch happens while planning, before the agent check
  - `config add-repo` and the clones yo makes don't stream git's output;
    a failed clone says so in one line
  - a settings layer that can't be written stops the create, before the
    record and the launch, rather than launching without it
  - yo is a binary, from Homebrew or `cargo install`; `yo setup` does what
    `install.sh` did besides placing it, from copies built into yo (or
    links, `--link`), with no python3 (#34)
- **Fakes can't prove the AppleScript drives iTerm correctly.** A short manual
  checklist covered that, run once before the switch: start, start in a new
  window, reach with a message, title, down, resume-all.

What the tests cover, area by area, plus the error paths each command has
today (unknown flag, unknown repo, bad name, ambiguous match):

| area | cases |
| --- | --- |
| create | unnamed; a new branch; an existing branch, local and on origin only; a slashed name that isn't a branch; a PR URL; an issue URL; an issue in another repo; `--here` (cwd, a named repo, a profile's `dir`); `--worktree` over a `here` default; `--title`, `--model`, `--agent`, `--from`; `--new-window`, `--tab`; a profile's settings, memory and prefix; `mirror` and `mcp`; an unknown URL with `--add-to=profile` and `--add-to=guest` |
| `yo NAME` | each step of the lookup winning: repo, profile, agent, live Claude name, ledger name, id, id prefix, home (running, ready, none), new; ambiguity; a flag skipping it all |
| reach | running with and without a message; running but no tab found; stopped (resumes with the message) |
| ledger | `sessions`; `adopt` (new, already known, after `/clear`); `forget`; `title`, with and without a name; the hook for `SessionStart`, `SessionEnd` with `prompt_input_exit` and with other reasons |
| resume | by name, id, prefix, directory; a transcript-only id; a guest repo no longer registered; `resume-all` with and without `--dry-run` |
| down | in a worktree (clean, dirty, with unpushed commits; `--yes`, `--discard`, `--dry-run`); in place; an agent's home (refused) |
| sweep | each verdict (DEAD, STALE, SUSP, KEEP and its reasons); squash-merged; reused branch with an open PR; `--apply`, `--include-stale`, `--include-suspended`, `--delete-branches`, `--repo` |
| config | no config (single-repo mode); `show`, `path`, `add-repo` from a directory, a URL and `owner/repo`; unknown keys; env overrides beating the file |
| agents | `yo agent` bare, start, reach; an agent not merged on base |

Later phases extend the same tests: a fake `codex` (phase 2), and a fake ACP
agent that plays scripted `session/update` streams (phase 5), so the ACP
runner and the relay's delivery check can be tested without a model.

## How it's built

yo will be built and maintained mostly by coding agents. The compiler is the
first gate, but not the only one: every PR clears the same bar, whoever or
whatever wrote it. #11 sets this up before phase 1 writes any Rust.

Two rules shape it:

- **A check lives in the cheapest gate that can catch it.** Rules that can be
  checked mechanically become tests and lints, which are free and certain.
  Review agents cover what can't be, and the maintainer reviews last.
- **Everything CI runs can run locally.** CI is just another place to launch
  the same commands a developer or an agent runs on their machine. It holds
  no logic of its own.

### CI is a launcher

- **One entry point: `cargo xtask`.** It's the usual Rust pattern: a small
  binary crate in the workspace, run with `cargo`, so it needs no extra tool.
  `cargo xtask ci` runs every gate CI runs, in the same order, and each gate
  also has its own subcommand (`cargo xtask lint`, `cargo xtask arch`,
  `cargo xtask deny`, `cargo xtask test`). How to run
  them, and the tools they need, is in `docs/dev/env.md`.
- **Workflows are thin.** A GitHub Actions job checks out the code, installs
  the toolchain, sets the environment, and calls `cargo xtask <gate>`. If a
  step does more than that, it moves into `xtask`.
- **Configuration through the environment.** Anything that differs between a
  laptop and CI (a token, a cache directory, a flag to skip a slow check) is an
  environment variable, documented in one list in `docs/dev/`. CI sets them
  from its secrets; locally they come from your shell, or from a gitignored
  `.env` that `xtask` loads if it's there. Nothing in `xtask` asks which of the
  two it's running in.
- **The default gates need no secrets.** Build, lint, tests and `cargo-deny`
  run offline, with fakes, so `cargo xtask ci` works on a fresh clone.
  The one exception is cargo fetching crates. Anything else that needs a
  secret or the network is its own subcommand, and says which variables it
  needs when they're missing. So far that's `cargo xtask advisories`, which
  downloads the RustSec database. CI runs it on every PR and weekly.
- **Linux as well as macOS.** `osascript`, `ps` and the harnesses are faked,
  so the default gates don't need a Mac. CI runs only on Linux, because
  macOS runners cost too much, especially for Rust builds. `yo-app`
  stays an empty crate without Tauri until phase 6, so the workspace builds
  without WebKit's system libraries until then.
- **The review skills already run locally,** and so does setting the
  `yo-review` status. If they ever move into CI, CI calls the same skills.

### Gates, cheapest first

| gate | what it catches | runs |
| --- | --- | --- |
| the compiler, `cargo fmt`, `cargo clippy -D warnings` | type errors, unhandled cases, lint rules | `cargo xtask lint`: locally, and every PR in CI |
| architecture rules over `cargo metadata` | forbidden dependencies (below) | `cargo xtask arch`: locally, and every PR in CI |
| tests: unit, architecture, security | behavior, and the rules below | `cargo xtask test`: locally, and every PR in CI |
| `cargo-deny` | licenses, duplicate crates, new dependencies (`bans.allow` in `deny.toml` lists every crate allowed), sources | `cargo xtask deny`: locally, and every PR in CI |
| `cargo-deny` advisories | known vulnerabilities, yanked crates | `cargo xtask advisories`: needs the network; every PR in CI, and weekly |
| review skills (`review-pr`) | what can't be checked mechanically | by hand; sets the `yo-review` commit status |
| the maintainer | everything else, and any loosened gate | before merging |

`main` requires the CI checks and `yo-review` at the PR's head commit. Running
the review skills automatically in CI is left until the cost is clearly worth
it.

### Architecture rules as tests and lints

| rule | check |
| --- | --- |
| `yo-core` stays synchronous and light | `cargo xtask arch` fails if `yo-core` depends, other than for dev, on tokio, axum, the ACP crate, ratatui or Tauri, directly or not |
| the app holds no yo logic | the same check fails if `yo-app` depends on `yo-core`, `yo-local` or `yo-daemon`, directly or not; it may use only `yo-client` and `yo-proto` |
| the TUI and app don't care which yo API they have | the TUI's tests run against both `yo-local` and `yo-client` (with a test daemon), from phase 5 on |
| the core never prints | `clippy::print_stdout`, `clippy::print_stderr` and `clippy::dbg_macro` denied in `yo-core` |
| every outside effect goes through `Sys` | `clippy::disallowed-methods` (`crates/yo-core/clippy.toml`) bans `std::process::Command::new`, the system clock, randomness, new uuids, reading the environment and `home_dir` in `yo-core` outside `sys.rs` |
| the fake `Sys` is for tests | `cargo xtask arch` fails if a crate turns on `yo-core`'s `fake` feature other than as a dev-dependency |
| a dry run changes nothing | each changing command (all have `--dry-run`, through plans) has a test that runs it dry against the fake `Sys`: the fake records no changing call (for sweep and `down`, no worktree removal or branch deletion), and its temporary `HOME` is byte-identical afterwards. sweep and `down` also have one dry run against a real temporary repo, with its worktrees and branches compared afterwards |
| Claude-specific, not Claude-assumed | the command tests run with a fake harness that has no optional capabilities, as well as with Claude |

### Security tests

- **Quoting:** randomized tests that pass arbitrary strings through the
  single-quoting, run the result through `sh`, and require them to come back
  unchanged. Messages and titles reach AppleScript as arguments, never pasted
  into the script's text.
- **Paths:** session names and URL inputs can't escape the worktree root.
- **Destructive operations:** over randomized repo states, sweep and `down`
  never remove a dirty worktree, the primary clone, the current directory, an
  agent's home, or a worktree a session still holds. These run against real
  temporary git repos, not only the fake `Sys`, so they check how git itself
  reports dirty and unpushed work and what it refuses to remove.
- **The daemon:** requests with no token or a wrong one are refused, on the
  socket and over TCP; over TCP, so are requests with a bad `Origin` or
  `Host`. The socket and token file are `0600`. With default settings it opens
  no TCP port at all, and when TCP is turned on it never binds all interfaces.
- **The app:** it's granted only the Tauri commands the UI uses, and a test
  fails if that list grows without the test being updated.
- **The relay:** a fake ACP stream with a changed recipient or changed text is
  a failed delivery. A relayed message is never reported as your voice.
- **The hook:** random and malformed payloads never make it fail or hang.
- **The event log:** newlines and control characters in any text can't break
  the JSON lines format.

### Review skills

In the repo's own `.claude/skills/`, for working on yo. These are separate from
`claude/skills/`, which yo installs for its users. The checklists they apply
live as plain markdown in `docs/dev/`, so any harness can follow them, and
`AGENTS.md` (which `CLAUDE.md` points to) tells authoring agents which gates
to run before opening a PR.

- **`review-architecture`:** the diff against this doc's principles and rules.
  Is the doc, and its status table, still true?
- **`review-security`:** the points above, plus new outside commands, new
  places that accept input, and new dependencies.
- **`review-tests`:** tests at the right layer for each change, dry runs,
  nothing weakened.
- **`review-pr`** runs the three, each as a separate agent. An agent sees the
  diff, the code at that commit and this doc, but not the author's
  explanation (the PR description, commits, the issue), so it can't simply
  agree with it. The prompt keeps it to those, and the reviewer agent has
  only read tools, so it can't run `gh` or `git` to fetch them. It posts one PR comment with blocking and non-blocking
  findings (a few, past a bar: see `docs/dev/review.md`) and the commit it
  reviewed, and sets the `yo-review` status on that commit. A re-review
  covers only what changed since the last one. The findings are a second
  opinion: the author weighs them, and the maintainer decides.
- **The rules come from the base branch.** `review-pr` runs from a clean
  checkout of the branch the PR targets, never the PR's own branch, because
  the skill and the reviewer agent are loaded from the session's checkout.
  It reads the checklists, the area skills and `AGENTS.md` from the base
  too, so a PR can't be reviewed by its own edited reviewers. The code and
  this doc are read as the PR changes them.

`yo-review` records that review ran on that commit, so its state is always
`success`, and its description gives the counts of findings. It doesn't
prove the blocking findings were dealt with, and anyone who can run the
skill can set it. Checking the findings before merging stays the
maintainer's job. The rules for a review, for any harness, are in
`docs/dev/review.md`.

They focus on yo's own rules. General correctness is the built-in
`/code-review`'s job.

### Tuning the review skills

`tune-review-skills` runs at the end of a phase, or after a miss. It looks at
bugs found after merging that review missed, findings dismissed as wrong, and
changes to this doc since the last tuning, and proposes edits to the skills
as a PR.

- **Example cases for the reviewers.** Each review skill has planted-bad
  diffs it must flag and known-good diffs it must pass, as plain files beside
  the skill (`cases/`). They aren't a test suite: nothing runs them on every
  PR. `tune-review-skills` runs them once, on the skills it edits, because a
  reworded prompt can stop catching something without the diff showing it.
  Its PR reports the result. Every miss adds a case, and an edit lands only
  if every case still holds. When a case runs, the reviewer gets the rules
  without the cases, so it can't read the answer.
- **Misses are issues** labeled `review-miss`, and each tuning is logged in
  `docs/dev/review-tuning.md`, where the next one starts.
- **Only the maintainer can loosen a gate.** An edit that weakens a check,
  removes an example case (`cases/`), or relaxes a lint or `cargo-deny` rule needs the
  maintainer's explicit approval in that PR. Agents don't lower the bar
  they're measured against.

## Open questions

These stay open. Nothing in the design depends on how they're answered.

- **ACP v1 or v2.** v2 has `state_update`, `session/status` and resume with
  history replay, but is a preview. Decide in phase 5, against what
  `claude-agent-acp` and `codex-acp` support then.
- **Sharing a live session** between an ACP client and the harness's own TUI.
  ACP leaves it to each harness, and a proposal for clients joining a session
  another client started isn't settled.
- **Approvals in yo's UI:** for ACP sessions, showing and answering
  `session/request_permission` is possible. Whether yo should, or always send
  you to the harness's TUI, is for phase 5 or 6.
- **The relay inside ACP:** whether a `claude-agent-acp` session can use
  `SendMessage` (see [the relay](#the-relay)). Checked first thing in phase 5.
- **The relay's reach:** whether cross-machine messages to sessions that
  aren't connected over Remote Control are useful enough to need anything
  more, and how replies with no reply address show up.
- **The hub:** self-hosted, on a home machine, or hosted. Phase 7.
- **An MCP server:** coordinating sessions calling yo as tools rather than
  shelling out. It would sit on the yo API, like the TUI.
