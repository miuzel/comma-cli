# i18n demo: COMMA_LANG=zh , --help (no API key needed)
# Run from the repo root: vhs demo/i18n.tape
# Uses a sandbox HOME (/tmp/comma-demo-home) whose config has no `lang` key,
# so COMMA_LANG actually takes effect (config.lang would win otherwise).

Output demo/i18n.gif

Set FontSize 16
Set Width 1200
Set Height 675
Set TypingSpeed 60ms
Set Theme "Catppuccin Mocha"

Env HOME "/tmp/comma-demo-home"

Type "COMMA_LANG=zh , --help"
Sleep 500ms
Enter
Sleep 3s
