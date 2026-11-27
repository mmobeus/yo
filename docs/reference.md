# yo reference

Every command, flag, config key and file, in one place. The
[guide](guide.md) explains how they fit together; `yo help` prints yo's own
summary plus the repos, profiles and agents it can see, and `yo --help` and
`yo <command> --help` list each command's flags. A flag's value can be given
as `--flag=value` or `--flag value`.

- [Commands](#commands)
- [Flags for starting a session](#flags-for-starting-a-session)
- [`yo NAME`](#yo-name)
- [Config file](#config-file)
- [Environment](#environment)
- [Files and locations](#files-and-locations)
- [Changes from the bash yo](#changes-from-the-bash-yo)

## Commands

| command | what it does |
| --- | --- |
| `yo [flags] [INPUT] [message]` | start a session (see [flags](#flags-for-starting-a-session)) |
| `yo NAME ['message']` | reach a session, agent home or home by name ([`yo NAME`](#yo-name)) |
| `yo down [--dry-run] [--yes [--discard]]` | tear down the session you're in: remove its worktree (branch kept), forget it, end its claude if it's still running in this tab, close its tab. Aliases `rm`, `teardown` |
| `yo sessions` | list the ledger: running, running (no tab), ready, ended, gone, no-id, waiting? (a Codex session with no id yet, a while after launch: likely stopped at a prompt in its tab). Alias `ls` |
| `yo resume [<name\|id\|dir>] [--model=M] [--new-window\|--tab]` | reopen one session; bare, list them |
| `yo resume-all [--dry-run] [--new-window\|--tab]` | reopen every session that was live; waits a few seconds for sessions whose tab was just closed, and lists those still running without a tab instead of resuming them |
| `yo title ["T"]` | retitle the session this runs in; bare, reset every yo tab's name |
| `yo adopt [--title=T] [--session-id=ID]` | record the session this runs in |
| `yo forget <name\|id\|dir>` | drop a session from the ledger |
| `yo agent [NAME] [--model=M] [--new-window\|--tab] [--dry-run] ['message']` | start or reach agent NAME's home; bare, list agents |
| `yo sweep [--apply] [--repo=R]... [--include-stale] [--include-suspended] [--delete-branches]` | find (and remove) worktrees whose work has landed |
| `yo config [show]` | the config file, and every repo, profile and agent it registers |
| `yo config path` | the config file's path |
| `yo config edit` | open it in `$EDITOR`, then validate |
| `yo config add-repo <dir\|url\|owner/repo> [--profile=P] [--key=K]` | list a repo, cloning it first unless it's a directory |
| `yo setup [--dry-run] [--uninstall] [--link=CHECKOUT] [--merge-settings] [--no-iterm] [--force] [--hooks]` | install what yo needs besides its binary ([`yo setup`](#yo-setup)) |
| `yo help` | yo's guide, and what's configured |
| `yo --version` | yo's version (`0.0.0-dev` when built from a checkout) |

### `yo setup`

Installs the skills, the settings layer and the statusline from copies built
into yo (Codex's copies of the skills too, into `$CODEX_HOME/skills`, when
Codex has its home, `~/.codex`), creates the config if there is none (with no repos, or listing the
`--link` checkout as `yo`), and adds yo's workgroup to iTerm2 if it isn't
there: the session, with a shell as its terminal peer ("CLI"). The profile yo
opens tabs with enters that workgroup for every harness's program, in place
of the workgroup triggers it copies from the Default profile (the Default
profile itself isn't changed). It's safe to run again:

- what's already in place is left alone, and what setup installed before is
  replaced;
- anything else in the way is moved aside to `<path>.bak-<time>` first;
- a link that points somewhere yo didn't make is left alone (`--force`
  replaces it);
- `~/.local/bin/yo`, if it's a link to a checkout's build (an old
  `install.sh`), is removed, unless it's the yo running.

If Codex is installed, setup offers to add yo's ledger hook to
`~/.codex/hooks.json`: a `SessionEnd` hook running `yo _session-hook codex`,
beside any hooks already there (the file is backed up first). It runs for
every Codex session, so at a terminal setup asks, and `--hooks` adds it
without asking; otherwise it's left out. A hook that's there is kept current
on every run, and `--uninstall` takes it out. Codex runs a hook only after
it's approved in Codex's hook review, so setup then offers to open `codex` for
that (`codex` itself, not `harnesses.codex.command`): at "Hooks need review", choose "Trust all and continue" (or "Review
hooks" to trust only yo's), then `/quit`.

What it installed is recorded in `~/.local/state/yo/setup.json`. When a newer
yo has different copies, every other command says to run `yo setup` again.

| flag | effect |
| --- | --- |
| `--dry-run`, `-n` | say what it would do, and change nothing |
| `--uninstall` | take out what setup installed: its copies (unless edited since) and links into a yo checkout, and yo's iTerm2 workgroup (unless `--no-iterm`). The config and yo itself stay |
| `--link=CHECKOUT` | link the files into a yo checkout instead of copying them, so edits there are live (for working on yo) |
| `--merge-settings` | also merge yo's Claude settings into `~/.claude/settings.json`, for every claude session: yo's keys win, objects merge key by key, the rest of the file stays (backed up first). With `--uninstall`, its top-level keys are taken back out |
| `--no-iterm` | leave iTerm2's workgroups alone. Otherwise iTerm2 sees yo's workgroup (or, uninstalling, its going) after it restarts (the old workgroups are saved to `~/.config/yo/iterm-workgroups.bak-<time>.json`) |
| `--force` | replace links that point somewhere unexpected |
| `--hooks` | add yo's ledger hook to `~/.codex/hooks.json` without asking |

### INPUT

| INPUT | session |
| --- | --- |
| omitted, or `-` | `sesh_<YYYYmmdd_HHMMSS>` (`_2`, `_3`, ... if a session already has it), titled with its codename |
| `.../pull/<n>` URL | `review_pr_<n>`: a worktree at the PR's head that tracks `refs/pull/<n>/head`, seeded with `<reviewCmd> <url>` |
| `.../issues/<n>` URL | `issue_<n>` off the base branch, seeded to work the issue |
| an existing branch | that branch, checked out as-is |
| anything else | a new branch `<INPUT>` off the base branch (`<prefix>/<INPUT>` with a `branchPrefix`) |

`yo <repo> [INPUT] [message]` and `yo <profile> [INPUT] [message]` pick the
repo or profile positionally.

Claude is told its codename at the end of the first message. When that
message is a slash command (it starts with `/`, as a PR session's
`<reviewCmd> <url>` does), everything after the command is its arguments,
so yo leaves the message as it is and passes the codename with
`--append-system-prompt` instead.

### `yo sweep` verdicts

| verdict | meaning | removed by `--apply` |
| --- | --- | --- |
| `DEAD` | PR merged, no commits beyond base, or a clean PR-review checkout | yes |
| `STALE` | unmerged commits | with `--include-stale` |
| `SUSP` | a session `resume-all` would bring back | with `--include-suspended` |
| `KEEP` | uncommitted changes, an open PR, a live session, the current directory, or an agent's home | never |

## Flags for starting a session

| flag | effect |
| --- | --- |
| `--repo=R` | the repo, instead of the one the cwd is in |
| `--profile=P` | run under profile P (beats the repo's own) |
| `--here` (`--in-place`) | no worktree: the named repo's clone, else the cwd |
| `--worktree` | a worktree, where in place would be the default |
| `--title="T"` | the tab and Session Status title |
| `--from=SESSION` | report back to that session when done or stuck (a Claude session uses `SendMessage`; a Codex one runs `yo SESSION 'report'`) |
| `--model=M` | this session's model (`opus`, `sonnet`, `haiku`, `opus[1m]`, a full id) |
| `--agent=NAME` | run as agent NAME; it must be merged on the base branch or in `~/.claude/agents` |
| `--harness=H` | the coding agent to run it on (see [Harnesses](#harnesses)) |
| `--add-to=PROFILE\|guest` | where an unconfigured URL's repo goes, without asking |
| `--new-window` (`--window`), `--tab` | a window of its own, or a tab in the shared one |
| `--dry-run` (`-n`) | print the plan and change nothing |

## `yo NAME`

With no flags and at most a message, `yo NAME ['message']` tries, in order:

1. **A repo or profile named NAME:** its usual meaning, a new session there.
2. **A registered agent:** its home, as `yo agent NAME`.
3. **A live session whose Claude name is NAME** (a Remote Control name like
   `cindy-home` counts).
4. **A ledger session** by yo name, session id or id prefix (8+ characters).
5. **A repo keyed `NAME-home`, or whose clone's directory is `NAME-home`:** the
   session running there, else the most recently used ready one, else a new
   one in place. Sessions quit with `/exit` are skipped.
6. **Otherwise:** a new session named NAME.

Reaching a running session brings its iTerm tab forward and types the message
in, then Enter. If its tab is showing the workgroup's CLI, the tab comes
forward but nothing is typed. If it has no tab, yo says how else to reach it,
or to end it and resume it. Reaching a stopped one resumes it with the message as its first
prompt. Any flag skips steps 2–5, except `--dry-run`.

## Config file

`$YO_CONFIG`, else `${XDG_CONFIG_HOME:-~/.config}/yo/config.json`. Unknown keys
are an error. A leading `~/` in a path is your home directory.

```json
{
  "defaults": { ... },
  "profiles": { "NAME": { ..., "repos": {}, "reposIn": [], "agents": {} } },
  "repos":    { "KEY": { ... } },
  "reposIn":  [ "DIR" ],
  "agents":   { "NAME": { ... } },
  "harnesses": { "NAME": { "command": "...", "args": [ ... ] } }
}
```

Precedence: command-line flag, then repo, then profile, then `defaults`. A
setting's environment variable, if set, beats the file.

### `defaults`

| key | sets | default | env |
| --- | --- | --- | --- |
| `branchPrefix` | prefix for branches yo creates, as `<prefix>/<name>` | none | `YO_BRANCH_PREFIX` |
| `mode` | `worktree` or `here` | `worktree` | `YO_MODE` |
| `harness` | the harness new sessions run on | `claude` | `YO_HARNESS` |
| `defaultRepo` | the repo used outside any known clone | the first repo | `YO_DEFAULT_REPO` |
| `claude` | the claude harness's command (beats `harnesses.claude.command`) | `claude --permission-mode auto` | `YO_CLAUDE_CMD` |
| `memories` | the shared memory root; `""` turns shared memory off | `~/.claude/memories` | `YO_MEMORIES` |
| `memory` | the store inside it | `yo` | `YO_MEMORY_STORE` |
| `settings` | yo's base settings layer | `claude-settings.json` beside the config | `YO_CLAUDE_SETTINGS` |
| `cloneRoot` | where unknown repos are cloned, and where owners are scanned | `~/dev/src/github.com` | `YO_CLONE_ROOT` |
| `cloneProtocol` | `ssh` or `https` | `ssh` | `YO_CLONE_PROTOCOL` |
| `guestProfile` | a profile to run guests under | none | `YO_ADHOC_PROFILE` |
| `reviewCmd` | the command a PR session is seeded with | `/code-review` | `YO_REVIEW_CMD` |
| `layout` | `auto`, `split` or `workgroup` | `auto` | `YO_LAYOUT` |
| `windowMode` | `tab` or `window` | `tab` | `YO_WINDOW_MODE` |
| `windowGroup` | the shared window's label; `{repo}` expands | `yo` | `YO_WINDOW_GROUP` |
| `winWidth`, `winHeight` | a new window's size | `1403`, `824` | `YO_WIN_WIDTH`, `YO_WIN_HEIGHT` |
| `itermProfile` | the generated title-locking iTerm profile | `worksesh` | `YO_ITERM_PROFILE` |

### Profile

| key | sets |
| --- | --- |
| `owners` | GitHub owners whose clones (`<cloneRoot>/<owner>/*`) and URLs join this profile |
| `repos` | repos in it: `{ KEY: REPO }` |
| `reposIn` | directories whose clones join it, keyed by directory name |
| `agents` | agents: `{ NAME: AGENT }` |
| `mode` | `here` or `worktree` |
| `harness` | the harness its sessions run on |
| `memory` | its memory store: a name inside `memories`, an absolute path, or `none` |
| `branchPrefix` | prefix for branches |
| `claude` | the claude command |
| `settings` | a Claude settings file stacked over yo's layer |
| `dir` | its home directory, used when no repo is named |
| `windowGroup` | the shared window its sessions join |
| `trust` | `false`: don't pre-trust its worktrees |

### Repo

| key | sets |
| --- | --- |
| `path` | the clone (required) |
| `base` | the base branch; default, the clone's default branch |
| `github` | `owner/repo`; default, worked out from the remotes |
| `mcp` | MCP servers from the repo's `.mcp.json` to pre-approve |
| `mirror` | gitignored files to copy into each worktree, such as `.env` |
| `titlePrefix` | tab title prefix; default `[key] `, none for the default repo |
| `memory`, `branchPrefix`, `mode`, `harness` | as for a profile, for this repo |

### Agent

| key | sets |
| --- | --- |
| `repo` | the repo its sessions default to, and where its home lives (required) |
| `title` | the name tabs and codenames lead with; default, NAME capitalized |

### Harnesses

A harness is the coding agent a session runs: `claude` (Claude Code) or
`codex` (the Codex CLI). A
session's harness is `--harness`, else its repo's `harness`, its profile's,
then `defaults.harness`. The ledger records it, so resuming and reaching a
session use the harness it started on. An agent's home runs on its repo's
harness.

`harnesses.NAME` sets how yo runs one:

| key | sets |
| --- | --- |
| `command` | its command, as shell words; default, the harness's own (`claude --permission-mode auto`, `codex`) |
| `args` | arguments added after the command, each one word |

For `claude`, the older `claude` setting (a profile's, `defaults.claude`,
`YO_CLAUDE_CMD`) beats `harnesses.claude.command`.

A flag the session's harness has nothing for (`--agent`, `--model`) is an
error that names the harness, never ignored.

What each harness gets:

| | `claude` | `codex` |
| --- | --- | --- |
| session id | yo picks it (`--session-id`) | Codex picks it; yo reads it from Codex's records (`~/.codex/sessions`) the next time it looks at the ledger |
| the codename | at the end of the first message | as developer instructions (`-c developer_instructions=...`) |
| no message | asked to introduce itself | the same, which is also what makes Codex record the session |
| `--model` | yes | yes (`-m`) |
| `--agent`, agents' homes | yes | no |
| settings layer, shared memory | yes | no |
| trust | a worktree of a trusted repo is pre-trusted | Codex asks once per repo (its main clone), and its worktrees inherit that. For a worktree of a trusted repo it hasn't trusted, yo asks at a terminal and on a yes adds the clone to `~/.codex/config.toml`; otherwise it warns that Codex will ask in the tab |
| running or not | `~/.claude/sessions` | the thread lock a running `codex` holds (`~/.codex/thread-writer-locks`), found with `lsof` |
| quit deliberately | `/exit` marks it ended | with yo's hook (`yo setup` adds it, when asked, and Codex approves it), `/quit` and ctrl-d mark it ended. Without it, a quit session stays ready, so `resume-all` brings it back |
| messages | typed into its tab; a stopped one is resumed with it | the same (`codex resume ID MESSAGE`) |
| `--from` report back | with `SendMessage` | by running `yo SESSION 'report'`, which types it in |
| inside a session | `CLAUDE_CODE_SESSION_ID` | `CODEX_THREAD_ID` |

## Environment

| variable | effect |
| --- | --- |
| `YO_CONFIG` | the config file to read |
| `YO_*` in the `defaults` table | override that setting for one run |
| `YO_DRY_LAUNCH=1` | print the tab title, directory and harness command instead of opening iTerm (the ledger is still written) |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME` | move yo's config and state |
| `YO_SESSION_KEY` | set by yo on the harness it launches: the session's ledger record |

## Files and locations

| what | where |
| --- | --- |
| config | `~/.config/yo/config.json` |
| base settings layer, statusline | `~/.config/yo/claude-settings.json`, `~/.config/yo/statusline.sh` |
| ledger | `~/.local/state/yo/sessions/*.json`: one record per session (id, directory, repo, profile, mode, title, codename, brief, agent, and `tab`: the iTerm tab it was last opened in, `<window id>/<tab id>`, recorded at launch and by `yo adopt`) |
| composed settings, per session label | `~/.local/state/yo/settings/<label>.json` |
| shared memory | `~/.claude/memories/yo`, or `<memories>/<memory>` |
| skills | `~/.claude/skills/yo-down`, `yo-adopt`, `yo-title`; for Codex, the same names in `$CODEX_HOME/skills` (`~/.codex/skills`) |
| what `yo setup` installed | `~/.local/state/yo/setup.json` |
| worktrees | `<clone>-worktrees/<name>`, branch `<name>` (or `<prefix>/<name>`) |
| an agent's home | `<clone>-worktrees/agent-<name>`, Remote Control and session name `<name>-home` |
| unknown repos | `<cloneRoot>/<owner>/<repo>` |
| iTerm title-lock profile | `~/Library/Application Support/iTerm2/DynamicProfiles/worksesh.json` |

## Changes from the bash yo

`yo` was a bash script until phase 1 of the [core design](design/core.md)
replaced it with a Rust binary. What a command does is the same; these are
the differences you'd notice (the design's "Testing the port" has the full
list):

- yo is installed with Homebrew (or `cargo install`), and `yo setup` does
  what `install.sh` did besides installing the binary; it needs neither
  python3 nor jq itself. Its files are copies built into yo unless `--link` is given; there's
  no `--copy`, `--bin`, `--claude-dir` or `--config-dir` (`HOME` and
  `XDG_CONFIG_HOME` move them), and the starting config lists no repos
  unless `--link` names a checkout.
- Flags are parsed by clap: `--flag value` works as well as `--flag=value`,
  usage errors read differently, and there's `yo --help` and
  `yo <command> --help`. `yo help` still prints the guide and the registry.
- `yo -d` and `yo --resume` are gone: use `yo down` (or `rm`, `teardown`)
  and `yo resume`.
- `--discard` needs `--yes`; `--here` and `--worktree` can't both be given.
- Starting a session, `yo NAME`, `yo agent`, `down` and `resume-all` take
  `--dry-run`.
- A session name with an empty or dot-only part, or a part starting with
  `-`, is refused.
- A bad config's error names the key and where it is in the file.
- Create's progress reads a little differently: `codename:`, `session:` and
  `brief:` lines, and no git chatter.
- `down --yes` with uncommitted work refuses up front, naming the changes.
- `sweep` keeps a worktree whose `git status` can't be read.
- An agent's home in an untrusted repo gets no copied `settings.local.json`
  or `mirror` files.
