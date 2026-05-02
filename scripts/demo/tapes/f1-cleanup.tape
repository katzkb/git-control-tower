# F1 — Branch cleanup
# Local view → select all merged → batch delete → watch the list shrink.
#
# Done in Local view so the sidebar is `git branch` driven: deleted branches
# disappear immediately. (My PR view is PR-driven, so list contents wouldn't
# update visibly until the PR fixture changes.)
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
Sleep 2200ms

# `a` selects all merged branches in the current view
Type "a"
Sleep 1500ms

# `d` opens batch-delete confirmation
Type "d"
Sleep 1500ms

# Confirm — branches are deleted, sidebar refreshes to drop them
Type "y"
Sleep 2800ms

Type "q"
Sleep 800ms
