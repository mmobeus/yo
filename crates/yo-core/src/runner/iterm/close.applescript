-- Closes the iTerm2 tab holding the session with unique id (argument 1),
-- which ends both panes, and the window if it was the last tab. Found by id,
-- not focus: run from a background session, the focused tab is someone
-- else's. From bash's close_iterm_own_tab.
--
-- If no session shown has that id, the tab "<window id>/<tab id>"
-- (argument 2, if any) is closed instead: a workgroup tab showing CLI
-- doesn't list its Chat session, so a session in Chat, closing its own
-- tab, finds it by the tab yo recorded.
on run argv
  set want to item 1 of argv
  set wantTab to item 2 of argv
  tell application "iTerm2"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        repeat with ss in (get sessions of tt)
          if (unique id of ss) is want then
            close tt
            return "closed"
          end if
        end repeat
      end repeat
    end repeat
    if wantTab is "" then return "notfound"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        repeat with ss in (get sessions of tt)
          set seen to ""
          try
            tell ss
              set seen to ((variable named "tab.window.id") as text) & "/" & ((variable named "tab.id") as text)
            end tell
          end try
          if seen is wantTab then
            close tt
            return "closed"
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return "notfound"
end run
