# Ctrl+C demo: interrupting a running command — `,` interrupts it, then asks
# `Exit REPL? [y/N]`; `N` drops the interrupted step and returns to `> `.
# The welcome block also shows the input history, which is ON by default.
# Requires a configured LLM API key. Run from the repo root: vhs demo/interrupt.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) so the UI renders in English.
# Warm the response cache first (a long-running command) — see demo/README.md.

Output demo/interrupt.gif

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

# The welcome block discloses the (default-on) input history.
Type ","
Enter
Sleep 2s

# A command that keeps running, so there is something to interrupt.
Type "count down from 20 to 1 one number per second"
Sleep 500ms
Enter
Sleep 4s

Enter
Sleep 5s

# Ctrl+C while the command runs: it is interrupted first, then `,` asks.
Ctrl+C
Sleep 3s

# N (or Enter, or another Ctrl+C) stays in the REPL.
Type "n"
Sleep 2s

Type "q"
Enter
Sleep 1s
