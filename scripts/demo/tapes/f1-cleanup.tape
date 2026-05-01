# F1 — Branch cleanup
# Toggle merged PRs, select all merged, batch delete.
#
# Regenerate via: make demos-f1

Output docs/images/f1-cleanup.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set TypingSpeed 50ms
Set Padding 20

Hide
Type 'eval "$(bash scripts/demo/setup.sh f1)"'
Enter
Sleep 800ms
Show

Type "gct"
Sleep 300ms
Enter
Sleep 1800ms

# My PR filter
Type "2"
Sleep 1500ms

# Toggle merged display
Type "m"
Sleep 1500ms

# Select all merged
Type "a"
Sleep 1200ms

# Open batch-delete confirmation dialog
Type "d"
Sleep 1500ms

# Confirm
Type "y"
Sleep 2000ms

# Quit
Type "q"
Sleep 800ms
