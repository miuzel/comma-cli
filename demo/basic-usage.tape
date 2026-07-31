# Basic usage demo: , find all TODO comments in python files
# Requires a configured LLM API key. Run from the repo root: vhs demo/basic-usage.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) so the UI renders in English
# (the real config sets lang: zh, which takes priority over COMMA_LANG).
# Warm the response cache first — see demo/README.md.

Output demo/basic-usage.gif

Set FontSize 16
Set Width 1200
Set Height 675
Set TypingSpeed 60ms
Set Theme "Catppuccin Mocha"

Env HOME "/tmp/comma-demo-home"
Env COMMA_LANG "en"

# Prepare a scratch dir with a Python file containing TODOs (off-screen).
Hide
Type "mkdir -p /tmp/comma-demo && printf '# TODO: fix this\nprint(1)  # TODO: refactor\n' > /tmp/comma-demo/app.py && cd /tmp/comma-demo"
Enter
Sleep 1s
Show

Type "clear"
Enter
Sleep 500ms

Type ", find all TODO comments in python files"
Sleep 500ms
Enter
Sleep 12s
Enter
Sleep 3s
