# gprr — reviewing: side-by-side diff, line-range comment, Markdown preview,
# and the review submission modal. Recorded against `gprr --demo`.
Output "assets/demo-review.gif"

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
Sleep 2800ms

# Open the PR and jump to the Files diff.
Enter
Sleep 1800ms
Type "]"
Sleep 1200ms
Enter
Sleep 1200ms

# Move the line cursor, then select a range and comment on it.
Type "j"  Sleep 350ms
Type "j"  Sleep 350ms
Type "j"  Sleep 500ms
Type "V"  Sleep 700ms
Type "j"  Sleep 400ms
Type "j"  Sleep 800ms
Type "c"
Sleep 1200ms

# Compose a Markdown comment, preview the rendered output, then submit it.
Type "Could we expose `chunk_size` through a builder arg as well?"
Sleep 1000ms
Ctrl+P
Sleep 2400ms
Ctrl+P
Sleep 900ms
Ctrl+S
Sleep 1800ms

Type "q"
Sleep 800ms
