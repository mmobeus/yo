# yo core: design

Phase 0 of moving yo from a bash script to a core with a CLI, a TUI, an app and
pluggable runners. This doc settles the language, sketches the core and its API,
and lays out how the port is proven to behave like today's `yo`. No code comes
with it; phase 1 builds against it.

- [Decisions](#decisions)
- [Language: Rust](#language-rust)
- [Shape](#shape)
- [Crates](#crates)
- [The core's API](#the-cores-api)
- [Machine ids](#machine-ids)
- [Events and status](#events-and-status)
- [Runners](#runners)
- [The daemon](#the-daemon)
- [Parity test plan](#parity-test-plan)
- [Open questions](#open-questions)

## Decisions

| question | decision |
| --- | --- |
| language | Rust, for the core, CLI, daemon and TUI |
| app shell | none at first: the daemon serves the web UI to a browser (and later a phone). A thin Tauri shell over the same UI if a real window is wanted. Not Wails |
| compatibility | yo has one user and isn't public yet, so there is no migration and no backward compatibility to keep. Config and ledger formats stay as they are unless there's a reason to change them; the one change planned is a `machine` field in ledger records |
| parity | the port must have today's *effects* exactly (ledger, config, git, files written, commands launched). Output text is compared too, but may change deliberately: every difference is recorded |
| daemon | optional for the CLI; started on demand by `yo tui` and `yo app`. A launchd agent can come later |
| "latest focus" | raw: the last prompt and the last tool used. A summary can come later |
| iTerm automation | keep shelling out to `osascript` |
| git, gh | keep shelling out to the `git` and `gh` CLIs, not a git library: same behavior as today, same auth, same config |

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
  the daemon's API and `claude`'s stream-json messages are all serde types.
  Later phases, often built by separate sessions, build against those types
  rather than against prose.
- **Ceremony is cheap when an agent writes it.** Rust's extra code (error
  enums, ownership plumbing) mostly costs typing, and the typing isn't done by
  hand.
- **Few human readers**, even once public, which takes away Go's main
  remaining advantage: that newcomers can read and contribute easily.
- **Tauri** is native, if the app ever gets a window of its own.

### What Tauri's security does and doesn't buy

Tauri's security model guards the boundary between the web UI and native code:
which native commands a page may call, and what each may touch. In this design
the UI does almost nothing natively. It talks to the daemon over HTTP, and the
daemon does the work. So the boundary that needs securing is **the daemon's
API**, whatever wraps the UI (see [the daemon](#the-daemon)). A Tauri shell
adds defense in depth on top of that; it doesn't replace it.

### Keeping Rust's costs down

- **A synchronous core.** Config, the ledger, name resolution, sweep's rules,
  git calls and the iTerm runner don't need async. Only the daemon and the
  headless runner use tokio. The CLI stays plain and starts fast, which
  matters for the ledger hook that runs at every `claude` start and exit.
- **Simple ownership.** Owned data, `Arc` where something is shared, and one
  task that owns the daemon's session table and takes messages. No
  long-lived borrowed references in the API.
- **Fast incremental builds:** a workspace of several crates, with the heavy
  dependencies (tokio, axum, ratatui) kept out of `yo-core`.
- **One bar to clear:** `cargo fmt --check`, `cargo clippy -- -D warnings` and
  the tests (`cargo nextest`), in CI and in every session's loop.

## Shape

```
     yo (CLI)       yo tui        yo app / phone browser     [MCP server?]
         \             |                  |                       /
          \            +---- yo-client ---+-----------------------+
           \                    |  (HTTP + WebSocket, local or remote)
            \             yo serve (daemon, async)
             \                  |
              +------------ yo-core (sync) ---------------------------+
              | config · registry (repos, profiles, agents, homes)    |
              | ledger · resolve (`yo NAME`) · create · resume · down |
              | sweep · settings composition · events · status        |
              | runners: iterm (now) · headless · remote control · remote |
              | sys: clock, ids, processes, fs roots (fakeable)       |
              +-------------------------------------------------------+
```

- The CLI calls `yo-core` directly and works with no daemon running, as
  today. When a daemon is running, the CLI can also ask it for things only it
  knows (headless sessions, live status).
- The TUI and app talk only to a daemon, through `yo-client`, and never
  assume it's local.
- There is one binary, `yo`, with `yo serve` and `yo tui` among its
  subcommands. No `--mode=` flag: `mode` already means in place vs worktree.

## Crates

A Cargo workspace:

| crate | kind | what it holds |
| --- | --- | --- |
| `yo-core` | lib, sync | everything that's site logic today; see [the core's API](#the-cores-api) |
| `yo-cli` | bin `yo` | clap commands, flags, output rendering, prompts. Thin over `yo-core` |
| `yo-proto` | lib | the daemon's API types (requests, responses, the event stream), shared by daemon and clients |
| `yo-client` | lib | a client for a daemon, local (Unix socket) or remote (HTTPS), used by the TUI, app and CLI |
| `yo-daemon` | lib, async | `yo serve`: the API, the event tailer, the headless runner's processes |
| `yo-tui` | lib | `yo tui` (phase 3) |
| `yo-testkit` | lib, dev | sandboxes, fakes and the parity harness |
| `web/` | not Rust | the web UI (phase 5) |

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
  live.rs       live claude processes, from ~/.claude/sessions and the process table
  resolve.rs    `yo NAME`: the ordered lookup
  input.rs      INPUT -> what to create: unnamed, PR, issue, existing branch, new branch
  create.rs     plan a new session: worktree, branch, title, codename, brief, message, settings
  resume.rs     resume one, resume-all
  down.rs       tear down the current session
  sweep.rs      classify worktrees; remove
  settings.rs   compose the per-session settings layer; trust seeding; mirroring; the iTerm profile
  naming.rs     codenames, titles, truncation
  plan.rs       Plan and Action: what a command will do, before it does it
  events.rs     event types, append, read, tail
  status.rs     the status model: a fold over records, liveness and events
  runner/       the Runner trait; iterm.rs (now), headless, remote control and remote come later
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
    fn cmd(&self) -> &dyn CommandRunner;             // git, gh, osascript, ps
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
    pub registry: Registry,     // config resolved: repos, profiles, agents
}

impl Ctx {
    /// Soft load: a missing config is single-repo mode against the cwd, as today.
    pub fn load(sys: Arc<dyn Sys>, cwd: &Path) -> Result<Ctx>;
}

pub struct Config {            // the JSON file, unchanged
    pub defaults: Defaults,
    pub profiles: BTreeMap<String, ProfileCfg>,
    pub repos: BTreeMap<String, RepoCfg>,
    pub repos_in: Vec<PathBuf>,
    pub agents: BTreeMap<String, AgentCfg>,
}

impl Registry {
    pub fn repo(&self, key: &str) -> Option<&Repo>;
    pub fn repo_at(&self, dir: &Path) -> Option<&Repo>;        // detect from the cwd
    pub fn repo_for_github(&self, owner_repo: &str) -> Option<&Repo>;
    pub fn profile(&self, name: &str) -> Option<&Profile>;
    pub fn profile_for(&self, repo: &Repo) -> Option<&Profile>;
    pub fn agent(&self, name: &str) -> Option<&Agent>;
    pub fn home_repo(&self, name: &str) -> Option<&Repo>;      // NAME-home
    pub fn setting<T>(&self, s: Setting<T>, repo: Option<&Repo>, flag: Option<T>) -> T;
}
```

`Registry::setting` is the one place precedence lives: flag, then repo, then
profile, then `defaults`, with a setting's environment variable beating the
file. Today that's spread over `*_for` functions.

### The ledger

The record keeps today's fields and adds `machine`. The file layout stays: one
JSON file per session in `$XDG_STATE_HOME/yo/sessions/`, keyed by the worktree
path.

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub worktree: PathBuf,
    pub session_id: Option<Uuid>,
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
    pub fn sync(&self, repos: &[&Repo], live: &Live) -> Result<Vec<Key>>;
    pub fn hook(&self, payload: &HookPayload) -> Result<()>;  // never fails the caller
    pub fn get(&self, q: &str, live: &Live) -> Result<Found>; // id, 8+ prefix, dir, name; transcript fallback
}

pub enum State { Running { pid: u32 }, Ready, Ended, Gone, NoId }

pub fn state_of(rec: &Record, live: &Live) -> State;          // today's status_of
```

Writes go to a temporary file followed by a rename, as today. A record goes
back to the file it came from, even when its session id has changed (after a
`/clear`).

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
    WriteLocalSettings { worktree: PathBuf, mcp: Vec<String> },
    SeedTrust { dirs: Vec<PathBuf> },                 // ~/.claude.json
    ComposeSettings { out: PathBuf, layers: Vec<PathBuf>, memory: Option<PathBuf> },
    PutRecord(Record),
    DropRecord(Key),
    Launch(LaunchSpec),
    Reach { target: LiveRef, message: Option<String> },
    CloseOwnTab,
}

pub fn plan_create(ctx: &Ctx, req: CreateRequest) -> Result<Plan>;
pub fn plan_reach(ctx: &Ctx, name: &str, message: Option<&str>) -> Result<Plan>;
pub fn plan_resume(ctx: &Ctx, q: &str, opts: ResumeOpts) -> Result<Plan>;
pub fn plan_resume_all(ctx: &Ctx, opts: ResumeOpts) -> Result<Plan>;
pub fn plan_down(ctx: &Ctx, here: &Path, opts: DownOpts) -> Result<Plan>;
pub fn plan_sweep(ctx: &Ctx, opts: SweepOpts) -> Result<(Vec<Verdict>, Plan)>;

pub fn execute(ctx: &Ctx, plan: &Plan, runner: &dyn Runner, out: &mut dyn Report) -> Result<()>;
```

`Report` is where output goes: the CLI renders it as today's text; the daemon
turns it into events. The core never prints.

### `yo NAME`

Resolution is a pure function over the registry, the ledger and the live
processes. It returns what NAME means; the caller acts on it.

```rust
pub enum Resolved {
    RepoOrProfile,                          // step 1: create, positionally
    Agent(String),                          // step 2
    Live(LiveSession),                      // step 3: by Claude name
    Ledger(Entry, State),                   // step 4: by yo name, id, 8+ prefix
    Home { repo: String, target: HomeTarget }, // step 5: running, else latest ready, else new in place
    New,                                    // step 6
}

pub fn resolve(ctx: &Ctx, name: &str, has_flags: bool, live: &Live) -> Result<Resolved, Ambiguous>;
```

Today this order is spread over `main`, `cmd_reach` and `cmd_home_reach`.
Having it in one function, tested case by case, is one of the main things the
port buys.

## Machine ids

Design for several machines now, build it in phase 6.

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
  session id alone is unique, but not every record has one (`no-id`).
- **What doesn't need it:** config. Each machine has its own config file;
  the hub will learn which machine can run which repo from each daemon's
  registry.

## Events and status

Phase 2 builds this, but the types live in `yo-core` from phase 1 so the
ledger hook already writes through them.

### The event log

JSON lines in `$XDG_STATE_HOME/yo/events/YYYY-MM-DD.jsonl`, one file per day,
so old days can be pruned. Hooks append with `O_APPEND` under an advisory lock
(`flock`), one line per event, kept small.

```rust
#[derive(Serialize, Deserialize)]
pub struct Event {
    pub v: u32,                      // schema version
    pub ts: i64,                     // unix ms
    pub machine: MachineId,
    pub session: Option<Uuid>,       // claude's session id
    pub key: Option<Key>,            // ledger key, when known (YO_SESSION_KEY)
    #[serde(flatten)]
    pub kind: EventKind,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    SessionStart { cwd: PathBuf, source: String },
    SessionEnd { reason: String },
    Prompt { text: String },                     // UserPromptSubmit, truncated
    ToolUsed { tool: String, summary: String },  // PostToolUse
    Notification { message: String },            // permission or input wanted
    Stop,                                        // turn done, waiting
    Usage { cost_usd: f64, context_pct: f32 },   // from the statusline
    Yo { action: String, detail: Value },        // created, resumed, reached, down, swept
}
```

The hook entry point becomes `yo _hook`, reading the event name from the
payload. Phase 1 handles `SessionStart` and `SessionEnd` (today's ledger
updates). Phase 2 adds `UserPromptSubmit`, `PostToolUse`, `Notification` and
`Stop` to yo's settings layer. The statusline script already sees cost and
context; it can append a `Usage` event every so often.

### Status

Status is derived, never stored: a fold over ledger records, liveness and
events.

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
    pub runner: RunnerKind,
    pub repo: Option<String>, pub profile: Option<String>, pub agent: Option<String>,
    pub branch: Option<String>, pub pr: Option<u64>,
    pub usage: Option<Usage>,
    pub last_activity: Option<i64>,
}

impl SessionStatus {
    pub fn apply(&mut self, ev: &Event);   // pure; the TUI, app and `yo status` all use it
}
```

`needs_me` is set by `Notification` and `Stop`, and cleared by the next
`Prompt` or `ToolUsed`.

## Runners

A session is a record in yo; a runner is how it runs. Every runner implements
the same synchronous trait:

```rust
pub trait Runner {
    fn kind(&self) -> RunnerKind;
    fn launch(&self, spec: &LaunchSpec) -> Result<Launched>;
    fn locate(&self, live: &LiveSession) -> Option<Location>;  // where it is, if this runner can tell
    fn send(&self, at: &Location, message: &str) -> Result<Delivery>;
    fn focus(&self, at: &Location) -> Result<()>;
    fn close(&self, at: &Location) -> Result<()>;
    fn retitle(&self, at: &Location, title: &str) -> Result<()>;
}

pub struct LaunchSpec {
    pub dir: PathBuf,
    pub title: String,
    pub claude: ClaudeCmd,           // argv, not a shell string: program, args, env, session id, model, agent, settings, first message
    pub window: WindowChoice,        // tab | window | config
    pub group: String,
}

pub enum Delivery { Delivered, NotFound { hint: String } }
```

- **Liveness isn't a runner's business.** Claude's own registry
  (`~/.claude/sessions/*.json`) and the process table say what's running, for
  every runner. A runner only answers where a live session is and how to reach
  it.
- **`ClaudeCmd` is argv, not a shell string.** The iTerm runner turns it into
  the single-quoted command line it types today. The headless runner execs it
  directly.
- **iTerm (phase 1):** today's AppleScript, kept as `.applescript` files
  embedded with `include_str!` and run through `CommandRunner` (so tests see
  them). Locating a session is by its tty. Sending a message keeps the
  type-pause-Enter sequence.
- **Headless (phase 4)** lives in `yo-daemon`, because something has to own
  the child process. It runs `claude --print --input-format stream-json
  --output-format stream-json`, sends messages as JSON on stdin, and turns
  the output stream into events. The CLI reaches it through a `DaemonRunner`
  that implements the same trait over `yo-client`.
- **Moving between runners** is `claude --resume <id>` under the other
  runner. The transcript doesn't care which runner it's under.
- **Remote Control and remote workers** are open questions (below). The trait
  makes no assumption of a tab or a tty, so neither is ruled out.

## The daemon

`yo serve`, phase 5, sketched here so the core doesn't paint it into a corner.

- **Started on demand** by `yo tui` and `yo app`, and left running. One per
  machine: a pid file and a lock in the state directory.
- **API:** versioned JSON over HTTP (`/v1/...`), with a WebSocket for the
  event stream. Mostly the core's plan and execute functions, made remote.
  The request and response types live in `yo-proto`.
- **Security,** the boundary that matters in this design:
  - a Unix socket (mode `0600`) by default; TCP only when asked, and then
    bound to localhost or a tailnet address
  - a bearer token in `$XDG_STATE_HOME/yo/daemon-token` (mode `0600`) on
    every request
  - the `Origin` and `Host` headers checked on every browser request, so a
    web page can't call a localhost daemon from your browser
  - the phone and the hub need real authentication on top (phase 6)
- **The web UI** is static files the daemon serves. A browser, an installed
  web app or a phone use it as is; a Tauri shell is optional.

## Parity test plan

The goal: prove the Rust `yo` does what `scripts/yo` does. Same effects
exactly; output the same or deliberately different.

### The harness

`yo-testkit` runs each scenario twice, in two sandboxes that start
identical: once with `scripts/yo` (the reference), once with the new binary.
Then it compares the results.

A sandbox is a temporary directory with:

| piece | how |
| --- | --- |
| `HOME` | `<sandbox>/home`, so `~/.claude.json`, `~/.claude/sessions`, `~/Library/...` and memory land there |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `YO_CONFIG` | inside the sandbox; `YO_CONFIG` points at the scenario's config fixture |
| `YO_DRY_LAUNCH` | unset, so the real launch path runs against the fakes below; set only for scenarios about dry launch itself |
| git repos | built by the fixture: a bare repo as `origin` (and one as `upstream`, for forks), a clone, branches, merged and unmerged work |
| remotes | `GIT_CONFIG_GLOBAL` with `url.<file path>.insteadOf` rules, so `git@github.com:owner/repo` and `https://github.com/...` URLs resolve to the local bare repos; nothing touches the network |
| `PATH` | a shim directory first: fake `gh`, `osascript`, `ps`, `uuidgen`, `date` and `claude` |
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
| ledger | every record, as JSON, with `createdAt`, `updatedAt` and `endedAt` replaced by `<ts>` (and the new `machine` field ignored) |
| config | the file, byte for byte (`yo config add-repo` writes it) |
| composed settings | `$XDG_STATE_HOME/yo/settings/*.json`, as JSON |
| Claude's files | `~/.claude.json` (trust), each worktree's `.claude/settings.local.json`, mirrored files |
| iTerm | the dynamic profile JSON, and the AppleScript sent to `osascript`, normalized for whitespace |
| git | `git worktree list --porcelain` and every branch with its commit, for each fixture repo |
| launched commands | the `claude` command line typed into the tab (from the `osascript` log), with the session id normalized |
| stdout, stderr | after normalizing ids, timestamps and sandbox paths |

The dates in session names (`sesh_<date>_<time>`) are fixed by the `date`
shim and the Rust version's fake clock, so names compare exactly.

### Accepted differences

When the port deliberately changes output, the scenario records the
difference in an `accepted.diff` file beside it. The test fails on any
difference that isn't recorded, and on any recorded one that no longer
happens. Every change to yo's output is then a choice someone made, in a
diff someone reviewed. Effects (everything in the table but stdout and stderr)
have no accepted differences: they must match.

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

Fakes can't prove the AppleScript drives iTerm correctly. A short manual
checklist covers that, run once before the switch: start, start in a new
window, reach with a message, title, down, resume-all.

## Open questions

These stay open. Nothing in the design depends on how they're answered.

- **Remote Control:** what Claude Code exposes for driving a Remote Control
  session programmatically. To find out before phase 4 designs a runner for
  it.
- **Approvals in the UI:** for headless sessions they come through the stream.
  Forwarding an iTerm session's permission prompt (from `Notification`)
  could show it, but answering it still means the tab, or typing into it.
- **The hub:** self-hosted, on a home machine, or hosted. Phase 6.
- **Distribution:** stay "clone and `install.sh`" (which would then run
  `cargo build`), release binaries, or a Homebrew tap.
- **An MCP server:** coordinating sessions calling yo as tools rather than
  shelling out. It would sit on `yo-client` next to the TUI.
