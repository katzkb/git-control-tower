# F2 — Branch search and filter switch
# Filter Local branches with `/`, then jump to My PR view.
#
# Regenerate via: make demos-f2

Output docs/images/f2-search.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set TypingSpeed 60ms
Set Padding 20

Hide
Type 'eval "$(bash scripts/demo/setup.sh f2)"'
Enter
Sleep 800ms
Show

Type "gct"
Sleep 300ms
Enter
Sleep 1800ms

# Open search — sidebar filter prompt appears
Type "/"
Sleep 600ms

# Type each character slowly so the narrowing is visible
Type "a"
Sleep 500ms
Type "u"
Sleep 500ms
Type "t"
Sleep 500ms
Type "h"
Sleep 1500ms

# Clear search and show full list again
Escape
Sleep 1200ms

# Switch to My PR filter — sidebar swaps to author-me PRs
Type "2"
Sleep 1800ms

Type "q"
Sleep 600ms
