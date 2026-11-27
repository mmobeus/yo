-- Opens a session's iTerm2 tab (or window) and starts it. Everything comes in
-- as arguments, never pasted into this text. From bash's open_iterm_session.
--
-- Sessions land as tabs in one shared window, found again by the user.yoWindow
-- variable each of its sessions carries, so Session Status lists them all.
on run argv
  set winTitle to item 1 of argv
  set profileName to item 2 of argv
  set setupCmd to item 3 of argv
  set claudeCmd to item 4 of argv
  set winW to (item 5 of argv) as integer
  set winH to (item 6 of argv) as integer
  set scrW to (item 7 of argv) as integer
  set scrH to (item 8 of argv) as integer
  set marker to item 9 of argv
  set forceNew to (item 10 of argv) as integer
  set doSplit to (item 11 of argv) as integer

  tell application "iTerm2"
    activate
    set targetWin to missing value

    -- Reuse the window already holding this group's sessions, if there is one.
    if forceNew is 0 then
      repeat with ww in (get windows)
        repeat with tt in (get tabs of ww)
          repeat with ss in (get sessions of tt)
            set v to ""
            try
              tell ss
                set v to variable named "user.yoWindow"
              end tell
            end try
            if v is marker then set targetWin to ww
          end repeat
        end repeat
      end repeat
    end if

    if targetWin is missing value then
      -- New window, centered on the PRIMARY display (its top-left is global
      -- (0,0), so half the leftover space is the top-left corner). If we
      -- couldn't read the screen size, fall back to Finder's desktop bounds —
      -- but that's the UNION across all displays, so on a multi-monitor setup
      -- the window lands off to the right (the old bug).
      if profileName is "" then
        set targetWin to (create window with default profile)
      else
        set targetWin to (create window with profile profileName)
      end if
      if scrW is 0 or scrH is 0 then
        tell application "Finder" to set sb to bounds of window of desktop
        set scrW to item 3 of sb
        set scrH to item 4 of sb
      end if
      set x1 to (scrW - winW) div 2
      set y1 to (scrH - winH) div 2
      set bounds of targetWin to {x1, y1, x1 + winW, y1 + winH}
      set leftSession to (current session of targetWin)
    else
      -- Existing window: add a tab and leave the window's size/position alone.
      try
        set frontmost of targetWin to true
      end try
      tell targetWin
        if profileName is "" then
          set newTab to (create tab with default profile)
        else
          set newTab to (create tab with profile profileName)
        end if
      end tell
      set leftSession to (current session of newTab)
    end if

    -- Name the panes; with the profile's title lock this sticks and claude
    -- (left pane) can't overwrite it. The TAB title follows the active
    -- session's name, so this is what labels the tab and the Session Status row.
    --
    -- Marker: how the next `yo` finds this window. yoTitle drives the tab
    -- title (see the profile's Custom Tab Title) and survives the workgroup
    -- renaming the session.
    tell leftSession
      write text setupCmd
      set name to winTitle
      set variable named "user.yoWindow" to marker
      set variable named "user.yoTitle" to winTitle
    end tell

    if doSplit is 1 then
      tell leftSession
        if profileName is "" then
          set rightSession to (split vertically with default profile)
        else
          set rightSession to (split vertically with profile profileName)
        end if
      end tell
      tell rightSession
        write text setupCmd
        set name to winTitle
        set variable named "user.yoWindow" to marker
        set variable named "user.yoTitle" to winTitle
      end tell
    end if

    tell leftSession
      write text claudeCmd
      select
    end tell

    -- Its tab, "<window id>/<tab id>", printed for yo to record: how it
    -- finds the tab again when a workgroup shows CLI and AppleScript no
    -- longer lists this session.
    set tabRef to ""
    try
      tell leftSession
        set tabRef to ((variable named "tab.window.id") as text) & "/" & ((variable named "tab.id") as text)
      end tell
    end try
    return tabRef
  end tell
end run
