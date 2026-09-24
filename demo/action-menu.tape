# Action-menu demo: the menu is now the default path — every generated command
# opens it immediately, with no hint line and no need to type `x` first
# (`x` only re-opens it after an Esc).
# Requires a configured LLM API key. Run from the repo root: vhs demo/action-menu.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) so the UI renders in English.
# Warm the response cache first — see demo/README.md.

Output demo/action-menu.gif

Set FontSize 16
Set Width 1200
Set Height 675
Set TypingSpeed 60ms
Set Theme "Catppuccin Mocha"

Env HOME "/tmp/comma-demo-home"
Env COMMA_LANG "en"

Hide
Type "mkdir -p /tmp/comma-demo && printf '# TODO: fix this\nprint(1)  # TODO: refactor\n' > /tmp/comma-demo/app.py && cd /tmp/comma-demo"
Enter
Sleep 1s
Show

Type "clear"
Enter
Sleep 500ms

# Enter the REPL: the welcome block names the model and discloses the
# (default-on) input history, including how to turn it off.
Type ","
Enter
Sleep 2s

# A fresh intent goes straight into the action menu.
Type "find all TODO comments in python files"
Sleep 500ms
Enter
Sleep 4s

# Esc cancels back to the `> ` prompt, `x` re-opens the menu for the same command.
Escape
Sleep 1s
Type "x"
Enter
Sleep 1s

# Enter executes the command.
Enter
Sleep 3s

Type "q"
Enter
Sleep 1s
