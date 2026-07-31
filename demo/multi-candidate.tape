# Multi-candidate demo: , list files by size → navigate candidates → select one
# Requires a configured LLM API key. Run from the repo root: vhs demo/multi-candidate.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) so the UI renders in English.
# Warm the response cache first (the cached response replays the full
# candidate list instantly) — see demo/README.md.
# Ends with 'n' so nothing is executed.

Output demo/multi-candidate.gif

Set FontSize 16
Set Width 1200
Set Height 675
Set TypingSpeed 60ms
Set Theme "Catppuccin Mocha"

Env HOME "/tmp/comma-demo-home"
Env COMMA_LANG "en"

Hide
Type "mkdir -p /tmp/comma-demo && cd /tmp/comma-demo && head -c 2000000 /dev/zero > big.bin && head -c 500000 /dev/zero > mid.bin && head -c 1000 /dev/zero > small.bin"
Enter
Sleep 1s
Show

Type "clear"
Enter
Sleep 500ms

Type ", list files by size"
Sleep 500ms
Enter
Sleep 12s
Down
Sleep 500ms
Up
Sleep 500ms
Down
Sleep 800ms
Enter
Sleep 1s
Type "n"
Sleep 2s
