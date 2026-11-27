# Yo

An iTerm2 + Claude Code work-session launcher: each session on its own git
worktree, tracked, and brought back after a restart.

```
crates/                 yo itself: a Rust workspace; crates/yo-cli is the `yo` binary
config/yo.config.example.json  an example config, every part of it used
docs/                   the user guide and reference
claude/settings.json    Claude Code settings layer, stacked onto each session
claude/statusline.sh    Claude Code statusline
claude/skills/          skills that drive yo from inside a session (yo-down, yo-adopt, yo-title)
```

## Install

```sh
brew install mmobeus/tap/yo
yo setup --dry-run     # see the plan
yo setup               # skills, settings layer, statusline, a starting config
```

The formula arrives with yo's first release. Until then, or to build it
yourself, install from source with Rust ([rustup](https://rustup.rs)):

```sh
git clone https://github.com/mmobeus/yo.git && cd yo
cargo install --locked --path crates/yo-cli
yo setup
```

That's enough to start: `cd` into any clone and run `yo my-fix 'what to do'`
(see [Start here](docs/guide.md#1-start-here)). Register repos when you want
them (`yo config add-repo <clone|owner/repo>`). `yo setup` creates
`~/.config/yo/config.json` once and never touches it again: it names your
repos, so it's yours.

| what | where it lands |
| --- | --- |
| yo config | `~/.config/yo/config.json` (a plain file, yours) |
| settings layer, statusline | `~/.config/yo/claude-settings.json`, `~/.config/yo/statusline.sh` |
| skills | `~/.claude/skills/` |
| terminal peer | iTerm2's "Claude Code" workgroup (only if it has none) |

The files are copies built into yo, so run `yo setup` again after upgrading
(yo says so when they're out of date). Useful flags: `--dry-run`,
`--uninstall`, `--merge-settings`, `--no-iterm`, `--force`; `yo setup --help`
has the rest.

## A quick tour

```sh
yo my-fix 'the retry test is flaky'      # a worktree + branch my-fix, claude seeded
yo https://github.com/o/r/pull/7         # a worktree at the PR's head, review seeded
yo --here - 'read HANDOFF.md'            # no worktree: this directory, as checked out
yo --agent=cindy <issue URL>             # work an issue as one of the repo's agents
yo my-fix 'rebase on main'               # reach a session by name: tab forward, message typed in
yo me 'what is open this week?'          # reach your home session (a repo called me-home)
yo sessions                              # what yo knows about, and what's running
yo resume-all                            # after a restart: bring them all back
yo down                                  # done: remove the worktree, close the tab
yo sweep                                 # which worktrees' work has landed (--apply removes them)
yo config                                # what's configured
```

## Documentation

- **[User guide](docs/guide.md):** from the basics up: your first session,
  everyday sessions, resuming and cleanup, more repos, working in place,
  profiles, coordinating sessions (homes, `--from`, agents), what yo sets up
  under the hood, and troubleshooting.
- **[Reference](docs/reference.md):** every command, flag, config key,
  environment variable and file.
- **`yo help`:** yo's own summary, plus the repos, profiles and agents it
  can see. `yo --help` and `yo <command> --help` list each command's flags.
- **[Core design](docs/design/core.md):** how yo is built (a Rust core under
  the CLI), and the plan for a TUI, an app and pluggable runners. Its status
  table says what's built.

## Making changes

[`AGENTS.md`](AGENTS.md) has the conventions, the gates to run before a PR,
and the review checklists. The short version for trying a change:

Keep your main checkout on `main` and do the work in a worktree. To have
your installed skills and settings follow a checkout, link them into it, and
install yo from it:

```sh
cargo install --locked --path crates/yo-cli   # yo, built from this checkout
yo setup --link .                              # the claude/ files, as links here
```

Then an edit to a skill or the statusline here is live immediately, and `yo`
is once it's reinstalled. Try a change by path from its worktree:

```sh
yo yo tweak-the-statusline     # worktree + session, the main checkout untouched
cd ../yo-worktrees/tweak-the-statusline
cargo build && target/debug/yo help    # test by path
cargo xtask ci                         # the gates, as CI runs them
```

To exercise `yo` without opening iTerm, set `YO_DRY_LAUNCH=1`: it prints the
tab title, directory and `claude` command instead of launching. Point `HOME`,
`XDG_STATE_HOME` and `YO_CONFIG` at a throwaway directory as well, so the test
leaves your ledger and `~/.claude.json` alone (and give `yo setup`
`--no-iterm`: iTerm2's preferences aren't under `HOME`).

## yo without this repo

`yo` is generic: every site-specific thing lives in `~/.config/yo/config.json`, and
with no config file at all it runs in single-repo mode against whatever clone
you are standing in. `yo help` prints the repos it can currently see.

## Requirements

macOS with iTerm2, `git` and `jq` (the statusline). `gh` is optional (PR and
issue titles in window titles). Building from source needs Rust
([rustup](https://rustup.rs)); `rust-toolchain.toml` pins the toolchain.

## License

[MIT](LICENSE).
