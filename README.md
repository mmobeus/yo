# Yo

Personal dev environment: an iTerm2 + Claude Code work-session launcher and the
config that goes with it, installed into `~` as symlinks back into this checkout.

```
scripts/yo              iTerm2 work-session launcher (git worktrees, or in place)
config/yo.config.example.sh   starting point for your own config
claude/settings.json    Claude Code settings layer, stacked onto each session
claude/statusline.sh    Claude Code statusline
claude/skills/          skills that drive yo from inside a session (yo-down)
install.sh              wires it all up
```

## Install

```sh
git clone git@github.com:mmobeus/yo.git
cd yo
./install.sh --dry-run     # see the plan
./install.sh
```

Then edit `~/.config/yo/config.sh` to point `yo` at your own clones. The
installer creates that file once, from `config/yo.config.example.sh`, and never
touches it again: it names your repos, so it lives outside this one.

| what | where it lands |
| --- | --- |
| `yo` | `~/.local/bin/` |
| yo config | `~/.config/yo/config.sh` (a plain file, yours) |
| settings layer, statusline | `~/.config/yo/claude-settings.json`, `~/.config/yo/statusline.sh` |
| skills | `~/.claude/skills/` |
| terminal peer | iTerm2's "Claude Code" workgroup (only if it has none) |

Useful flags: `--dry-run`, `--uninstall`, `--copy` (no dependency on this
checkout staying put), `--force`, `--merge-settings`, `--no-iterm`,
`--bin=PATH`. `./install.sh --help` has the rest.

## Sessions

```sh
yo my-fix                         # worktree + branch <prefix>/my-fix, off the base branch
yo https://github.com/o/r/pull/7  # worktree at the PR's head, review seeded
yo --repo=backend -  'look into the flaky test'
yo --here - 'read HANDOFF.md'     # no worktree: this directory, as checked out
yo down                           # tear the session down
yo sessions                       # what yo knows about, and what's running
yo resume-all                     # after a restart: bring them all back
```

`yo help` has the full story, and lists the repos it can see.

### Resuming after a restart

Claude Code forgets which sessions were running as soon as it restarts, so yo
keeps its own ledger under `~/.local/state/yo/sessions/`: each session's id,
directory, repo, profile and title. yo picks the id itself when it creates a
session, and a hook in the session's settings layer follows it through
`/clear`.

```sh
yo sessions              # running, ready to resume, or gone
yo resume my-fix         # one session, by name, session id or directory
yo resume-all --dry-run  # every session that was live
```

A resumed session comes back under the same repo, profile, memory and settings
layer it started with. `yo resume <session-id>` works for any session, even one
yo didn't start: its directory is read from the transcript, and it joins the
ledger from then on. `yo sessions` also picks up claude sessions running in a
yo worktree that were started by hand, so run it before a planned restart.

### Tearing down from inside the session

`yo down --dry-run` reports what teardown would do; `yo down --yes` skips the
prompt (and refuses to lose uncommitted work unless `--discard`). That is what
the `/yo-down` skill uses, so a session can be wound up from inside claude,
including a remote-controlled one with no terminal to type into. `yo down`
closes its own iTerm2 tab, not whichever one has focus.

### Repos yo doesn't know

A PR or issue URL needs no configuration. If its repo isn't registered, yo
clones it to `<YO_CLONE_ROOT>/<owner>/<repo>` — by default
`~/dev/src/github.com/<owner>/<repo>` — or uses the clone already there, and
gives the session a worktree as usual.

Such a session is a guest:

- **no shared memory** — Claude keeps its own per-folder memory for it;
- **no pre-trusted worktree** — the repo is code nobody here has vetted, and it
  can configure hooks and MCP servers, so Claude's trust prompt is left to
  appear.

`YO_CLONE_PROTOCOL` picks `ssh` (default) or `https`; `YO_ADHOC_PROFILE` names
a profile to run guests under. Register the repo with `yo_repo` once it stops
being a guest.

### In place: `--here`

`yo --here` opens a session in an existing directory — no worktree, no branch,
nothing fetched. It is for handing work from one session to the next in the
same tree: write the handoff, `yo --here`, let the two sessions talk, stop the
old one.

Which directory: a repo you *named* (`--repo`, `yo <repo>`, a PR/issue URL)
means that repo's clone, wherever you are standing; otherwise it is the current
directory, which needn't be a git clone.

In place can also be the default, per repo or everywhere, with `--worktree` to
opt out for one session:

```sh
yo_repo backend --path=... --mode=here   # in ~/.config/yo/config.sh
YO_MODE="here"                           # or for every repo
```

Since nothing is fetched, a PR or issue URL opened in place gets a warning: the
tree is whatever was last checked out, not the PR's head or a fresh base.

In such a session `yo down` only forgets the session and closes the tab;
there is no worktree to remove, and the directory is left exactly as it is.

## Profiles

A profile is one of *your* projects: a named bundle of defaults that the repos
belonging to it share, and that can be picked on its own.

```sh
# ~/.config/yo/config.sh
yo_profile acme  --memory=acme --window-group=acme
yo_repo backend  --path=… --profile=acme
yo_repo frontend --path=… --profile=acme

yo_profile notes --dir="$HOME/notes" \
  --memory=notes --settings="$HOME/.config/yo/notes.settings.json"
```

```sh
yo frontend my-fix       # frontend belongs to acme, so acme's defaults apply
yo notes                 # from anywhere: a session in the profile's --dir
yo --profile=acme -      # the cwd's repo if it is acme's, else its first repo
```

| option | what it sets |
| --- | --- |
| `--mode=here\|worktree` | in place, or a worktree per session |
| `--memory=STORE` | the profile's own memory store |
| `--branch-prefix=S` | prefix for branches yo creates |
| `--claude="CMD"` | the claude command (permission mode, model, …) |
| `--settings=FILE` | Claude settings stacked over yo's layer: permissions, hooks, env |
| `--dir=PATH` | the profile's home, used when no repo is named; needn't be a git clone |
| `--window-group=LABEL` | the shared iTerm window its sessions join |

Precedence is the same for everything: command-line flag, then the `yo_repo`
option, then the profile, then the global setting. `--profile=P` on the command
line beats the repo's own profile.

A `--dir` that isn't a registered clone has nothing to take a worktree from, so
those sessions always run in place.

## Claude settings: stacked, not merged

`~/.claude/settings.json` is written by Claude Code itself and by iTerm2's
Claude Code integration (its hooks live there), so yo stays out of it. Claude
Code has no user-level `settings.local.json` — that file only exists per
project — but it does take an extra layer on the command line, and that is what
yo uses:

```sh
claude --settings ~/.local/state/yo/settings/<repo>.json
```

The layer outranks every settings file on disk, and arrays such as hooks and
permissions are merged with yours rather than replacing them. yo composes it
per session from `claude/settings.json`, the profile's `--settings` file, the
session's memory directory, and the hook that keeps the session ledger
current. `--settings` takes a single value, so all of it goes in one file.

Only sessions yo launches see the layer. To have those settings in every
`claude` session, `./install.sh --merge-settings` `jq`-merges them into
`~/.claude/settings.json` (this repo's keys win, the rest is preserved, and the
file is backed up first).

## Memory

Claude's auto-memory normally lives in per-folder stores under
`~/.claude/projects/`, so every worktree starts with none. yo gives all of its
sessions one shared memory instead — across repos, worktrees and plain
directories — at `~/.claude/memories/yo`.

To keep it in a repo of your own, point yo at the checkout in your config:

```sh
YO_MEMORIES="$HOME/src/github.com/me/memories"
```

| setting | memory directory |
| --- | --- |
| default | `~/.claude/memories/yo` |
| `YO_MEMORIES=DIR` | `DIR/yo` |
| `YO_MEMORY_STORE=NAME` | `<YO_MEMORIES>/NAME` |
| `yo_repo … --memory=NAME` | `<YO_MEMORIES>/NAME`, for that repo only |
| `YO_MEMORIES=""` | off — Claude's own per-folder memory |

This has to come from yo (or user settings): Claude ignores
`autoMemoryDirectory` in project settings, so a cloned repo can't redirect
where memory is written. Memories Claude already wrote under
`~/.claude/projects/` stay there; yo sessions just don't read them.

## iTerm2

yo works with iTerm2's Claude Code integration rather than around it. When the
"Claude Code" workgroup has a terminal peer (e.g. "CLI"), that peer is the
session's shell, so yo opens claude alone in the tab instead of splitting a
pane. Without one it splits, as before. `YO_LAYOUT` (`auto`, `split`,
`workgroup`) overrides the choice.

The installer adds a "CLI" peer when the workgroup has none, saving the
previous workgroups first. iTerm2 reads them at launch, so with iTerm2 running
the peer appears after its next restart — and editing workgroups in iTerm2's
settings before then overwrites it (re-run `install.sh` if so).

## Making changes

Installed files are symlinks into this checkout, so an edit here is live
immediately — which is what you want while iterating on a script, and not what
you want while a change is half-finished. Keep this checkout on `main` and do
the work in a worktree:

```sh
yo yo tweak-the-installer     # worktree + session, this checkout untouched
../yo-worktrees/tweak-the-installer/scripts/yo help   # test by path
```

Merge to `main` and the installed symlinks pick it up with nothing to re-run.
`install.sh` is only needed again when a *new* file needs linking.

## yo without this repo

`yo` is generic: every site-specific thing lives in `~/.config/yo/config.sh`, and
with no config file at all it runs in single-repo mode against whatever clone
you are standing in. `yo help` prints the repos it can currently see.

## Requirements

`git`, `jq`, `python3`, and macOS with iTerm2. `gh` is optional (PR and issue
titles in window titles).
