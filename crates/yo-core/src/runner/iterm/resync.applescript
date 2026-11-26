-- Every iTerm2 session carrying a user.yoTitle gets it back as its name.
-- From bash's _yo_resync_titles.
tell application "iTerm2"
  repeat with ww in (get windows)
    repeat with tt in (get tabs of ww)
      repeat with ss in (get sessions of tt)
        set v to missing value
        try
          tell ss to set v to variable named "user.yoTitle"
        end try
        if v is not missing value and v is not "" then
          if (name of ss) is not v then set name of ss to v
        end if
      end repeat
    end repeat
  end repeat
end tell
