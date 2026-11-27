# yo user guide

yo starts Claude Code work sessions in iTerm2, keeps track of them, and brings
them back after a restart. This guide starts with the basics and adds a piece
at a time; each part opens with when you'd want it, so stop wherever yo already
does what you need. For every command, flag and config key in one place, see
the [reference](reference.md).

1. [Start here](#1-start-here): install, your first session, ending it
2. [Everyday sessions](#2-everyday-sessions): PRs, issues, titles, models
3. [Coming back to sessions](#3-coming-back-to-sessions): resume, restarts, cleanup
4. [More than one repo](#4-more-than-one-repo): registering repos, repos yo doesn't know
5. [Working in place](#5-working-in-place): no worktree
6. [Profiles](#6-profiles): grouping repos and their settings
7. [Coordinating sessions](#7-coordinating-sessions): reaching by name, a home, handing off, agents
8. [Under the hood](#8-under-the-hood): what yo sets up for each session
9. [Troubleshooting](#9-troubleshooting)

## 1. Start here

### Install

```sh
brew install mmobeus/tap/yo
yo setup --dry-run    # see what it would do
yo setup
```

Upgrade with `brew upgrade yo`, then `yo setup` again. To build it yourself
instead, install from source with Rust ([rustup](https://rustup.rs)):

```sh
git clone https://github.com/mmobeus/yo.git && cd yo
cargo install --locked --path crates/yo-cli
yo setup
```

`yo setup` installs a few skills and settings, described later, and is safe
to run again; run it again after upgrading yo. You need git, jq and iTerm2 on
macOS; `gh` is optional.

### Your first session

No configuration is needed. Stand in any clone and run:

```sh
cd ~/src/github.com/acme/backend
yo my-fix 'the retry test is flaky; find out why'
```

An iTerm tab opens with claude already reading your message. It's working in
a new git worktree at `../backend-worktrees/my-fix`, on a new branch `my-fix`
off the repo's main branch, so your clone is untouched and you
can start more sessions side by side. Claude introduces itself with a
codename ("Plucky Gary") that identifies the session.

Leave off the name for a quick, unnamed session:

```sh
yo                                    # titled with its codename
yo - 'prod is broken, investigate!'   # `-` lets a message follow
```

### Ending it

When the work is done (committed and pushed), run this in the session's shell,
or type `/yo-down` in claude:

```sh
yo down
```

It warns about anything uncommitted or unpushed, removes the worktree (the
branch is kept), and closes the tab. If claude is still running in the tab
(you ran `yo down` in the CLI shell), it ends claude first, and says so
before you confirm: closing a tab while it shows CLI would leave claude
running with no tab. A session running in another tab is left alone.

### Seeing what's running

```sh
yo sessions
```

That's the whole basic loop: `yo <name> '<what to do>'`, work, `yo down`.

## 2. Everyday sessions

### Starting from a PR or an issue

```sh
yo https://github.com/acme/backend/pull/123     # review a PR
yo https://github.com/acme/backend/issues/45    # work an issue
```

A PR session's worktree is the PR's own code, tracking it so `git pull`
brings in new pushes, and claude starts on `/code-review <url>`. An issue
session starts off the main branch with the issue to read and discuss first.
Add a message to either to steer it:

```sh
yo https://github.com/acme/backend/pull/123 'focus on the migration'
```

### Picking up an existing branch

Name the branch in full and yo checks it out as-is:

```sh
yo fix/rollbacks
```

### Titles and models

Tabs are titled with the session name, `PR#123: <title>` or `Issue#45:
<title>`, or the codename. To choose your own:

```sh
yo --title='Fix flaky retry' my-fix
yo title 'Retry: found it'          # later, from inside the session (or /yo-title)
```

To run one session on a different model without changing your default:

```sh
yo --model=sonnet my-fix
```

### Codex

Sessions run on Claude Code unless you pick another coding agent. To run one on
the Codex CLI:

```sh
yo --harness=codex my-fix 'make the retry test pass'
```

To make it the default for a repo, a profile or everywhere, set `"harness":
"codex"` there (see the [reference](reference.md#harnesses)). yo remembers
each session's harness, so `yo resume`, `resume-all` and `yo my-fix 'and
now?'` bring a Codex session back on Codex.

A Codex session gets less than a Claude one: there are no agents, settings
layer or shared memory. Codex picks its own session id, which yo picks up
from Codex's records the next time it looks, for example with `yo sessions`.
Until then, `yo sessions` shows it as `waiting?`, which a while after launch
usually means its tab is stopped at one of Codex's prompts.

Codex asks once per repo whether to trust it, and every worktree of that repo
inherits the answer. When it hasn't been asked yet, yo says so before the tab
opens. For a repo yo trusts, it offers to answer for you: at a terminal, yo
asks, and on a yes it adds the repo's main clone to `~/.codex/config.toml`
(one entry at the end; nothing else there changes).

To tell `/quit` from a closed tab, Codex needs yo's hook, which only `yo setup`
adds, and only when asked (it runs for every Codex session, yo's or not). Codex
runs a hook only once you've approved it, so setup then offers to open Codex
for that. With the hook, a session you quit shows as `ended`, like a Claude
one. Without it, a quit session stays resumable, and `yo down` or `yo forget`
removes it. Until the hook is approved, yo warns that the new tab will stop
at Codex's hook review.

## 3. Coming back to sessions

### How a session ends matters

| how it ends | what happens next |
| --- | --- |
| `yo down` | gone: worktree removed (branch kept), tab closed |
| `/exit` in claude | **ended**: stays out of `resume-all`, but `yo resume <name>` reopens it |
| close the tab, quit iTerm, or a crash | **ready**: `resume-all` brings it back |
| close the tab while it shows CLI | **running (no tab)**: iTerm leaves claude running; see [Troubleshooting](#9-troubleshooting) |

### Resuming

```sh
yo resume my-fix          # by name, session id, or directory
yo my-fix 'and now?'      # the same, with a message to start on
yo resume-all             # every session that was live, e.g. after a restart
```

Claude Code itself forgets running sessions when it restarts; yo keeps its own
record, so a reboot costs nothing. To restart safely, quit iTerm rather than
`/exit`-ing each session.

A resumed session comes back as it was: same directory, title and settings.
`yo resume --model=opus my-fix` switches its model on the way back.

### Cleaning up

Sessions that end without `yo down` leave their worktrees behind. `yo sweep`
finds the ones whose work has landed:

```sh
yo sweep            # dry run: shows what's merged, unmerged, or still in use
yo sweep --apply    # remove the merged ones (branches kept)
```

It never removes a worktree with uncommitted changes, an open PR, or a
session that's running or would be resumed. `yo forget <name>` drops a session
from yo's records without touching its directory.

## 4. More than one repo

*When you want to start sessions in a repo without `cd`-ing into it, or use
PR links from repos you haven't cloned.*

### Registering repos

```sh
yo config add-repo ~/src/github.com/acme/backend
yo config add-repo acme/frontend      # clones it first, to ~/dev/src/github.com/acme/frontend
yo config                             # what yo knows about
```

Now a session can go to any of them from anywhere:

```sh
yo --repo=frontend my-fix
yo frontend my-fix                    # the same
yo                                    # outside any known clone: the default repo
```

Registered repos are written to `~/.config/yo/config.json`. You rarely need to
edit it by hand, but `yo config edit` opens it and checks it when you save.

### Repos yo doesn't know

A PR or issue link works for any repo. yo clones it if needed, then asks where
it belongs:

```
acme/widgets isn't configured. Where does it go?
  [1] add it to profile mmobeus
  [n] a new profile "acme" that claims every acme/* repo
  [g] just this once, as a guest (no shared memory, not pre-trusted)
choice [g]:
```

Choosing a profile (see [Profiles](#6-profiles)) registers it for good. A
**guest** is for code you haven't vetted: Claude asks before trusting the
folder, and the session doesn't share memory with your others.
`--add-to=PROFILE` or `--add-to=guest` answers in advance; with nobody at the
terminal to ask, it's a guest.

### Clones of a fork

If a clone's `origin` is your fork, keep the real repo as another remote
(`upstream`, conventionally). yo fetches PRs and the main branch from the real
repo, and your own branches from `origin`.

## 5. Working in place

*When you don't want a new worktree: handing work between sessions in the same
tree, or a directory that isn't a unit of work at all.*

```sh
yo --here - 'read HANDOFF.md and pick up where it left off'
```

The session runs where you are, on whatever is checked out; nothing is
fetched or created. Name a repo (`yo --here --repo=backend`) to run in its
clone instead. `yo down` in such a session only closes it; the directory is
left alone.

To make in place the default for a repo, set `"mode": "here"` on it in the
config; `--worktree` then asks for a worktree for one session.

## 6. Profiles

*When you work on more than one project and want each to have its own
settings: separate memory, a different claude command or permissions, its own
iTerm window.*

A profile groups repos and gives them shared settings:

```json
{
  "profiles": {
    "acme": {
      "owners": ["acme"],
      "memory": "acme",
      "windowGroup": "acme",
      "claude": "claude --permission-mode plan"
    }
  }
}
```

- **`owners`** makes every clone of an `acme/*` repo (under
  `~/dev/src/github.com/acme/`) part of the profile, and any `acme` PR or
  issue link too. Nothing else to list.
- Repos can also be listed under the profile's `repos`, and
  `yo config add-repo <path> --profile=acme` does that for you.
- `yo acme` or `yo --profile=acme -` starts a session in the profile from
  anywhere; give the profile a `dir` to say where.

Every profile setting is in the [reference](reference.md#profile). Settings
resolve from the most specific: command-line flag, then repo, then profile,
then `defaults`.

## 7. Coordinating sessions

*When one session should direct others: a standing session you check in with,
which hands work to workers that report back.*

### Reaching a session by name

```sh
yo my-fix 'rebase on main'
```

If the session is running, its tab comes to the front and your message is
typed in. If not, it's resumed with your message. Any of these names work:
yo's session name, a session id, or the session's Claude name, which its
statusline shows after 📡. Names that match nothing start a new session, as
always. The exact lookup order is in the [reference](reference.md#yo-name).

### A home session

A home is a standing session for the work that isn't any one repo: notes,
plans, shared memory. Make a repo for it called `NAME-home` and register it
to run in place (`yo config edit`):

```json
"repos": { "home": { "path": "~/dev/src/github.com/me/me-home", "mode": "here" } }
```

Then `yo NAME` reaches it:

```sh
yo me 'what is open this week?'
```

`yo me` brings forward its running session, else resumes the most recent one,
else starts one. Quit it with `/exit` to have the next `yo me` start fresh.

### Handing work out

From the home (or any session), start a worker that reports back:

```sh
yo --repo=backend --from=me-home --title='Fix retry' retry-fix '<the brief>'
```

`--from` tells the worker to `SendMessage` that session when it's done or
stuck. The brief is all the worker knows: the goal, links, constraints, and
what "done" means.

### Agents

*When a repo defines agents (`.claude/agents/*.md`) with their own
instructions.*

```sh
yo --agent=cindy https://github.com/acme/backend/issues/123   # work an issue as Cindy
yo agent cindy                    # Cindy's home
yo cindy 'how is the queue?'      # reach it, however it's doing
```

The agent's instructions replace the default ones, and its tabs and
codenames lead with its name. Only agents merged on the repo's main branch, or
installed in `~/.claude/agents`, are used. Register one under `"agents"` in the config to give it a default repo
and make `yo cindy` reach its home.

An agent's home is a standing session like the home above, with its own
checkout of the main branch (`<clone>-worktrees/agent-cindy`, refreshed when
it starts), run with Remote Control as `cindy-home`. It starts workers with
`--agent=cindy --from=cindy-home`. `yo down` and `yo sweep` leave it alone.

## 8. Under the hood

*What yo sets up for each session, for when you want to change it or need to
know why something behaves as it does.*

### What creating a session does

1. Fetches the main branch (or the PR head) and adds the worktree, reusing a
   local or remote branch of the same name if there is one.
2. Pre-trusts the worktree in `~/.claude.json`, so Claude doesn't ask, and
   copies the clone's `.claude/settings.local.json` (MCP approvals) plus any
   gitignored files the repo lists in `mirror`, such as `.env`.
3. Records the session in yo's ledger (`~/.local/state/yo/sessions/`), under
   an id yo picks, which a hook keeps current through `/clear` and `/exit`.
   The record holds what yo needs to bring the session back (directory, repo,
   profile, title, agent) and what it is: its codename and its brief, the
   first message it was given.
4. Opens the tab and starts claude with yo's settings layer and your message.

### The settings layer

yo never edits `~/.claude/settings.json`; Claude Code and iTerm2 both write
it. Each session gets `claude --settings <file>` instead, composed from yo's
`claude-settings.json`, the profile's `settings` file, the session's memory
directory, and the ledger's hooks. It outranks your settings files, and lists
like permissions merge with yours. `yo setup --merge-settings` applies the
same settings to every claude session.

### Memory

Every yo session shares one auto-memory, across repos and worktrees, at
`~/.claude/memories/yo`. Point `"memories"` in the config's `defaults` at a
repo to keep it in git. A profile or repo can have its own store
(`"memory": "acme"`), `"none"` leaves Claude its per-folder memory, and
`"memories": ""` turns the shared memory off.

### Skills and the statusline

Three skills let claude drive yo, which is how a session is managed through
Remote Control:

| skill | does |
| --- | --- |
| `/yo-down` | tears this session down |
| `/yo-adopt` | records a session yo didn't start, so it survives restarts |
| `/yo-title` | retitles this session's tab |

The statusline shows the model, the session's Claude name (📡, what you'd
give `--from` or `yo NAME`), the branch, context used, cost and time.

### iTerm2

- **One window, many tabs.** Sessions open as tabs in one shared window, so
  iTerm's Session Status tool, which lists one window, shows them all.
  `--new-window` opts out once, `"windowMode": "window"` always, and
  `"windowGroup"` names the window (`{repo}` in it gives one per repo).
- **Layout.** With iTerm2's Claude Code workgroup set up with a terminal peer
  ("CLI"), claude has the tab to itself and the peer is the shell. Otherwise
  the tab is split: claude left, a shell right. `yo setup` adds the peer;
  it appears after iTerm2 restarts.
- **Titles that stick.** claude rewrites the terminal title constantly, so yo
  uses an iTerm profile (`worksesh`) that ignores that and pins the tab title.

## 9. Troubleshooting

**`yo forget` says the session "is still running" right after I closed its
tab.** Claude takes a moment to exit. The forget worked.

**Claude asks whether to trust the folder.** The repo is a guest, or its
profile has `"trust": false`, or it's an in-place session in a directory yo
didn't create. That's deliberate; answer it once.

**A message didn't reach a running session.** yo finds a session's tab by its
terminal; a session running only through Remote Control has none. yo says so
and suggests another way to reach it. If the tab is showing CLI, yo brings it
forward but doesn't type into the shell: switch it to the chat.

**`yo sessions` says `running (no tab)`.** The session's claude is still
running, but its tab is gone. iTerm2 does this when a tab with the Claude
Code workgroup is closed while it shows CLI: only the CLI shell is ended,
and the chat runs on with no tab (it isn't under Session › Buried Sessions
either). `resume-all` lists these instead of resuming them. End one with
`kill -HUP <pid>` (the pid `yo sessions` shows), then `yo resume <name>`.
`yo down` from the CLI ends claude itself, so it doesn't leave one behind.

**`worktree path already exists`.** A session by that name already has a
worktree. If yo knows the session (`yo sessions`), `yo my-fix 'message'`
reaches it instead. Otherwise pick another name, or `yo sweep` to clean up.

**The tab says "Chat".** iTerm2's Claude Code workgroup renamed it. `yo title`
with no argument restores every yo tab's name.

**`yo` says the config failed to load.** The message names the bad key or the
JSON error. `yo config edit` checks the file when you save.

**Seeing what yo would do.** `--dry-run` (on starting a session, `yo NAME`,
`yo agent`, `down` and `resume-all`) prints the plan and changes nothing.

**Testing yo itself.** Build it (`cargo build`) and run `target/debug/yo` by
path with `YO_DRY_LAUNCH=1` to print the launch instead of opening iTerm. yo
still records the session before launching, so point `HOME`,
`XDG_STATE_HOME` and `YO_CONFIG` at a throwaway directory too.
