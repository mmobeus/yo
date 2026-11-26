#!/usr/bin/env bash
#
# install.sh — wire this checkout into the places the tools expect to find it.
#
# Everything is installed as a SYMLINK back into this repo, so editing a file
# here takes effect immediately and `git status` is the source of truth for what
# has drifted. The one exception is yo's config, which names YOUR repos: it is
# created once, as a plain config.json listing this checkout, and never touched
# again — not by a re-install, not by --uninstall.
#
# Claude Code's own ~/.claude/settings.json is left alone by default: yo stacks
# claude/settings.json on top of it per session (`claude --settings`), so only
# sessions yo launches see it. --merge-settings is for wanting those settings in
# EVERY claude session: it merges them into ~/.claude/settings.json (this repo's
# keys win; the file is never replaced, so machine-local settings survive).
#
# usage:
#   ./install.sh [options]
#
#   --dry-run            print the plan; touch nothing
#   --copy               install copies instead of symlinks (for a machine that
#                        shouldn't depend on this checkout staying put)
#   --uninstall          remove what we installed (see the note on settings.json)
#   --force              replace symlinks that point somewhere unexpected
#   --merge-settings     also merge claude/settings.json into
#                        ~/.claude/settings.json (with --uninstall: take the
#                        merged keys back out)
#   --no-iterm           leave iTerm2's Claude Code workgroup alone
#   --bin=PATH           where scripts go        (default: ~/.local/bin)
#   --claude-dir=PATH    Claude Code's home      (default: ~/.claude)
#   --config-dir=PATH    XDG config root         (default: ~/.config)
#   -h, --help           this
#
# Existing real files are backed up to <path>.bak-<timestamp> before being
# replaced — nothing is destroyed silently.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TS="$(date +%Y%m%d-%H%M%S)"

DRY=0; COPY=0; UNINSTALL=0; FORCE=0; MERGE_SETTINGS=0; NO_ITERM=0
BIN="$HOME/.local/bin"
CLAUDE_DIR="$HOME/.claude"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"

RED=''; GRN=''; YEL=''; DIM=''; RST=''
if [ -t 1 ]; then
  RED=$'\033[31m'; GRN=$'\033[32m'; YEL=$'\033[33m'; DIM=$'\033[2m'; RST=$'\033[0m'
fi

die()  { printf '%sinstall.sh: %s%s\n' "$RED" "$*" "$RST" >&2; exit 1; }
note() { printf '%s\n' "$*"; }
warn() { printf '%swarn:%s %s\n' "$YEL" "$RST" "$*" >&2; }
usage() { awk 'NR>=3 { if ($0 ~ /^#/) { sub(/^# ?/, ""); print; next } else exit }' "$0"; }

for a in "$@"; do
  case "$a" in
    --dry-run)        DRY=1 ;;
    --copy)           COPY=1 ;;
    --uninstall)      UNINSTALL=1 ;;
    --force)          FORCE=1 ;;
    --merge-settings) MERGE_SETTINGS=1 ;;
    --no-iterm)       NO_ITERM=1 ;;
    --bin=*)          BIN="${a#--bin=}" ;;
    --claude-dir=*)   CLAUDE_DIR="${a#--claude-dir=}" ;;
    --config-dir=*)   CONFIG_DIR="${a#--config-dir=}" ;;
    -h|--help)        usage; exit 0 ;;
    *)                die "unknown option: $a (try --help)" ;;
  esac
done

# --- the manifest -------------------------------------------------------------
# One record per line: <kind> <TAB> <source> <TAB> <destination>
#   link   symlink dest -> source (or copy, under --copy)
#   seed   create yo's config at dest ONCE (source unused); an existing one is
#          the user's and stays
#   merge  jq-merge source into dest, source's keys winning
#   peer   make sure iTerm2's Claude Code workgroup has a terminal peer named
#          <source> (there is no destination; it lives in iTerm's preferences)
PLAN=""
TAB=$(printf '\t')
add() { PLAN="${PLAN}${1}${TAB}${2}${TAB}${3}
"; }

add link  "$REPO/scripts/yo"            "$BIN/yo"
add seed  "-"                           "$CONFIG_DIR/yo/config.json"
# The settings layer and the statusline it names live with yo's config, not in
# ~/.claude: nothing of Claude Code's own is replaced.
add link  "$REPO/claude/statusline.sh"  "$CONFIG_DIR/yo/statusline.sh"
add link  "$REPO/claude/settings.json"  "$CONFIG_DIR/yo/claude-settings.json"
# Skills that drive yo from inside a session. These do go in ~/.claude, since
# that is where Claude Code finds personal skills; each is its own directory,
# so nothing already there is touched.
add link  "$REPO/claude/skills/yo-down" "$CLAUDE_DIR/skills/yo-down"
add link  "$REPO/claude/skills/yo-adopt" "$CLAUDE_DIR/skills/yo-adopt"
add link  "$REPO/claude/skills/yo-title" "$CLAUDE_DIR/skills/yo-title"
if [ "$MERGE_SETTINGS" -eq 1 ]; then
  add merge "$REPO/claude/settings.json" "$CLAUDE_DIR/settings.json"
fi
if [ "$NO_ITERM" -eq 0 ]; then
  add peer "CLI" "-"
fi

# --- preflight ----------------------------------------------------------------
preflight() {
  local missing=0 c
  for c in git jq python3; do
    command -v "$c" >/dev/null 2>&1 || { warn "required tool not found: $c"; missing=1; }
  done
  [ "$missing" -eq 0 ] || die "install the missing required tools first"
  for c in gh; do
    command -v "$c" >/dev/null 2>&1 \
      || warn "$c not found — the parts of yo that use it will degrade"
  done
  [ -d /Applications/iTerm.app ] || warn "iTerm2 not found — yo drives it to open sessions"
}

path_has() {
  case ":$PATH:" in *":$1:"*) return 0 ;; *) return 1 ;; esac
}

# --- actions ------------------------------------------------------------------
do_backup() {
  local dest="$1"
  local bak="${dest}.bak-${TS}"
  note "    ${DIM}backing up existing $(basename "$dest") -> $(basename "$bak")${RST}"
  [ "$DRY" -eq 1 ] || mv "$dest" "$bak"
}

install_link() {
  local src="$1" dest="$2" cur
  [ -e "$src" ] || die "missing source: $src"

  if [ -L "$dest" ]; then
    cur="$(readlink "$dest")"
    if [ "$cur" = "$src" ] && [ "$COPY" -eq 0 ]; then
      note "  ${GRN}ok${RST}      $dest ${DIM}(already linked)${RST}"
      return 0
    fi
    if [ "$FORCE" -eq 0 ] && [ "$cur" != "$src" ]; then
      warn "$dest is a symlink to $cur — leaving it alone (use --force to replace)"
      return 0
    fi
    note "  ${YEL}relink${RST}  $dest ${DIM}(was -> $cur)${RST}"
    [ "$DRY" -eq 1 ] || rm "$dest"
  elif [ -e "$dest" ]; then
    do_backup "$dest"
  fi

  [ "$DRY" -eq 1 ] || mkdir -p "$(dirname "$dest")"
  if [ "$COPY" -eq 1 ]; then
    note "  ${GRN}copy${RST}    $dest ${DIM}<- ${src#$REPO/}${RST}"
    [ "$DRY" -eq 1 ] || cp -R "$src" "$dest"
  else
    note "  ${GRN}link${RST}    $dest ${DIM}-> ${src#$REPO/}${RST}"
    [ "$DRY" -eq 1 ] || ln -s "$src" "$dest"
  fi
}

# The config is the user's own file. A symlink that points nowhere is what an
# older install left behind (it used to link the config into the repo).
install_seed() {
  local dest="$2"
  if [ -L "$dest" ] && [ ! -e "$dest" ]; then
    note "  ${YEL}unlink${RST}  $dest ${DIM}(dangling, was -> $(readlink "$dest"))${RST}"
    [ "$DRY" -eq 1 ] || rm "$dest"
  elif [ -e "$dest" ]; then
    note "  ${GRN}ok${RST}      $dest ${DIM}(yours — left alone)${RST}"
    return 0
  fi
  note "  ${GRN}create${RST}  $dest ${DIM}(lists this checkout; add yours with \`yo config add-repo\`)${RST}"
  [ "$DRY" -eq 1 ] && return 0
  mkdir -p "$(dirname "$dest")"
  # This checkout, so yo can open sessions on itself.
  jq -n --arg path "$REPO" '{repos: {yo: {path: $path}}}' > "$dest"
}

# jq's `*` merges objects recursively, right operand winning — so this repo's
# keys land without disturbing anything else already in the file (Claude Code
# writes it too, and iTerm2's Claude Code integration installs hooks into it).
install_merge() {
  local src="$1" dest="$2" tmp
  [ -f "$src" ] || die "missing source: $src"

  if [ ! -e "$dest" ]; then
    note "  ${GRN}create${RST}  $dest ${DIM}<- ${src#$REPO/}${RST}"
    [ "$DRY" -eq 1 ] || { mkdir -p "$(dirname "$dest")"; cp "$src" "$dest"; }
    return 0
  fi
  [ -L "$dest" ] && warn "$dest is a symlink; merging through it into $(readlink "$dest")"
  jq -e . "$dest" >/dev/null 2>&1 || die "$dest is not valid JSON — fix or move it first"

  if jq -e -s '.[0] * .[1] == .[0]' "$dest" "$src" >/dev/null 2>&1; then
    note "  ${GRN}ok${RST}      $dest ${DIM}(already merged)${RST}"
    return 0
  fi
  note "  ${GRN}merge${RST}   $dest ${DIM}<- ${src#$REPO/} (repo keys win; rest preserved)${RST}"
  if [ "$DRY" -eq 1 ]; then
    note "    ${DIM}keys contributed: $(jq -r 'keys | join(", ")' "$src")${RST}"
    return 0
  fi
  cp "$dest" "${dest}.bak-${TS}"
  note "    ${DIM}backed up to $(basename "$dest").bak-${TS}${RST}"
  tmp="$(mktemp "${dest}.XXXXXX")"
  jq -s '.[0] * .[1]' "$dest" "$src" > "$tmp"
  mv "$tmp" "$dest"
}

uninstall_link() {
  local src="$1" dest="$2"
  if [ -L "$dest" ] && [ "$(readlink "$dest")" = "$src" ]; then
    note "  ${GRN}rm${RST}      $dest"
    [ "$DRY" -eq 1 ] || rm "$dest"
  elif [ -e "$dest" ]; then
    warn "$dest isn't a link into this repo — leaving it alone"
  fi
}

# Can't truly unmerge, so drop the top-level keys this repo contributes. If you
# had your own value for one of those before installing, it went into the
# .bak- file at install time.
uninstall_merge() {
  local src="$1" dest="$2" tmp
  [ -f "$dest" ] || return 0
  note "  ${YEL}unmerge${RST} $dest ${DIM}(dropping keys: $(jq -r 'keys | join(", ")' "$src"))${RST}"
  [ "$DRY" -eq 1 ] && return 0
  cp "$dest" "${dest}.bak-${TS}"
  tmp="$(mktemp "${dest}.XXXXXX")"
  jq --slurpfile s "$src" 'delpaths([$s[0] | keys[] | [.]])' "$dest" > "$tmp"
  mv "$tmp" "$dest"
}

# iTerm2's Claude Code integration runs claude inside a "workgroup": the chat
# session plus whatever peers the workgroup declares. A terminal peer gives
# every claude session a shell of its own, which is what lets yo open claude
# alone instead of splitting a pane for one (YO_LAYOUT=workgroup).
#
# The workgroups are a JSON blob in iTerm's preferences. iTerm reads it at
# launch, so a peer added while iTerm is running only shows up after iTerm is
# restarted — and editing the workgroups in iTerm's settings before then writes
# iTerm's own copy back over it (re-run this installer if that happens).
#
# `check` prints PRESENT, MISSING, or NOGROUP (the integration isn't set up, so
# there is no workgroup to add to). `add <name> <backup>` saves the current blob
# to <backup> and prints the new one as hex, for `defaults write -data`.
iterm_peer() {
  python3 - "$@" <<'PY'
import subprocess, plistlib, os, json, sys, uuid
mode = sys.argv[1]
prefs = os.path.expanduser("~/Library/Preferences/com.googlecode.iterm2.plist")
GROUP, ROOT = "builtin.claudeCode", "builtin.claudeCode.main"
try:
    xml = subprocess.run(["plutil", "-convert", "xml1", "-o", "-", prefs],
                         capture_output=True, check=True).stdout
    raw = plistlib.loads(xml)["Workgroups"]
    groups = json.loads(raw)
    group = next(g for g in groups if g.get("uniqueIdentifier") == GROUP)
except Exception:
    print("NOGROUP"); sys.exit(0)
sessions = group.setdefault("sessions", [])
if any((s.get("kind") or {}).get("kind") == "peer" for s in sessions):
    print("PRESENT"); sys.exit(0)
if mode == "check":
    print("MISSING"); sys.exit(0)
name, backup = sys.argv[2], sys.argv[3]
with open(backup, "wb") as f:
    f.write(raw)
sessions.append({
    "mode": "regular",
    "kind": {"kind": "peer"},
    "command": "",
    "perFileCommand": "",
    "toolbarItems": [{"kind": "modeSwitcher"}],
    "displayName": name,
    "uniqueIdentifier": str(uuid.uuid4()).upper(),
    "urlString": "",
    "parentID": ROOT,
})
print(json.dumps(groups, separators=(",", ":")).encode().hex())
PY
}

install_peer() {
  local name="$1" state backup hex
  [ -d /Applications/iTerm.app ] || return 0
  state=$(iterm_peer check) || state="NOGROUP"
  case "$state" in
    PRESENT)
      note "  ${GRN}ok${RST}      iTerm2 Claude Code workgroup ${DIM}(already has a terminal peer)${RST}"
      return 0 ;;
    NOGROUP)
      warn "iTerm2's Claude Code workgroup isn't in its preferences — skipping the '$name' peer
       set the integration up in iTerm2 first; until then yo splits a pane for the shell"
      return 0 ;;
  esac
  note "  ${GRN}peer${RST}    iTerm2 Claude Code workgroup ${DIM}<- terminal peer '$name'${RST}"
  if pgrep -xq iTerm2; then
    note "    ${DIM}iTerm2 is running: the peer appears after its next restart. Don't edit${RST}"
    note "    ${DIM}workgroups in iTerm2's settings until then, or it is overwritten.${RST}"
  fi
  [ "$DRY" -eq 1 ] && return 0
  backup="$CONFIG_DIR/yo/iterm-workgroups.bak-${TS}.json"
  mkdir -p "$(dirname "$backup")"
  hex=$(iterm_peer add "$name" "$backup")
  defaults write com.googlecode.iterm2 Workgroups -data "$hex"
  note "    ${DIM}previous workgroups saved to $backup${RST}"
}

# --- run ----------------------------------------------------------------------
main() {
  local kind src dest

  if [ "$UNINSTALL" -eq 1 ]; then
    note "${DIM}uninstalling from ${REPO}${RST}"
  else
    preflight
    note "${DIM}installing from ${REPO}${RST}"
  fi
  [ "$DRY" -eq 1 ] && note "${YEL}dry run — nothing will be written${RST}"
  note ""

  while IFS="$TAB" read -r kind src dest; do
    [ -n "$kind" ] || continue
    if [ "$UNINSTALL" -eq 1 ]; then
      case "$kind" in
        link)  uninstall_link  "$src" "$dest" ;;
        seed)  note "  ${DIM}kept${RST}    $dest ${DIM}(your config)${RST}" ;;
        merge) uninstall_merge "$src" "$dest" ;;
        peer)  : ;;   # a workgroup peer is yours to keep; remove it in iTerm2
      esac
    else
      case "$kind" in
        link)  install_link  "$src" "$dest" ;;
        seed)  install_seed  "$src" "$dest" ;;
        merge) install_merge "$src" "$dest" ;;
        peer)  install_peer  "$src" ;;
      esac
    fi
  done <<EOF
$PLAN
EOF

  note ""
  if [ "$UNINSTALL" -eq 1 ]; then
    note "done. Backups of anything rewritten: *.bak-${TS}"
    return 0
  fi

  path_has "$BIN" || warn "$BIN is not on your PATH — add to your shell rc:
       export PATH=\"$BIN:\$PATH\""

  if [ "$COPY" -eq 1 ]; then
    note "done (copies). Re-run install.sh after changing anything in this repo."
  else
    note "done. This checkout is now live — moving or deleting it breaks the links."
    note "${DIM}Edit on a branch without disturbing the install: yo yo <name>${RST}"
  fi
}

main
