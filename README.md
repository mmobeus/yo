# Yo

Personal dev environment: an iTerm2 + Claude Code work-session launcher and the
config that goes with it, installed into `~` as symlinks back into this checkout.

```
scripts/yo              iTerm2 work-session launcher (git worktrees, or in place)
config/yo.config.example.json  an example config, every part of it used
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

Then point `yo` at your own clones (see [Configuration](#configuration)). The
installer creates `~/.config/yo/config.json` once, listing this checkout, and
never touches it again: it names your repos, so it lives outside this one.

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

## Sessions

```sh
yo my-fix                         # worktree + branch <prefix>/my-fix, off the base branch
yo https://github.com/o/r/pull/7  # worktree at the PR's head, review seeded
yo --repo=backend -  'look into the flaky test'
yo --here - 'read HANDOFF.md'     # no worktree: this directory, as checked out
yo --title='Fix flaky retry' my-fix   # your own tab / Session Status title
yo --from=<session> my-fix 'brief'    # report back to that Claude session when done
yo --model=sonnet my-fix          # this session only; your default is untouched
yo title 'new title'              # retitle the session you're in (or /yo-title in claude)
yo down                           # tear the session down
yo sessions                       # what yo knows about, and what's running
yo resume-all                     # after a restart: bring them all back
yo sweep                          # which worktrees' work has landed (--apply removes them)
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
yo resume --model=opus my-fix   # ...switching its model on the way back
yo resume-all --dry-run  # every session that was live
```

A session you quit with `/exit` is marked ended: `resume-all` leaves it out,
though `yo resume <name>` still reopens it. One whose tab was closed, or that
crashed, stays ready to resume.

A resumed session comes back under the same repo, profile, memory and settings
layer it started with. `yo resume <session-id>` works for any session, even one
yo didn't start: its directory is read from the transcript, and it joins the
ledger from then on. `yo sessions` also picks up claude sessions running in a
yo worktree that were started by hand, so run it before a planned restart.

A session yo didn't start, anywhere (a primary clone, a scratch directory),
can record itself: `yo adopt` from inside it, or the `/yo-adopt` skill. `yo
forget <name|id>` drops a session from the ledger; its transcript and
directory are left alone.

### Sweeping up worktrees

Only `yo down` removes a worktree, so sessions that end any other way leave
theirs behind. `yo sweep` lists every worktree of every registered repo and
sorts them out:

| verdict | meaning | removed by `--apply` |
| --- | --- | --- |
| `DEAD` | PR merged, no commits beyond base, or a clean PR-review checkout | yes |
| `STALE` | unmerged commits | only with `--include-stale` |
| `SUSP` | a session `resume-all` would bring back | only with `--include-suspended` |
| `KEEP` | uncommitted changes, an open PR, a live session, or the current directory | never |

It is a dry run without `--apply`. Branches are kept unless
`--delete-branches`, and a worktree is never removed with `--force`. PR states
come from `gh`, since a squash merge leaves the merged PR as the only sign the
work landed; `--repo=R` narrows it to some repos.

### Tearing down from inside the session

`yo down --dry-run` reports what teardown would do; `yo down --yes` skips the
prompt (and refuses to lose uncommitted work unless `--discard`). That is what
the `/yo-down` skill uses, so a session can be wound up from inside claude,
including a remote-controlled one with no terminal to type into. `yo down`
closes its own iTerm2 tab, not whichever one has focus.

### Clones of a fork

If a clone's `origin` is your fork, keep the real repo as another remote
(conventionally `upstream`). yo fetches PR heads and the base branch from
whichever remote points at the repo's owner/repo, and still looks for your
own branches on `origin`. Without `--github`, the owner/repo comes from
`upstream` before `origin`.

### Repos yo doesn't know

A PR or issue URL needs no configuration. If its repo isn't registered, yo
clones it to `<cloneRoot>/<owner>/<repo>` — by default
`~/dev/src/github.com/<owner>/<repo>` — or uses the clone already there, and
gives the session a worktree as usual. Where the repo belongs:

- a profile whose `owners` include the repo's owner: it joins that profile, no
  questions asked;
- `--add-to=PROFILE`: it joins that profile, and is saved there in the config;
- otherwise yo asks, when there's a terminal to ask at:

  ```
  acme/widgets isn't configured. Where does it go?
    [1] add it to profile mmobeus
    [n] a new profile "acme" that claims every acme/* repo
    [g] just this once, as a guest (no shared memory, not pre-trusted)
  choice [g]:
  ```

  Either profile choice is saved, so it's asked once per repo (or once per
  owner, for a new profile);
- with nobody to ask (no terminal, or yo started from inside claude), a guest.

A guest gets:

- **no shared memory** — Claude keeps its own per-folder memory for it;
- **no pre-trusted worktree** — the repo is code nobody here has vetted, and it
  can configure hooks and MCP servers, so Claude's trust prompt is left to
  appear.

`cloneProtocol` picks `ssh` (default) or `https`; `guestProfile` names a
profile to run guests under. `yo config add-repo <owner/repo>` registers a repo
once it stops being a guest.

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

```json
"repos":    { "backend": { "path": "...", "mode": "here" } },
"defaults": { "mode": "here" }
```

(the first for one repo, the second for every repo; a profile takes `"mode"`
too).

Since nothing is fetched, a PR or issue URL opened in place gets a warning: the
tree is whatever was last checked out, not the PR's head or a fresh base.

In such a session `yo down` only forgets the session and closes the tab;
there is no worktree to remove, and the directory is left exactly as it is.

### Agents

A repo's merged agents (its `.claude/agents/`) can run a session as
themselves, with their own instructions:

```sh
yo --agent=cindy https://github.com/acme/backend/issues/123   # work an issue as Cindy
yo --agent=cindy unit-time 'cut the test time'                 # a named session as Cindy
yo agent cindy            # Cindy's home: a long-lived checkout of the base branch
yo cindy 'how is the queue today?'   # reach the home: start, resume, or type into it
yo agent                  # the agents registered in the config
```

- Only agents merged on the repo's base branch (or installed in
  `~/.claude/agents`) are accepted, checked before anything is created.
- The ledger records the agent, so `resume` and `resume-all` bring a session
  back as the same agent. Tabs and codenames lead with its name
  ("Cindy · Plucky Gary"); `yo sessions` has an AGENT column.
- An agent's home (`<clone>-worktrees/agent-<name>`) reads and coordinates. It
  runs with Remote Control on and is named `<name>-home`, so workers it starts
  with `yo --agent=<name> --from=<name>-home ...` can report back to it by
  `SendMessage`. Each start moves a clean home to the newest base branch.
  `yo down` and `yo sweep` never remove it; quit it with `/exit`.
- An entry under `"agents"` in the config gives an agent a default repo and a
  display name.
  Unregistered merged agents still work with `--agent` and a repo.

### Reaching a session by name

```sh
yo cindy 'how is the queue today?'   # a registered agent's home
yo cindy-home 'status?'              # a live session by its Claude name (statusline, ListAgents)
yo my-fix 'rebase on main'           # a session in yo's ledger: yo name, session id or id prefix
```

With no flags, `yo NAME ['message']` looks for a registered agent, then a live
session whose Claude name is NAME (a Remote Control name like `cindy-home`
counts), then a session in yo's ledger. A running one has its iTerm tab
brought forward and the message typed in and submitted (its pane is found by
the tty of its `claude` process); a stopped one is resumed with the message as
its first prompt. A name that matches nothing creates a session as before. A
repo or profile name keeps its own meaning (`yo <repo>`), and any flag makes it
an ordinary new session (`yo --repo=backend cindy`).

## Configuration

`~/.config/yo/config.json` (or `$YO_CONFIG`) is plain data:

```json
{
  "defaults": { "branchPrefix": "me", "cloneRoot": "~/src/github.com" },
  "profiles": {
    "acme": {
      "owners": ["acme"],
      "memory": "acme",
      "windowGroup": "acme",
      "repos":  { "backend": { "path": "~/src/github.com/acme/backend", "base": "dev" } },
      "agents": { "cindy": { "repo": "backend" } }
    },
    "notes": { "dir": "~/notes", "memory": "notes" }
  },
  "repos": { "yo": { "path": "~/src/github.com/me/yo" } }
}
```

`config/yo.config.example.json` uses every part. Unknown keys are an error, so
a typo doesn't quietly do nothing.

```sh
yo config                 # the file, and every repo, profile and agent it registers
yo config add-repo ~/src/github.com/acme/frontend --profile=acme
yo config add-repo acme/frontend --profile=acme    # clones it first, if need be
yo config edit            # $EDITOR, then validated
yo config migrate         # an older config.sh -> config.json (the old file is kept)
```

Precedence is the same for everything: command-line flag, then the repo, then
its profile, then `defaults`. A setting already in the environment
(`YO_MODE=here yo ...`) beats the file, for one run.

### Profiles

A profile is one of *your* projects: a named bundle of settings that the repos
belonging to it share, and that can be picked on its own.

```sh
yo backend my-fix        # backend belongs to acme, so acme's settings apply
yo notes                 # from anywhere: a session in the profile's dir
yo --profile=acme -      # the cwd's repo if it is acme's, else its first repo
```

A repo belongs to a profile by being listed in its `repos`, by being a clone
under one of its `reposIn` directories, or by belonging to a GitHub owner it
claims in `owners`: every clone at `<cloneRoot>/<owner>/*` joins, with nothing
to list, and so does a PR or issue URL for one.

| profile key | what it sets |
| --- | --- |
| `owners` | GitHub owners whose repos join this profile |
| `repos`, `reposIn`, `agents` | what belongs to it |
| `mode` | `here` (in place) or `worktree` |
| `memory` | the profile's own memory store |
| `branchPrefix` | prefix for branches yo creates |
| `claude` | the claude command (permission mode, model, …) |
| `settings` | Claude settings stacked over yo's layer: permissions, hooks, env |
| `dir` | the profile's home, used when no repo is named; needn't be a git clone |
| `windowGroup` | the shared iTerm window its sessions join |
| `trust` | `false`: don't pre-trust its worktrees (Claude asks, as for a guest) |

A repo takes `path` (required), `base`, `github`, `mcp` and `mirror` (lists),
`titlePrefix`, `memory`, `branchPrefix` and `mode`. `--profile=P` on the
command line beats the repo's own profile. A `dir` that isn't a registered
clone has nothing to take a worktree from, so those sessions always run in
place.

### Defaults

| key | what it sets | default |
| --- | --- | --- |
| `branchPrefix` | prefix for branches yo creates | `$USER` |
| `mode` | `worktree` or `here` | `worktree` |
| `defaultRepo` | where bare `yo` goes outside a known clone | the first repo |
| `claude` | the claude command | `claude --permission-mode auto` |
| `memories`, `memory` | the shared memory root and store (see [Memory](#memory)) | `~/.claude/memories`, `yo` |
| `settings` | yo's base settings layer | `claude-settings.json` beside the config |
| `cloneRoot`, `cloneProtocol` | where unknown repos are cloned, and how | `~/dev/src/github.com`, `ssh` |
| `guestProfile` | a profile to run guests under | none |
| `reviewCmd` | the command a PR-review session is seeded with | `/code-review` |
| `layout` | `auto`, `split` or `workgroup` | `auto` |
| `windowMode`, `windowGroup` | `tab` or `window`; the shared window's label | `tab`, `yo` |
| `winWidth`, `winHeight`, `itermProfile` | new-window size; the title-locking iTerm profile | `1403`, `824`, `worksesh` |

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

```json
"defaults": { "memories": "~/src/github.com/me/memories" }
```

| setting | memory directory |
| --- | --- |
| default | `~/.claude/memories/yo` |
| `"memories": DIR` | `DIR/yo` |
| `"memory": NAME` in `defaults` | `<memories>/NAME` |
| `"memory": NAME` on a repo or profile | `<memories>/NAME`, for that repo or profile only |
| `"memories": ""` | off — Claude's own per-folder memory |

This has to come from yo (or user settings): Claude ignores
`autoMemoryDirectory` in project settings, so a cloned repo can't redirect
where memory is written. Memories Claude already wrote under
`~/.claude/projects/` stay there; yo sessions just don't read them.

## iTerm2

yo works with iTerm2's Claude Code integration rather than around it. When the
"Claude Code" workgroup has a terminal peer (e.g. "CLI"), that peer is the
session's shell, so yo opens claude alone in the tab instead of splitting a
pane. Without one it splits, as before. `"layout"` in `defaults` (`auto`, `split`,
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
