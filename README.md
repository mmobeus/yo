# Yo

Personal dev environment: an iTerm2 + Claude Code work-session launcher and the
config that goes with it, installed into `~` as symlinks back into this checkout.

```
scripts/yo              iTerm2 work-session launcher (git worktrees, or in place)
config/yo.config.example.json  an example config, every part of it used
docs/                   the user guide and reference
claude/settings.json    Claude Code settings layer, stacked onto each session
claude/statusline.sh    Claude Code statusline
claude/skills/          skills that drive yo from inside a session (yo-down, yo-adopt, yo-title)
install.sh              wires it all up
```

## Install

```sh
git clone git@github.com:mmobeus/yo.git
cd yo
./install.sh --dry-run     # see the plan
./install.sh
```

That's enough to start: `cd` into any clone and run `yo my-fix 'what to do'`
(see [Start here](docs/guide.md#1-start-here)). Register more repos when you
want them (`yo config add-repo <clone|owner/repo>`). The installer creates
`~/.config/yo/config.json` once, listing this checkout, and never touches it
again: it names your repos, so it lives outside this one.

| what | where it lands |
| --- | --- |
| `yo` | `~/.local/bin/` |
| yo config | `~/.config/yo/config.json` (a plain file, yours) |
| settings layer, statusline | `~/.config/yo/claude-settings.json`, `~/.config/yo/statusline.sh` |
| skills | `~/.claude/skills/` |
| terminal peer | iTerm2's "Claude Code" workgroup (only if it has none) |

Useful flags: `--dry-run`, `--uninstall`, `--copy` (no dependency on this
checkout staying put), `--force`, `--merge-settings`, `--no-iterm`,
`--bin=PATH`. `./install.sh --help` has the rest.

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
- **`yo help`:** the script's own summary, plus the repos, profiles and agents
  it can see.
- **[Core design](docs/design/core.md):** the plan (not built yet) for moving yo from bash to a
  Rust core with a CLI, a TUI, an app and pluggable runners.

## Making changes

Installed files are symlinks into this checkout, so an edit here is live
immediately — which is what you want while iterating on a script, and not what
you want while a change is half-finished. Keep this checkout on `main` and do
the work in a worktree:

```sh
yo yo tweak-the-installer     # worktree + session, this checkout untouched
../yo-worktrees/tweak-the-installer/scripts/yo help   # test by path
```

To exercise `yo` without opening iTerm, set `YO_DRY_LAUNCH=1`: it prints the
tab title, directory and `claude` command instead of launching. Point `HOME`,
`XDG_STATE_HOME` and `YO_CONFIG` at a throwaway directory as well, so the test
leaves your ledger and `~/.claude.json` alone.

Merge to `main` and the installed symlinks pick it up with nothing to re-run.
`install.sh` is only needed again when a *new* file needs linking.

## yo without this repo

`yo` is generic: every site-specific thing lives in `~/.config/yo/config.json`, and
with no config file at all it runs in single-repo mode against whatever clone
you are standing in. `yo help` prints the repos it can currently see.

## Requirements

`git`, `jq`, `python3`, and macOS with iTerm2. `gh` is optional (PR and issue
titles in window titles).
