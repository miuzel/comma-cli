# Demo GIFs

Every GIF in this directory is recorded with
[vhs](https://github.com/charmbracelet/vhs) from the tape of the same name.
`README.md` / `README.zh-CN.md` embed the GIFs and note the record command.

To reproduce a recording:

1. [Install the tools](#install-the-tools)
2. [Build `,` and put it first on `PATH`](#build-and-put-comma-first-on-path)
3. [Create the sandbox HOME](#sandbox-home)
4. [Warm the response cache](#warm-the-response-cache)
5. [Record](#record)

## Install the tools

`vhs` needs `ttyd`, `ffmpeg` and a Chromium/Chrome binary. On macOS
`brew install vhs` pulls all of that in. On Linux you can do it without root by
dropping the two static binaries into `./tmp/` (gitignored):

```bash
mkdir -p tmp/bin
curl -sSL -o tmp/vhs.tar.gz \
  https://github.com/charmbracelet/vhs/releases/download/v0.11.0/vhs_0.11.0_Linux_x86_64.tar.gz
tar xzf tmp/vhs.tar.gz --strip-components=1 -C tmp/bin
curl -sSL -o tmp/bin/ttyd https://github.com/tsl0922/ttyd/releases/download/1.7.7/ttyd.x86_64
chmod +x tmp/bin/ttyd
export PATH="$PWD/tmp/bin:$PATH"
```

Two things that bite on Linux:

- **Use vhs 0.11.0, not 0.12.0.** 0.12.0 renders the GIF with a context that
  `vhs.Render` already cancelled, so it prints `Creating demo/x.gif...` and
  silently writes nothing (`vhs` still exits 0). 0.11.0 is fine.
- **Chromium must be a Linux build.** vhs looks for `google-chrome` before
  `chromium`; on WSL `/usr/local/bin/google-chrome` is often a symlink to a
  Windows `chrome.exe`, which cannot use a Linux `--user-data-dir` and dies
  immediately ("browser exited unexpectedly"). Shadow it with a wrapper early
  on `PATH`:

  ```bash
  printf '#!/bin/sh\nexec /usr/bin/chromium "$@"\n' > tmp/bin/google-chrome
  chmod +x tmp/bin/google-chrome
  ```

  `export VHS_NO_SANDBOX=1` if Chromium refuses to start otherwise.

## Build and put `,` first on PATH

The demos must run the build you are documenting, not an older installed `,`:

```bash
cargo build --release
ln -sf "$PWD/target/release/comma" tmp/bin/,
, --version   # must match Cargo.toml
```

## Sandbox HOME

The tapes point `HOME` at `/tmp/comma-demo-home` so the demo UI is consistent
regardless of your real config (a `lang` key in the real config would override
`COMMA_LANG`). Set it up once — the config lives in `~/.config/comma/`:

```bash
mkdir -p /tmp/comma-demo-home/.config/comma
# Copy your config, drop the "lang" key and cap the auto-refine budget:
python3 -c "
import json, os
cfg = json.load(open(os.path.expanduser('~/.config/comma/config.json')))
cfg.pop('lang', None)            # config.lang beats COMMA_LANG
cfg['auto_refine_rounds'] = 2    # auto-refine.gif shows the exhausted path at round 2/2
json.dump(cfg, open('/tmp/comma-demo-home/.config/comma/config.json', 'w'), indent=2)
"
```

Leave your real config untouched: it holds API keys, and none of that may end
up in the repository.

## Warm the response cache

Caching the LLM responses makes the recordings deterministic: no spinner
timing, no rate-limit noise, and no `#CHECK:`/`#EXPLORE:` probe hijacking the
keystrokes. Cache keys include the working directory, so run these in
`/tmp/comma-demo` exactly like the tapes do:

```bash
export HOME=/tmp/comma-demo-home COMMA_LANG=en
mkdir -p /tmp/comma-demo && cd /tmp/comma-demo
printf '# TODO: fix this\nprint(1)  # TODO: refactor\n' > app.py
head -c 2000000 /dev/zero > big.bin
head -c 500000 /dev/zero > mid.bin
head -c 1000 /dev/zero > small.bin

# 'y' answers any #CHECK:/#EXPLORE: probe and the final execute prompt.
printf 'y\ny\ny\ny\ny\n' | , find all TODO comments in python files
printf 'y\ny\ny\ny\ny\n' | , compress video to 10mb
printf 'y\ny\ny\ny\ny\n' | , list files by size    # re-run until the reply offers ||| candidates
printf 'y\ny\ny\ny\ny\n' | , systemctl status demo-api
printf 'y\ny\ny\ny\ny\n' | , count down from 20 to 1 one number per second
```

A cached entry is written only when the command is executed, so the model may
return a single command for `list files by size` instead of a `|||` candidate
list; delete that entry and warm it again until the multi-candidate demo has
something to select.

The automatic refine in `auto-refine.tape` is REPL-only, so its refine turns
are warmed by running that tape once before the recording pass: the first pass
is live and its GIF is thrown away, the second pass replays everything from
the cache. (`edit-refine.tape`'s refine turn is a manual one and is never
cached, so it is the only step that still calls the live API while recording —
if it gets rate-limited (429 fallback noise in the GIF), wait a moment and
re-run that tape.)

## Record

From the repository root:

```bash
vhs demo/basic-usage.tape
vhs demo/action-menu.tape
vhs demo/auto-refine.tape   # run twice: the first pass warms the refine turns
vhs demo/interrupt.tape
vhs demo/edit-refine.tape
vhs demo/multi-candidate.tape
vhs demo/i18n.tape
```

`auto-refine.tape` sets `Set Height 780` instead of the usual 675: it prints
about 20 lines, and the taller terminal keeps the session from scrolling,
which keeps the GIF around 210 KB instead of ~580 KB. The other tapes keep
`1200x675`.

Keep the GIFs in the 100–200 KB range. If one balloons, look for a full-screen
repaint (scrolling, a resizing window) rather than just shortening sleeps —
static frames are nearly free, scrolling is not.
