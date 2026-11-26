# yo core: design

## Status

> **None of this is built yet.** This is a design for review. The `yo` that
> runs today is the bash script, `scripts/yo`, and the [guide](../guide.md)
> and [reference](../reference.md) describe it. Nothing below, including the
> Rust core, harnesses other than Claude, ACP sessions, the relay, the daemon,
> `yo status`, `yo tui` and `yo app`, exists until its phase lands.

| phase | what | status |
| --- | --- | --- |
| 0 | this design | done (#8, PR #10) |
| before 1 | the gates: CI, lints, review skills, skill tuning | in progress (#11): workspace, `xtask`, CI, `AGENTS.md` and checklists done |
| 1 | Rust core with CLI parity | not started |
| 2 | a second harness: Codex | not started |
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
says how yo stays open to coding agents other than Claude Code, and lays out how
the port is proven to behave like today's `yo`. No code comes with it; phase 1
builds against it.

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
- [Parity test plan](#parity-test-plan)
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
| parity | the port must have today's *effects* exactly (ledger, config, git, files written, commands launched). Output text is compared too, but may change deliberately: every difference is recorded |
| daemon | optional for the CLI and the TUI; started on demand by what needs it (`yo app`, background sessions, the relay). Listens only on a Unix socket unless TCP is turned on. A launchd agent can come later |
| how it's built | gates cheapest first, everything CI runs can run locally (`cargo xtask`), review skills by hand, and only the maintainer loosens a gate; see [how it's built](#how-its-built) |
| "latest focus" | raw: the last prompt and the last tool used. A summary can come later |
| phone | Claude's Remote Control for Claude sessions, for now; for every harness later, the same web UI over the daemon's opt-in HTTP, on a tailnet |
| iTerm automation | keep shelling out to `osascript` |
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
  needs an Apple Developer ID and notarization on each release; that comes
  with the distribution question (see [open questions](#open-questions)).
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
| `yo-testkit` | lib, dev | sandboxes, fakes and the parity harness |
| `xtask` | bin, dev | the gates, run the same way locally and in CI (`cargo xtask ci`); see [how it's built](#how-its-built) |
| `web/` | not Rust | the web UI: inside `yo-app`, and served by the daemon's opt-in HTTP |

Inside `yo-core`:

```
yo-core/src/
  sys.rs        Sys: clock, ids, rng, process table, command runner, env, paths
  paths.rs      XDG dirs, config path, state dir, worktree roots
  config/       the config file: types (serde, deny_unknown_fields), load, validate, write (add-repo)
  registry/     resolved repos, profiles, agents, homes; precedence (flag > repo > profile > defaults, env beats file)
  git.rs        the git CLI: worktrees, branches, remotes, status, default branch
  github.rs     the gh CLI: PR and issue titles, PR listings for sweep; URL parsing
  ledger/       session records: load, put, adopt, drop, retitle, sync, the hook
  resolve.rs    `yo NAME`: the ordered lookup
  input.rs      INPUT -> what to create: unnamed, PR, issue, existing branch, new branch
  create.rs     plan a new session: worktree, branch, title, codename, brief, message
  resume.rs     resume one, resume-all
  down.rs       tear down the current session
  sweep.rs      classify worktrees; remove
  naming.rs     codenames, titles, truncation
  plan.rs       Plan and Action: what a command will do, before it does it
  events.rs     event types, append, read, tail
  status.rs     the status model: a fold over records, liveness and events
  harness/      the Harness trait; claude/ (settings layer, trust, memory, hooks, agents), codex/
  runner/       the Runner trait; iterm.rs. The ACP runner's client side lives here; its processes live in the daemon
  message.rs    delivering a message: choosing a method, and the result
```

## The core's API

These signatures are the shape, not the final code: phase 1 may rename things.

### Effects go through `Sys`

Everything that's nondeterministic or touches the outside goes through one
trait object. The real one does what bash does today; tests use fakes. This
is what makes the suite possible.

```rust
pub trait Sys: Send + Sync {
    fn now(&self) -> SystemTime;
    fn new_session_id(&self) -> Uuid;                // today: uuidgen
    fn pick(&self, n: usize) -> usize;               // codenames; today: $RANDOM
    fn env(&self, key: &str) -> Option<String>;
    fn home(&self) -> PathBuf;
    fn cmd(&self) -> &dyn CommandRunner;             // git, gh, osascript, ps, claude, codex
    fn procs(&self) -> &dyn ProcessTable;            // is pid alive, its comm, its tty
    fn hostname(&self) -> String;
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output>;   // program, args, cwd, env, stdin
}
```

Files are touched with plain `std::fs` under paths the `Sys` gives (`home`, and
XDG variables via `env`), so a throwaway `HOME` sandboxes them, as today.

### Context and config

```rust
pub struct Ctx {
    pub sys: Arc<dyn Sys>,
    pub paths: Paths,           // config path, state dir, settings dir, ...
    pub machine: MachineId,
    pub registry: Registry,     // config resolved: repos, profiles, agents, harnesses
}

impl Ctx {
    /// Soft load: a missing config is single-repo mode against the cwd, as today.
    pub fn load(sys: Arc<dyn Sys>, cwd: &Path) -> Result<Ctx>;
}

pub struct Config {            // the JSON file, as today, plus harness settings
    pub defaults: Defaults,
    pub profiles: BTreeMap<String, ProfileCfg>,
    pub repos: BTreeMap<String, RepoCfg>,
    pub repos_in: Vec<PathBuf>,
    pub agents: BTreeMap<String, AgentCfg>,
    pub harnesses: BTreeMap<String, HarnessCfg>,   // new, optional: command, extra args per harness
}

impl Registry {
    pub fn repo(&self, key: &str) -> Option<&Repo>;
    pub fn repo_at(&self, dir: &Path) -> Option<&Repo>;        // detect from the cwd
    pub fn repo_for_github(&self, owner_repo: &str) -> Option<&Repo>;
    pub fn profile(&self, name: &str) -> Option<&Profile>;
    pub fn profile_for(&self, repo: &Repo) -> Option<&Profile>;
    pub fn agent(&self, name: &str) -> Option<&Agent>;
    pub fn home_repo(&self, name: &str) -> Option<&Repo>;      // NAME-home
    pub fn harness(&self, name: &str) -> Option<&dyn Harness>;
    pub fn setting<T>(&self, s: Setting<T>, repo: Option<&Repo>, flag: Option<T>) -> T;
}
```

`Registry::setting` is the one place precedence lives: flag, then repo, then
profile, then `defaults`, with a setting's environment variable beating the
file. Today that's spread over `*_for` functions.

A new setting, `harness` (default `claude`), sits at every level like `mode`
does, with a `--harness=H` flag. Today's `claude` setting stays, as the
command for the `claude` harness; `harnesses.<name>.command` is the same for
any harness.

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
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,   // unknown keys survive a rewrite
}

pub struct Ledger { /* dir */ }

impl Ledger {
    pub fn all(&self) -> Vec<Entry>;                          // Entry = key + record
    pub fn put(&self, rec: Record) -> Result<Key>;
    pub fn adopt(&self, a: Adopt) -> Result<Key>;
    pub fn drop(&self, key: &Key) -> Result<()>;
    pub fn drop_dir(&self, worktree: &Path) -> Result<()>;
    pub fn retitle(&self, by: RetitleBy, title: &str) -> Result<Option<Key>>;
    pub fn learn_id(&self, key: &Key, session_id: &str) -> Result<()>;  // a harness that names its own ids
    pub fn sync(&self, repos: &[&Repo], live: &[LiveSession]) -> Result<Vec<Key>>;
    pub fn hook(&self, payload: &HookPayload) -> Result<()>;  // never fails the caller
    pub fn get(&self, q: &str, live: &[LiveSession]) -> Result<Found>; // id, 8+ prefix, dir, name; transcript fallback
}

pub enum State { Running { pid: Option<u32> }, Ready, Ended, Gone, NoId }

pub fn state_of(rec: &Record, live: &[LiveSession]) -> State;   // today's status_of
```

Writes go to a temporary file followed by a rename, as today. A record goes
back to the file it came from, even when its session id has changed (after a
`/clear`).

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
pub fn plan_reach(ctx: &Ctx, name: &str, message: Option<&str>) -> Result<Plan>;
pub fn plan_resume(ctx: &Ctx, q: &str, opts: ResumeOpts) -> Result<Plan>;
pub fn plan_resume_all(ctx: &Ctx, opts: ResumeOpts) -> Result<Plan>;
pub fn plan_down(ctx: &Ctx, here: &Path, opts: DownOpts) -> Result<Plan>;
pub fn plan_sweep(ctx: &Ctx, opts: SweepOpts) -> Result<(Vec<Verdict>, Plan)>;

pub fn execute(ctx: &Ctx, plan: &Plan, runners: &Runners, out: &mut dyn Report) -> Result<()>;
```

`Report` is where output goes: the CLI renders it as today's text; the daemon
turns it into events. The core never prints.

### `yo NAME`

Resolution is a pure function over the registry, the ledger and the live
sessions. It returns what NAME means; the caller acts on it.

```rust
pub enum Resolved {
    RepoOrProfile,                          // step 1: create, positionally
    Agent(String),                          // step 2
    Live(LiveSession),                      // step 3: by the harness's own name for it
    Ledger(Entry, State),                   // step 4: by yo name, id, 8+ prefix
    Home { repo: String, target: HomeTarget }, // step 5: running, else latest ready, else new in place
    New,                                    // step 6
}

pub fn resolve(ctx: &Ctx, name: &str, has_flags: bool, live: &[LiveSession]) -> Result<Resolved, Ambiguous>;
```

Today this order is spread over `main`, `cmd_reach` and `cmd_home_reach`.
Having it in one function, tested case by case, is one of the main things the
port buys.

## Harnesses

A harness is a coding agent's CLI: `claude`, `codex`, and later others. Most of
what's Claude-specific in yo today is in how it prepares, launches and watches
a Claude session, not in runners or sessions. All of that moves behind one
trait.

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
    pub first_message: Option<String>,
    pub permission_mode: Option<String>,
}
```

- **The Claude harness** owns what's Claude-specific in yo today:
  `--session-id`, `--settings` and the composed settings layer, the trust
  seeding in `~/.claude.json`, each worktree's `.claude/settings.local.json`
  (MCP pre-approval), shared memory, `--agent` and agents' homes, the ledger
  hooks, Remote Control names, live sessions from `~/.claude/sessions`, status
  from `claude agents --json`. It also starts passing `--name <yo name>`, so
  other sessions (and the relay) can address it by yo's name.
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
  tty. Works for any harness with a TUI.
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
`SessionStart` and `SessionEnd` (today's ledger updates), so the composed
settings compare exactly in the parity tests. Phase 3 renames it `yo _hook`,
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
   tests ([below](#parity-test-plan)). One harness (Claude) and one runner
   (iTerm), but both already behind their traits. `--name` is passed at launch.
2. **A second harness:** Codex, in iTerm tabs. Launch, resume, the ledger
   with ids learned late, `--harness`, messages by typing. This is what proves
   the `Harness` trait.
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

## Parity test plan

The goal: prove the Rust `yo` does what `scripts/yo` does. Same effects
exactly; output the same or deliberately different.

### The harness

`yo-testkit` runs each scenario twice, in two sandboxes that start
identical: once with `scripts/yo` (the reference), once with the new binary.
Then it compares the results. ("Harness" here means the test harness, not a
coding agent.)

A sandbox is a temporary directory with:

| piece | how |
| --- | --- |
| `HOME` | `<sandbox>/home`, so `~/.claude.json`, `~/.claude/sessions`, `~/Library/...` and memory land there |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `YO_CONFIG` | inside the sandbox; `YO_CONFIG` points at the scenario's config fixture |
| `YO_DRY_LAUNCH` | unset, so the real launch path runs against the fakes below; set only for scenarios about dry launch itself |
| git repos | built by the fixture: a bare repo as `origin` (and one as `upstream`, for forks), a clone, branches, merged and unmerged work |
| remotes | `GIT_CONFIG_GLOBAL` with `url.<file path>.insteadOf` rules, so `git@github.com:owner/repo` and `https://github.com/...` URLs resolve to the local bare repos; nothing touches the network |
| `PATH` | a shim directory first: fake `gh`, `osascript`, `ps`, `uuidgen`, `date` and `claude` (which later also answers `claude agents --json` from fixtures) |
| live sessions | a long-running dummy process named `claude` (a copied `sleep`), with a matching `~/.claude/sessions/<pid>.json`, so the `kill -0` and `ps -o comm=` checks pass |

The shims record each call (argv, stdin, cwd) to a log and answer from the
scenario's fixtures:

- **`gh`:** canned JSON for PR and issue titles and PR listings.
- **`osascript`:** records the AppleScript it was given; answers `found` or
  `notfound`, a screen size, a tab title.
- **`ps`:** a tty for the dummy `claude`, so "bring the tab forward" can be
  tested.
- **`uuidgen`, `date`:** fixed sequences.

### Two small changes to the bash script first

Today's script calls `/usr/bin/osascript` by absolute path, and picks
codenames with `$RANDOM`. Neither can be faked from outside. Before the
harness can run, the bash version needs:

1. `YO_OSASCRIPT`, defaulting to `/usr/bin/osascript`, used at every call site.
2. `YO_CODENAME`, which when set is used instead of a random codename.

There's also `time.time()` in the python helpers. Rather than faking that,
the comparison normalizes timestamps (below).

### What's compared

| what | how |
| --- | --- |
| exit code | exactly |
| ledger | every record, as JSON, with `createdAt`, `updatedAt` and `endedAt` replaced by `<ts>`, and the new `machine`, `harness` and `runner` fields ignored |
| config | the file, byte for byte (`yo config add-repo` writes it) |
| composed settings | `$XDG_STATE_HOME/yo/settings/*.json`, as JSON, with yo's own path in the hook command normalized (the script's path in one run, the binary's in the other) |
| Claude's files | `~/.claude.json` (trust), each worktree's `.claude/settings.local.json`, mirrored files |
| iTerm | the dynamic profile JSON, and the AppleScript sent to `osascript`, normalized for whitespace |
| git | `git worktree list --porcelain` and every branch with its commit, for each fixture repo |
| launched commands | the `claude` command line typed into the tab (from the `osascript` log), with the session id normalized, and `--name` (new) accepted as a difference |
| stdout, stderr | after normalizing ids, timestamps and sandbox paths |

The dates in session names (`sesh_<date>_<time>`) are fixed by the `date`
shim and the Rust version's fake clock, so names compare exactly.

### Accepted differences

When the port deliberately changes output, the scenario records the
difference in an `accepted.diff` file beside it. The test fails on any
difference that isn't recorded, and on any recorded one that no longer
happens. Every change to yo's output is then a choice someone made, in a
diff someone reviewed. Effects (everything in the table but stdout and stderr)
have no accepted differences, beyond the new fields and `--name` noted above:
they must match.

### Scenarios

At least one per row, plus the error paths each command has today (unknown
flag, unknown repo, bad name, ambiguous match):

| area | scenarios |
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

### After the switch

The bash script is retired at the end of phase 1. Before it goes, the harness
records each scenario's reference results as fixtures (`tests/golden/`), and
from then on the Rust version runs against those. The same scenarios stay
the regression suite, and `yo-core` has its own unit tests for resolution,
precedence, sweep's rules and status.

Later phases extend the same suite rather than starting new ones: a fake
`codex` (phase 2), and a fake ACP agent that plays scripted `session/update`
streams (phase 5), so the ACP runner and the relay's delivery check can be
tested without a model.

Fakes can't prove the AppleScript drives iTerm correctly. A short manual
checklist covers that, run once before the switch: start, start in a new
window, reach with a message, title, down, resume-all.

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
  `cargo xtask deny`, `cargo xtask test`, `cargo xtask parity`). How to run
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
- **The default gates need no secrets.** Build, lint, tests, `cargo-deny` and
  parity run offline, with fakes, so `cargo xtask ci` works on a fresh clone.
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
| tests: unit, parity, architecture, security | behavior, and the rules below | `cargo xtask test`, `cargo xtask parity`: locally, and every PR in CI |
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
| a dry run changes nothing | every scenario also runs against the Rust version with `--dry-run` (which every changing command has there, through plans), and the sandbox must be byte-identical afterwards |
| Claude-specific, not Claude-assumed | a fake harness with no optional capabilities runs the whole scenario suite |

### Security tests

- **Quoting:** randomized tests that pass arbitrary strings through the
  single-quoting, run the result through `sh`, and require them to come back
  unchanged. Messages and titles reach AppleScript as arguments, never pasted
  into the script's text.
- **Paths:** session names and URL inputs can't escape the worktree root.
- **Destructive operations:** over randomized repo states, sweep and `down`
  never remove a dirty worktree, the primary clone, the current directory, an
  agent's home, or a worktree a session still holds.
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
- **`review-tests`:** tests at the right layer for each change; parity
  scenarios and `accepted.diff` entries justified.
- **`review-pr`** runs the three, each as a separate agent that sees only the
  diff and this doc, not the author's explanation, so it can't simply agree
  with it. It posts one PR comment with blocking and non-blocking findings and
  the commit it reviewed, and sets the `yo-review` status on that commit.

`yo-review` records that review ran on that commit. It doesn't prove the
blocking findings were dealt with, and anyone who can run the skill can set
it. Checking the findings before merging stays the maintainer's job.

They focus on yo's own rules. General correctness is the built-in
`/code-review`'s job.

### Tuning the review skills

`tune-review-skills` runs at the end of a phase, or after a miss. It looks at
bugs found after merging that review missed, findings dismissed as wrong, and
changes to this doc since the last tuning, and proposes edits to the skills
as a PR.

- **Test cases for the reviewers.** Each review skill has planted-bad diffs it
  must flag and known-good diffs it must pass. Every miss adds a case, and an
  edit lands only if every case still holds.
- **Only the maintainer can loosen a gate.** An edit that weakens a check,
  removes a test case, or relaxes a lint or `cargo-deny` rule needs the
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
- **Distribution:** stay "clone and `install.sh`" (which would then run
  `cargo build`), release binaries, or a Homebrew tap.
- **An MCP server:** coordinating sessions calling yo as tools rather than
  shelling out. It would sit on the yo API, like the TUI.
