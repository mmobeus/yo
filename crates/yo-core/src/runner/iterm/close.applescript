-- Closes the iTerm2 tab holding the session with unique id (argument 1),
-- which ends both panes, and the window if it was the last tab. Found by id,
-- not focus: run from a background session, the focused tab is someone
-- else's. From bash's close_iterm_own_tab.
on run argv
  set want to item 1 of argv
  tell application "iTerm2"
    repeat with ww in windows
      repeat with tt in tabs of ww
        repeat with ss in sessions of tt
          if (unique id of ss) is want then
            close tt
            return
          end if
        end repeat
      end repeat
    end repeat
  end tell
end run
