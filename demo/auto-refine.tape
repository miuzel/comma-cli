# Auto-refine demo: a failing command is refined automatically, and the
# per-intent budget caps the chain — the round counter is shown, and an
# exhausted budget stops the chain and returns to the `> ` prompt.
# Requires a configured LLM API key. Run from the repo root: vhs demo/auto-refine.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) whose config caps the budget at
# `auto_refine_rounds: 2` (the default is 3) so the GIF stays short and reaches
# the exhausted path — see demo/README.md.
# Warm the response cache first (initial command AND the refine turns).
# The canvas is a little taller than the other demos: this one prints ~20 lines
# (three commands, their menus and their output), and a taller terminal keeps
# the session from scrolling — which is what keeps the GIF around 200 KB
# instead of ~580 KB.

Output demo/auto-refine.gif

Set FontSize 16
Set Width 1200
Set Height 780
Set TypingSpeed 60ms
Set Theme "Catppuccin Mocha"

Env HOME "/tmp/comma-demo-home"
Env COMMA_LANG "en"

Hide
Type "cd /tmp/comma-demo"
Enter
Sleep 1s
Show

Type "clear"
Enter
Sleep 500ms

Type ","
Enter
Sleep 2s

# A service that does not exist: the command always exits non-zero.
Type "systemctl status demo-api"
Sleep 500ms
Enter
Sleep 4s

# Enter runs the failed command's replacement each round.
Enter
Sleep 4s
Enter
Sleep 4s
Enter
Sleep 4s

Type "q"
Enter
Sleep 1s
