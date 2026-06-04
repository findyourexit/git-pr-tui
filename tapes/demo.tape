# gprr — headline tour: dashboard → PR detail → diff → checks → help.
# Recorded against `gprr --demo` (offline sample data, no token/network).
Output "assets/demo.gif"

Set Shell "bash"
Set FontSize 18
Set Width 1280
Set Height 800
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 75ms

Hide
Type "export PS1='$ '; export XDG_CONFIG_HOME=/tmp/gprr-demo/cfg; export XDG_DATA_HOME=/tmp/gprr-demo/data; export PATH=/tmp/gprr-demo:$PATH; cd /tmp/gprr-demo; clear"
Enter
Show

Type "gprr --demo"
Sleep 600ms
Enter
Sleep 3s

# Cross-repo dashboard: move within and between buckets.
Type "j"  Sleep 700ms
Type "k"  Sleep 600ms
Type "l"  Sleep 900ms
Type "j"  Sleep 700ms
Type "l"  Sleep 900ms
Type "h"  Sleep 500ms
Type "h"  Sleep 800ms

# Open the top review-requested PR.
Enter
Sleep 2500ms

# Read the rendered Markdown conversation.
Type "j"  Sleep 450ms
Type "j"  Sleep 450ms
Type "j"  Sleep 450ms
Type "j"  Sleep 900ms

# Files sub-tab: browse the tree, then dive into the diff.
Type "]"
Sleep 1500ms
Type "j"  Sleep 700ms
Type "k"  Sleep 700ms
Enter
Sleep 1300ms
Type "j"  Sleep 350ms
Type "j"  Sleep 350ms
Type "j"  Sleep 600ms
Type "}"  Sleep 1400ms
Type "t"  Sleep 1900ms
Type "t"  Sleep 1000ms

# Leave the diff pane (focus the file tree) so the bracket keys cycle sub-tabs.
Type "h"
Sleep 800ms

# Checks rollup, then commits.
Type "]"
Sleep 2200ms
Type "]"
Sleep 2000ms

# Context-aware help overlay.
Type "?"
Sleep 2600ms
Type "?"
Sleep 700ms

Type "q"
Sleep 800ms
