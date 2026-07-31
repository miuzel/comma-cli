# Demo GIFs

Recorded with [vhs](https://github.com/charmbracelet/vhs). To reproduce:

## Prerequisites

- `vhs` (with its own deps: `ttyd`, `ffmpeg`, Chromium)
- A configured `,` (the LLM-backed demos call the real API)

## Sandbox HOME

The tapes point `HOME` at `/tmp/comma-demo-home` so the demo UI is consistent
regardless of your real config (e.g. a `lang` key in the real config would
override `COMMA_LANG`). Set it up once:

```bash
mkdir -p /tmp/comma-demo-home/.config/comma
# Copy your config but REMOVE the "lang" key (config.lang beats COMMA_LANG):
python3 -c "
import json
cfg = json.load(open('$HOME/.local/bin/,.config.json'))
cfg.pop('lang', None)
json.dump(cfg, open('/tmp/comma-demo-home/.config/comma/config.json', 'w'), indent=2)
"
cp ~/.local/bin/,.prompt.md /tmp/comma-demo-home/.config/comma/prompt.md
```

## Warm the response cache

Caching the initial LLM responses makes the recordings deterministic:
no spinner timing, no rate-limit noise, and no `#EXPLORE:` probe hijacking
the keystrokes. Cache keys include the working directory, so run these in
`/tmp/comma-demo` exactly like the tapes do:

```bash
export HOME=/tmp/comma-demo-home COMMA_LANG=en
mkdir -p /tmp/comma-demo && cd /tmp/comma-demo
printf '# TODO: fix this\nprint(1)  # TODO: refactor\n' > app.py

# 'y' answers the #EXPLORE probe and the final execute prompt
# (ffmpeg fails on the missing input.mp4 — harmless, and it caches).
printf 'y\ny\ny\ny\n' | , compress video to 10mb
printf 'y\n' | , find all TODO comments in python files
printf 'y\n' | , list files by size   # piped mode auto-picks the first candidate
```

## Record

From the repository root:

```bash
vhs demo/basic-usage.tape
vhs demo/edit-refine.tape
vhs demo/multi-candidate.tape
vhs demo/i18n.tape
```

Only the refine step in `edit-refine.tape` still hits the live API; if it
gets rate-limited (429 fallback noise in the GIF), wait a moment and re-run
that tape.
