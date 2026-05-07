# F4 — Cross-repo Reviews
# Review view aggregates PRs from multiple repos. Sidebar groups by repo,
# detail pane shows `repo: owner/name`, action menu surfaces a "not cloned"
# hint for entries that don't have a local clone.
#
# Regenerate via: make demos-f4

Output docs/images/f4-cross-repo.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set TypingSpeed 50ms
Set Padding 20

Hide
Type 'eval "$(bash scripts/demo/setup.sh cross-repo)"'
Enter
Sleep 800ms
Show

Type "gct"
Sleep 300ms
Enter
Sleep 1800ms

# Switch to Review filter — sidebar fills with cross-repo PRs grouped by repo
Type "3"
Sleep 2500ms

# Move down through the entries — detail pane updates with `repo: owner/name`
Type "j"
Sleep 1200ms
Type "j"
Sleep 1500ms

# Open the action menu — entries without a local clone show a "not cloned" footer
Enter
Sleep 2500ms

# Close the menu
Escape
Sleep 800ms

# Quit
Type "q"
Sleep 600ms
