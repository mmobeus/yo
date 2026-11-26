# yo reference

Every command, flag, config key and file, in one place. The
[guide](guide.md) explains how they fit together; `yo help` prints the
script's own summary plus the repos, profiles and agents it can see.

- [Commands](#commands)
- [Flags for starting a session](#flags-for-starting-a-session)
- [`yo NAME`](#yo-name)
- [Config file](#config-file)
- [Environment](#environment)
- [Files and locations](#files-and-locations)

## Commands

| command | what it does |
| --- | --- |
| `yo [flags] [INPUT] [message]` | start a session (see [flags](#flags-for-starting-a-session)) |
| `yo NAME ['message']` | reach a session, agent home or home by name ([`yo NAME`](#yo-name)) |
| `yo down [--dry-run] [--yes [--discard]]` | tear down the session you're in: remove its worktree (branch kept), forget it, close its tab. Aliases `rm`, `teardown`, `-d` |
| `yo sessions` | list the ledger: running, ready, ended, gone. Alias `ls` |
| `yo resume <name\|id\|dir> [--model=M] [--new-window\|--tab]` | reopen one session. Alias `--resume` |
| `yo resume-all [--dry-run] [--new-window\|--tab]` | reopen every session that was live |
| `yo title ["T"]` | retitle the session this runs in; bare, reset every yo tab's name |
| `yo adopt [--title=T] [--session-id=ID]` | record the session this runs in |
| `yo forget <name\|id\|dir>` | drop a session from the ledger |
| `yo agent [NAME] [--model=M] [--new-window\|--tab] ['message']` | start or reach agent NAME's home; bare, list agents |
| `yo sweep [--apply] [--repo=R]... [--include-stale] [--include-suspended] [--delete-branches]` | find (and remove) worktrees whose work has landed |
| `yo config [show]` | the config file, and every repo, profile and agent it registers |
| `yo config path` | the config file's path |
| `yo config edit` | open it in `$EDITOR`, then validate |
| `yo config add-repo <dir\|url\|owner/repo> [--profile=P] [--key=K]` | list a repo, cloning it first unless it's a directory |
| `yo help` | the script's help, and what's configured |

### INPUT

| INPUT | session |
| --- | --- |
| omitted, or `-` | `sesh_<YYYYmmdd_HHMMSS>`, titled with its codename |
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
| `--from=SESSION` | report back to that Claude session with `SendMessage` when done or stuck |
| `--model=M` | this session's model (`opus`, `sonnet`, `haiku`, `opus[1m]`, a full id) |
| `--agent=NAME` | run as agent NAME; it must be merged on the base branch or in `~/.claude/agents` |
| `--add-to=PROFILE\|guest` | where an unconfigured URL's repo goes, without asking |
| `--new-window` (`--window`), `--tab` | a window of its own, or a tab in the shared one |

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
in, then Enter. Reaching a stopped one resumes it with the message as its first
prompt. Any flag skips steps 2–5.

## Config file

`$YO_CONFIG`, else `${XDG_CONFIG_HOME:-~/.config}/yo/config.json`. Unknown keys
are an error. A leading `~/` in a path is your home directory.

```json
{
  "defaults": { ... },
  "profiles": { "NAME": { ..., "repos": {}, "reposIn": [], "agents": {} } },
  "repos":    { "KEY": { ... } },
  "reposIn":  [ "DIR" ],
  "agents":   { "NAME": { ... } }
}
```

Precedence: command-line flag, then repo, then profile, then `defaults`. A
setting's environment variable, if set, beats the file.

### `defaults`

| key | sets | default | env |
| --- | --- | --- | --- |
| `branchPrefix` | prefix for branches yo creates, as `<prefix>/<name>` | none | `YO_BRANCH_PREFIX` |
| `mode` | `worktree` or `here` | `worktree` | `YO_MODE` |
| `defaultRepo` | the repo used outside any known clone | the first repo | `YO_DEFAULT_REPO` |
| `claude` | the claude command | `claude --permission-mode auto` | `YO_CLAUDE_CMD` |
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
| `memory`, `branchPrefix`, `mode` | as for a profile, for this repo |

### Agent

| key | sets |
| --- | --- |
| `repo` | the repo its sessions default to, and where its home lives (required) |
| `title` | the name tabs and codenames lead with; default, NAME capitalized |

## Environment

| variable | effect |
| --- | --- |
| `YO_CONFIG` | the config file to read |
| `YO_*` in the `defaults` table | override that setting for one run |
| `YO_DRY_LAUNCH=1` | print the tab title, directory and claude command instead of opening iTerm (the ledger is still written) |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME` | move yo's config and state |
| `YO_SESSION_KEY` | set by yo on the claude it launches: the session's ledger record |

## Files and locations

| what | where |
| --- | --- |
| config | `~/.config/yo/config.json` |
| base settings layer, statusline | `~/.config/yo/claude-settings.json`, `~/.config/yo/statusline.sh` |
| ledger | `~/.local/state/yo/sessions/*.json`: one record per session (id, directory, repo, profile, mode, title, codename, brief, agent) |
| composed settings, per session label | `~/.local/state/yo/settings/<label>.json` |
| shared memory | `~/.claude/memories/yo`, or `<memories>/<memory>` |
| skills | `~/.claude/skills/yo-down`, `yo-adopt`, `yo-title` |
| worktrees | `<clone>-worktrees/<name>`, branch `<name>` (or `<prefix>/<name>`) |
| an agent's home | `<clone>-worktrees/agent-<name>`, Remote Control and session name `<name>-home` |
| unknown repos | `<cloneRoot>/<owner>/<repo>` |
| iTerm title-lock profile | `~/Library/Application Support/iTerm2/DynamicProfiles/worksesh.json` |
