# yo user guide

yo starts Claude Code work sessions in iTerm2, keeps track of them, and brings
them back after a restart. This guide walks through how to use it, task by
task. For every command, flag and config key in one place, see the
[reference](reference.md); `yo help` prints the same from the script itself.

- [How yo thinks](#how-yo-thinks)
- [Getting started](#getting-started)
- [Everyday sessions](#everyday-sessions)
- [Working in place](#working-in-place)
- [Ending sessions and cleaning up](#ending-sessions-and-cleaning-up)
- [Surviving restarts](#surviving-restarts)
- [Coordinating work across sessions](#coordinating-work-across-sessions)
- [Repos, profiles and the config](#repos-profiles-and-the-config)
- [Inside a session: settings, memory, skills, statusline](#inside-a-session-settings-memory-skills-statusline)
- [iTerm2](#iterm2)
- [Troubleshooting](#troubleshooting)

## How yo thinks

- **A session** is one `claude` running in an iTerm2 tab, in one directory.
  Every session gets a codename ("Plucky Gary") in its first message, which it
  uses to introduce and identify itself.
- **A worktree session** gets a fresh git worktree and branch of its own, off
  the repo's base branch: `<clone>-worktrees/<name>` on branch
  `<prefix>/<name>`. Several sessions can work on one repo without stepping on
  each other. This is the default.
- **An in-place session** runs in an existing directory, as it is: no
  worktree, no branch, nothing fetched.
- **The ledger** is yo's record of every session it knows about: its id,
  directory, repo, profile, title, agent. It is what lets yo resume sessions
  after Claude Code has forgotten them.
- **Repos** are clones yo knows by a short key (`yo`, `backend`). **Profiles**
  group repos and carry shared settings: a memory store, the claude command,
  a window, extra Claude settings. Both live in one JSON config.
- **Homes** are long-lived coordinating sessions: a repo called `NAME-home`, or
  an agent's home. You reach them by name (`yo mmobeus 'how is it going?'`),
  and they start worker sessions that report back.

## Getting started

```sh
git clone git@github.com:mmobeus/yo.git && cd yo
./install.sh --dry-run    # see what it would do
./install.sh
```

The installer symlinks `yo` into `~/.local/bin`, links the skills and the
settings layer, and creates `~/.config/yo/config.json` listing this checkout.
Then tell yo about your own clones:

```sh
yo config add-repo ~/src/github.com/acme/backend
yo config add-repo acme/frontend          # clones it to <cloneRoot>/acme/frontend first
yo config                                 # what yo can see now
```

With no config at all, yo still works in single-repo mode: the clone you are
standing in is the repo.

Your first session:

```sh
cd ~/src/github.com/acme/backend
yo my-fix 'the retry test is flaky; find out why'
```

A tab opens with claude in a new worktree, already reading your message.

## Everyday sessions

### Starting one

`yo [INPUT] [message]`, where INPUT decides what the session is:

| you type | you get |
| --- | --- |
| `yo` or `yo -` | an unnamed session (`sesh_<timestamp>`), titled with its codename |
| `yo my-fix` | session `my-fix` on a new branch `<prefix>/my-fix` |
| `yo jj/rollbacks` | an existing branch, checked out as-is |
| `yo <PR URL>` | a worktree at the PR's head, tracking it, with `/code-review <url>` seeded |
| `yo <issue URL>` | session `issue_<n>`, seeded with "help me address this issue" |

The optional last argument is a message for claude: appended to the seeded
one for a PR or issue, or the whole first message otherwise. Quote it.

The repo is the one you're standing in, or `--repo=R`, or the default repo
when you're somewhere yo doesn't know. `yo <repo> [name]` works too.

Useful flags:

```sh
yo --title='Fix flaky retry' my-fix     # your own tab title
yo --model=sonnet my-fix                # this session only; your default is untouched
yo --new-window my-fix                  # its own window instead of a tab
yo --profile=acme -                     # run under a profile (see below)
```

### What yo does when it creates one

1. Fetches the base branch (or the PR head) and adds the worktree, reusing a
   local or remote branch of the same name if there is one.
2. Pre-trusts the worktree in `~/.claude.json`, so Claude doesn't ask, and
   copies the clone's `.claude/settings.local.json` (MCP approvals) and any
   gitignored files the repo lists in `mirror` (such as `.env`).
3. Records the session in the ledger, under an id yo picks itself.
4. Opens the tab and starts claude with yo's settings layer and the first
   message.

### Titles

The tab title is the session name, `PR#123: <title>` / `Issue#45: <title>`
for URLs (via `gh`), or the codename for an unnamed session. Repos other than
the default get a `[repo]` prefix. `--title` overrides all of that, and from
inside a session `yo title 'new title'` (or `/yo-title`) renames the tab, the
Session Status row and the ledger record together.

## Working in place

```sh
yo --here - 'read HANDOFF.md and pick up where it left off'
```

`--here` runs the session in an existing directory, on whatever is checked
out. It is meant for handing work from one session to the next in the same
tree, and for directories that aren't units of work at all, like a home repo.

- A repo you *named* (`--repo`, `yo <repo>`, a URL) means its clone; otherwise
  the current directory, which needn't be a git clone.
- A repo or profile can make in place its default (`"mode": "here"`);
  `--worktree` then asks for a worktree this once.
- A PR or issue opened in place gets a warning: nothing was fetched, so the
  tree may not be the PR's code.

## Ending sessions and cleaning up

There are three ways a session ends, and they mean different things:

| how | what yo does |
| --- | --- |
| `/exit` in claude | marks it **ended**: `resume-all` leaves it out; `yo resume <name>` still reopens it |
| close the tab, quit iTerm, crash | nothing: it stays **ready**, and `resume-all` brings it back |
| `yo down` | removes the worktree (the branch is kept), forgets the session, closes the tab |

`yo down` warns about uncommitted and unpushed work and asks first. From
inside claude, the `/yo-down` skill runs `yo down --yes`, which refuses to
throw away uncommitted work unless told `--discard`. In an in-place session
`yo down` only forgets the session and closes the tab.

Sessions that end any other way leave their worktrees behind. `yo sweep`
sorts them out:

```sh
yo sweep            # dry run: DEAD / STALE / SUSP / KEEP for every worktree
yo sweep --apply    # remove the DEAD ones (merged, or nothing beyond base)
```

It never removes a worktree with uncommitted changes, an open PR, a live
session, one `resume-all` would bring back, or an agent's home, and it keeps
branches unless `--delete-branches`.

`yo forget <name|id>` drops a session from the ledger without touching its
directory.

## Surviving restarts

Claude Code forgets which sessions were running the moment it restarts. yo's
ledger doesn't:

```sh
yo sessions               # running, ready, ended, or gone
yo resume-all             # after a restart: one tab per session that was live
yo resume my-fix          # just one, by name, id, id prefix or directory
yo resume --model=opus my-fix
```

To restart safely, quit iTerm rather than `/exit`-ing each session: `/exit`
means "done". A resumed session comes back with the same repo, profile,
memory, settings layer, agent and title.

Sessions yo didn't start can join: `yo sessions` picks up claude running in a
yo worktree, `yo resume <session-id>` works for any session with a
transcript, and `yo adopt` (the `/yo-adopt` skill) records the session it runs
in, wherever it is.

## Coordinating work across sessions

### Reaching a session by name

```sh
yo my-fix 'rebase on main'                 # a session in the ledger
yo cindy-home 'status?'                    # a live session by its Claude name
yo mmobeus 'how is the yo work going?'     # a home (see below)
```

With no flags, `yo NAME ['message']` finds what NAME names. A running session
has its iTerm tab brought forward and the message typed in and submitted; a
stopped one is resumed with the message as its first prompt. A name that
matches nothing starts a new session, as always. The full lookup order is in
the [reference](reference.md#yo-name).

A session's Claude name is what its statusline shows after 📡, what
`ListAgents` prints and what `SendMessage` takes.

### A home session

A home is a repo for the work that isn't any one repo: notes, plans, memory,
skills. Name it `NAME-home` (or give it that key), make it in place, and
`yo NAME` reaches its session:

```json
"repos": { "home": { "path": "~/dev/src/github.com/me/me-home", "mode": "here" } }
```

```sh
yo me 'what is open this week?'
```

It brings forward the session running there, else resumes the most recently
used one, else starts a new one, always with your message. A session you
`/exit` is left alone, so the next `yo me` starts fresh.

### Handing work out with `--from`

A coordinating session starts workers and hears back from them:

```sh
yo --repo=backend --from=me-home --title='Fix retry' retry-fix '<the brief>'
```

`--from` tells the worker to report to that session with `SendMessage` when it
finishes or gets stuck. The brief is the worker's entire context: the goal,
links, constraints, and what "done" means.

### Agents

A repo's agents (`.claude/agents/*.md`, merged on its base branch) can run a
session as themselves:

```sh
yo --agent=cindy https://github.com/acme/backend/issues/123   # work an issue as Cindy
yo agent cindy                    # Cindy's home
yo cindy 'how is the queue?'      # reach it, however it's doing
```

- The agent's instructions replace the default ones, and an issue or PR is
  handed to it to work by those instructions. Tabs and codenames lead with
  its name ("Cindy · Plucky Gary").
- **An agent's home** is a long-lived, detached checkout of the base branch at
  `<clone>-worktrees/agent-<name>`, refreshed when it starts and is clean. It
  runs with Remote Control as `<name>-home`, coordinates, and starts workers
  with `--agent=<name> --from=<name>-home`. `yo down` and `yo sweep` leave it
  alone.
- Register an agent under `"agents"` in the config to give it a default repo
  and a display name, and to make `yo <name>` reach its home. A registered
  agent wins over a `NAME-home` repo.

## Repos, profiles and the config

Everything site-specific is in `~/.config/yo/config.json`:

```json
{
  "defaults": { "branchPrefix": "jj", "memories": "~/dev/src/github.com/me/me-home/memories" },
  "profiles": {
    "acme": {
      "owners": ["acme"],
      "memory": "acme",
      "repos": { "backend": { "path": "~/src/github.com/acme/backend", "base": "dev" } }
    }
  },
  "repos": { "home": { "path": "~/dev/src/github.com/me/me-home", "mode": "here" } }
}
```

`yo config` shows what it registers, `yo config edit` opens it and checks it
afterwards, and unknown keys are an error. Settings resolve flag → repo →
profile → defaults; a variable already in your environment
(`YO_MODE=here yo ...`) wins for that run.

### Profiles

A profile is one of your projects. Its repos share its memory store, claude
command, window, branch prefix and extra Claude settings, and `yo <profile>`
or `--profile=P` starts a session in it from anywhere.

A repo joins a profile by being listed in its `repos`, by sitting in one of
its `reposIn` directories, or by belonging to a GitHub owner the profile
claims in `owners`. Owners are the low-maintenance option: every clone at
`<cloneRoot>/<owner>/*` joins, and so does a PR or issue URL for one, with
nothing to list.

### Repos yo doesn't know

A PR or issue URL needs no setup: yo clones the repo (or finds the clone) at
`<cloneRoot>/<owner>/<repo>`. If a profile claims the owner, the repo joins
it. Otherwise yo asks, when there is a terminal to ask at:

```
acme/widgets isn't configured. Where does it go?
  [1] add it to profile mmobeus
  [n] a new profile "acme" that claims every acme/* repo
  [g] just this once, as a guest (no shared memory, not pre-trusted)
choice [g]:
```

Profile choices are saved. `--add-to=PROFILE|guest` answers in advance, and
with nobody to ask (from inside claude, say) it's a guest.

A **guest** gets no shared memory, and its worktree isn't pre-trusted, so
Claude asks before running anything the repo configures. A profile with
`"trust": false` treats its own repos the same way.

### Clones of a fork

If a clone's `origin` is your fork, keep the real repo as another remote
(`upstream`, conventionally). yo fetches PR heads and the base branch from
the remote that points at the real repo, and your own branches from `origin`.

## Inside a session: settings, memory, skills, statusline

**Settings layer.** yo never edits `~/.claude/settings.json`; Claude Code and
iTerm2 both write it. Each session instead gets
`claude --settings ~/.local/state/yo/settings/<label>.json`, composed from
yo's `claude-settings.json`, the profile's `settings` file, the session's
memory directory, and the ledger's hooks. It outranks every settings file on
disk, and lists such as permissions merge with yours. `./install.sh
--merge-settings` puts the same settings in every claude session.

**Memory.** Every yo session shares one auto-memory store, across repos and
worktrees: `~/.claude/memories/yo` by default, or `<memories>/<memory>` from
the config. Point `memories` at a repo to keep it in git. A repo or profile
can have a store of its own (`"memory": "acme"`), `"none"` leaves Claude its
per-folder memory, and `"memories": ""` turns the whole thing off.

**Skills.** Three skills drive yo from inside claude, so a session can be
managed even through Remote Control:

| skill | does |
| --- | --- |
| `/yo-down` | tears this session down (`yo down --yes`) |
| `/yo-adopt` | records this session in the ledger |
| `/yo-title` | retitles this session's tab |

**Statusline.** The installed statusline shows the model, the session's
Claude name (📡, what you'd pass to `--from` or `yo NAME`), the branch, a
context-usage bar, cost and time.

## iTerm2

- **One window, many tabs.** Sessions open as tabs in a shared window, so
  iTerm2's Session Status tool, which lists one window's sessions, becomes a
  dashboard. `windowGroup` names that window (`{repo}` in it gives one per
  repo); `--new-window` or `"windowMode": "window"` opts out.
- **Layout.** With iTerm2's Claude Code workgroup set up with a terminal peer
  ("CLI"), claude gets the tab to itself and the peer is the shell. Without
  one, the tab is split: claude left, a shell right. The installer adds the
  peer if missing; it shows up after iTerm2 restarts.
- **Titles that stick.** claude sets the terminal title constantly, so yo
  uses a generated iTerm profile (`worksesh`) that ignores title changes and
  pins the tab title to yo's. The workgroup renames sessions to "Chat";
  `yo title` with no argument puts the names back.

## Troubleshooting

**`yo forget` says the session "is still running" after I closed its tab.**
Claude takes a moment to exit after its tab closes. The forget still worked.

**Claude asks whether to trust the folder.** The repo is a guest, or its
profile has `"trust": false`, or the session is in place in a directory yo
didn't create. That's deliberate; answer it once.

**A message didn't reach a running session.** yo finds the session's tab by
its terminal. A session running without an iTerm tab (Remote Control only)
can't be typed into; yo says so and names how to reach it instead.

**`worktree path already exists`.** A session by that name already has a
worktree. If yo knows the session (`yo sessions`), `yo my-fix 'message'` with
no flags reaches it instead. Otherwise pick another name, or `yo sweep` to
clean up finished ones.

**The tab says "Chat".** The Claude Code workgroup renamed it. `yo title`
restores every yo tab's name.

**`yo` says the config failed to load.** The message names the bad key or the
JSON error. `yo config edit` validates after you save.

**Testing yo itself.** Run the script by path with `YO_DRY_LAUNCH=1` to print
the launch instead of opening iTerm. yo still records the session in the
ledger before launching, so point `HOME`, `XDG_STATE_HOME` and `YO_CONFIG` at
a throwaway directory too.
