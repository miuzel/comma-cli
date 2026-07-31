# Edit & refine demo: , compress video to 10mb → press 'r' → add --dry-run
# Requires a configured LLM API key. Run from the repo root: vhs demo/edit-refine.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) so the UI renders in English.
# Warm the response cache first (the initial command is replayed from cache,
# which also avoids the #EXPLORE probe) — see demo/README.md.
# Ends with 'n' so nothing is executed.

Output demo/edit-refine.gif

Set FontSize 16
Set Width 1200
Set Height 675
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

Type ", compress video to 10mb"
Sleep 500ms
Enter
Sleep 12s
Type "r"
Sleep 500ms
Type " add --dry-run"
Sleep 300ms
Enter
Sleep 12s
Type "n"
Sleep 2s
