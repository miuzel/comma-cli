# Edit & refine demo: , compress video to 10mb → 'r' in the action menu → add --dry-run
# Requires a configured LLM API key. Run from the repo root: vhs demo/edit-refine.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) so the UI renders in English.
# Warm the response cache first (the initial command is replayed from cache,
# which also avoids the #EXPLORE probe) — see demo/README.md.
# The refine turn itself calls the live API (the one deliberate exception).
# Ends with Esc so nothing is executed.

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
# The command opens the action menu; 'r' refines it from here.
Type "r"
Sleep 500ms
Type " add --dry-run"
Sleep 300ms
Enter
Sleep 12s
# Esc cancels the menu for the refined command, so nothing runs.
Escape
Sleep 2s
