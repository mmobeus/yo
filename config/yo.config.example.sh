# yo configuration — everything site-specific lives here.
#
# Sourced by `yo` from $YO_CONFIG, else ${XDG_CONFIG_HOME:-~/.config}/yo/config.sh.
# install.sh copies this example there once and never touches it again: that
# file is yours, and it stays out of the yo repo.
#
# If no config file exists at all, `yo` falls back to single-repo mode: the git
# clone you are standing in is the repo, and its default branch (origin/HEAD) is
# the base branch. That makes `yo` usable with zero configuration; everything
# below is about teaching it more than one repo.

# --- repo registry ------------------------------------------------------------
#
#   yo_repo <key> --path=<clone> [options]
#
#     --path=PATH           (required) the primary clone. Worktrees are created
#                           as siblings under <PATH>-worktrees/.
#     --base=REF            branch new sessions fork from, and what teardown's
#                           "unpushed commits" check compares against.
#                           Default: the clone's origin/HEAD, else main.
#     --github=OWNER/REPO   used to map a PR/issue URL back to this repo.
#                           Default: the clone's origin remote, falling back to
#                           the checkout path if it looks like
#                           .../github.com/<owner>/<repo>. Set it explicitly for
#                           a clone kept somewhere that says neither.
#     --mcp="a b c"         MCP servers (from the repo's .mcp.json) to
#                           pre-approve in every worktree, so Claude doesn't
#                           prompt. Space-separated.
#     --mirror="a b"        gitignored, machine-local files to copy from the
#                           clone into each new worktree (e.g. .env). Refused
#                           if the file is NOT gitignored in the worktree, so a
#                           secret can never end up staged.
#     --title-prefix=S      literal iTerm window-title prefix. Default: "[key] "
#                           for every repo except the default one (below), which
#                           gets none.
#     --memory=STORE        a memory store of this repo's own, instead of the
#                           one every session shares (see YO_MEMORIES below).
#                           A name inside YO_MEMORIES, or an absolute path, or
#                           `none` to leave Claude its own per-folder memory.
#     --branch-prefix=S     prefix for the branches yo creates in this repo.
#                           Default: YO_BRANCH_PREFIX.
#     --profile=NAME        the profile (below) this repo belongs to. Its
#                           settings apply to every session in the repo;
#                           options given here still win over the profile's.
#     --mode=here|worktree  how sessions in this repo run. `here` means in
#                           place, in the clone itself: no worktree, no branch,
#                           nothing fetched. Default: YO_MODE.
#                           Override per-invocation with --here / --worktree.
#
#   yo_repos_in <dir> [options]
#
#     Registers every git clone directly under <dir>, keyed by directory name.
#     Any yo_repo options given (except --path) apply to each of them. Keys
#     already registered are skipped, so a repo that needs options of its own
#     gets its yo_repo line first.
#
# --- profiles -----------------------------------------------------------------
#
#   yo_profile <name> [options]
#
#   A profile is one of YOUR projects: a named bundle of defaults shared by the
#   repos that join it (yo_repo --profile=NAME), and usable on its own with
#   `yo --profile=NAME` or `yo NAME`. Order doesn't matter — a repo may name a
#   profile that is defined further down.
#
#     --mode=here|worktree  as for yo_repo.
#     --memory=STORE        the profile's own memory store: a name inside
#                           YO_MEMORIES, or an absolute path.
#     --branch-prefix=S     as for yo_repo.
#     --claude="CMD"        command launched in the claude pane, e.g. to pick a
#                           permission mode or a model. Default: YO_CLAUDE_CMD.
#     --settings=FILE       a Claude settings JSON file (permissions, hooks,
#                           env, ...) stacked over yo's own layer for these
#                           sessions — objects merge, lists are appended to.
#     --dir=PATH            the profile's home: where `yo --profile=NAME` goes
#                           when no repo is named. Needn't be a git clone; if
#                           it isn't (or isn't registered), the session runs
#                           in place there.
#     --window-group=LABEL  the shared iTerm window its sessions join.
#                           Default: YO_WINDOW_GROUP.
#
#   Without --dir, `yo --profile=NAME` uses the cwd's repo if it belongs to the
#   profile, else the first repo registered under it.
#
#   Precedence, for everything: command-line flag, then the yo_repo option,
#   then the profile, then the global setting further down.
#
# Examples:
#
#   yo_profile acme  --memory=acme --window-group=acme
#   yo_repo backend  --path=... --profile=acme
#   yo_repo frontend --path=... --profile=acme
#
#   yo_profile notes --dir="$HOME/notes" \
#     --memory=notes --settings="$HOME/.config/yo/notes.settings.json"

# The first repo registered is the default: it's what bare `yo` uses when the
# cwd isn't inside a known clone. Override with YO_DEFAULT_REPO.

#yo_repo backend \
#  --path="$HOME/src/github.com/acme/backend" \
#  --base=dev \
#  --mcp="grafana" \
#  --mirror=".env"
#
#yo_repos_in "$HOME/src/github.com/acme"
#yo_repos_in "$HOME/src/scratch" --mode=here

# --- settings -----------------------------------------------------------------

# Prefix for branches yo creates: <prefix>/<session-name>. Default: $USER.
# A repo's --branch-prefix wins.
#YO_BRANCH_PREFIX="me"

# How sessions run when the repo doesn't say (--mode):
#   worktree  (default) a fresh worktree and branch per session.
#   here      in place: the named repo's clone, or the cwd when no repo was
#             named. Nothing is fetched, so yo warns when a PR/issue URL is
#             opened this way.
#YO_MODE="worktree"

# Repo bare `yo` falls back to when the cwd isn't inside a known clone.
# Default: the first repo registered above.
#YO_DEFAULT_REPO="backend"

# Command launched in the left pane.
#YO_CLAUDE_CMD="claude --permission-mode auto"

# Slash command seeded into a PR-review session, as "<cmd> <pr-url>".
#YO_REVIEW_CMD="/code-review"

# --- unconfigured repos -------------------------------------------------------

# A PR or issue URL for a repo that isn't registered above still works: yo
# clones it to <YO_CLONE_ROOT>/<owner>/<repo> (or uses the clone already there)
# and gives the session a worktree. Default root: ~/dev/src/github.com.
#YO_CLONE_ROOT="$HOME/dev/src/github.com"

# How such a repo is cloned: ssh (git@github.com:...) or https.
#YO_CLONE_PROTOCOL="ssh"

# Those sessions are guests: no shared memory, and no pre-trusted worktree, so
# Claude asks before trusting the repo's own hooks and MCP servers. To give
# them settings of their own, name a profile here; its --memory applies
# (the trust prompt stays either way).
#YO_ADHOC_PROFILE="guest"

# --- memory -------------------------------------------------------------------

# Every yo session shares ONE memory — across repos, worktrees and plain
# directories — instead of Claude's per-folder stores under ~/.claude/projects/.
# It lives at <YO_MEMORIES>/<YO_MEMORY_STORE>, by default ~/.claude/memories/yo.
#
# To keep it in a repo of your own, point YO_MEMORIES at the checkout:
#YO_MEMORIES="$HOME/src/github.com/me/memories"
# Set it EMPTY to turn all this off and leave Claude's own memory alone:
#YO_MEMORIES=""

# The shared store's name inside YO_MEMORIES. A repo's own --memory wins.
#YO_MEMORY_STORE="yo"

# The settings layer stacked onto every session with `claude --settings`.
# Default: claude-settings.json next to this file (install.sh links it there).
#YO_CLAUDE_SETTINGS="$HOME/.config/yo/claude-settings.json"

# --- iTerm2 -------------------------------------------------------------------

# What a session's tab holds:
#   split      claude on the left, a shell in the same directory on the right.
#   workgroup  claude alone. The shell is the terminal peer ("CLI") of iTerm2's
#              Claude Code workgroup, reached from the workgroup's mode
#              switcher. install.sh adds that peer if it's missing.
#   auto       (default) workgroup if that peer is configured, else split.
#YO_LAYOUT="auto"

# New-window size in points; the window is centered on the primary display.
#YO_WIN_WIDTH=1403
#YO_WIN_HEIGHT=824

# Name of the generated iTerm2 dynamic profile that locks the window title.
#YO_ITERM_PROFILE="worksesh"

# Where a new session goes:
#   tab     (default) a tab in one shared window. iTerm2's Session Status
#           toolbelt lists the sessions of a single window, so keeping them
#           together turns it into a dashboard of every session.
#   window  one window per session (how yo behaved before).
# Override per-invocation with --new-window / --tab.
#YO_WINDOW_MODE="tab"

# Which shared window a session joins. Any label works; {repo} expands to the
# repo key, so "yo-{repo}" gives one window per repo.
#YO_WINDOW_GROUP="yo"
