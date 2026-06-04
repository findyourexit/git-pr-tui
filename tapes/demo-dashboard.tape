# gprr — getting around: the cross-repo dashboard, the command palette, and a
# searchable / filterable per-repo PR list. Recorded against `gprr --demo`.
Output "assets/demo-dashboard.gif"

Set Shell "bash"
Set FontSize 18
Set Width 1280
Set Height 800
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms

Hide
Type "export PS1='$ '; export XDG_CONFIG_HOME=/tmp/gprr-demo/cfg; export XDG_DATA_HOME=/tmp/gprr-demo/data; export PATH=/tmp/gprr-demo:$PATH; cd /tmp/gprr-demo; clear"
Enter
Show

Type "gprr --demo"
Sleep 600ms
Enter
Sleep 3s

# Three buckets across every repo: needs-review, authored, assigned.
Type "j"  Sleep 700ms
Type "l"  Sleep 800ms
Type "j"  Sleep 700ms
Type "l"  Sleep 800ms
Type "h"  Sleep 500ms
Type "h"  Sleep 800ms

# Jump straight to any repo's PR list with the command palette.
Type ":"
Sleep 800ms
Type "repo octoadmin/atlas"
Sleep 800ms
Enter
Sleep 2200ms

# Incremental search filters the list as you type.
Type "/"
Sleep 600ms
Type "config"
Sleep 1800ms
Escape
Sleep 900ms

# Filter by state with the picker.
Type "f"
Sleep 900ms
Type "j"  Sleep 500ms
Type "j"  Sleep 700ms
Enter
Sleep 1800ms
Type "f"
Sleep 700ms
Enter
Sleep 1400ms

# Open the highlighted PR, then hop back to the dashboard via the palette.
Type "g"
Sleep 400ms
Enter
Sleep 2400ms
Type ":"
Sleep 700ms
Type "dashboard"
Sleep 700ms
Enter
Sleep 1600ms

Type "q"
Sleep 800ms
