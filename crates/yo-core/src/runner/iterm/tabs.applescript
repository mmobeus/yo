-- Every iTerm2 session iTerm shows, one per line: its terminal, a tab
-- character, then its tab as "<window id>/<tab id>". From iTerm's
-- variables, which a session in a workgroup shares with the others in its
-- tab: AppleScript lists only the member that's showing (Chat or CLI), so
-- the tab is how yo knows a hidden member still has one.
on run argv
  set out to ""
  tell application "iTerm2"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        repeat with ss in (get sessions of tt)
          set seen to ""
          try
            tell ss
              set seen to ((variable named "tab.window.id") as text) & "/" & ((variable named "tab.id") as text)
            end tell
          end try
          set out to out & (tty of ss) & (character id 9) & seen & linefeed
        end repeat
      end repeat
    end repeat
  end tell
  return out
end run
