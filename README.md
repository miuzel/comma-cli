# `,`

> **Small is big.** A comma is the smallest punctuation — yet it changes everything.

LLM-powered shell command generator. Describe what you want, get the command, run it.

[中文文档](README.zh-CN.md)

## Why?

You know the feeling: you want to do something in the terminal, but can't remember the exact flags. `tar` with compression? `ffmpeg` encoding options? `find` with size filters? You end up opening a browser, searching, reading man pages, or copy-pasting from Stack Overflow.

**`,` is a single-character command that turns intent into shell commands.** Type what you want, get the command, run it. That's it.

### vs. Codex / OpenCode / Claude Code

| | `,` | Codex / Claude Code / OpenCode |
|---|---|---|
| **Weight** | Single 3MB binary, no runtime | Node.js / Python runtime, 100MB+ |
| **Session** | Stateless — no sessions, no history, no files | Heavy session management, conversation state |
| **Startup** | Instant (no warmup) | 2-5s cold start |
| **Scope** | One command at a time | Multi-file editing, code generation, agentic loops |
| **Dependencies** | None (static binary) | Node.js, Python, npm, etc. |
| **Config** | 3 fields in JSON | Complex config, API keys, project setup |
| **Privacy** | No personal data sent to API (placeholders) | Full context sent |

**When to use `,`:**
- You need a quick shell command, not a coding assistant
- You want something that starts instantly and exits cleanly
- You're on a remote server and don't want to install Node.js
- You prefer the terminal over a chat interface
- You want to keep your workflow minimal

**When to use Codex / Claude Code:**
- You need multi-file code generation or refactoring
- You want agentic task execution (read files, run tests, iterate)
- You need conversation context across multiple turns
- You're doing complex debugging or architecture work

Think of `,` as the `curl` of LLM tools — minimal, focused, does one thing well. Claude Code is the IDE — powerful but heavy.

### Who needs this?

- **Sysadmins**: "find all files modified today larger than 100MB" → `fd --changed-today --size +100M`
- **Developers**: "compress this video for Slack" → `ffmpeg -i input.mp4 -b:v 1M ...`
- **DevOps**: "check which ports are in use" → `ss -tlnp`
- **Anyone** who occasionally needs a terminal command but can't remember the syntax

## Install

```bash
./install.sh
```

Or manually:

```bash
cargo build --release
cp target/release/comma ~/.local/bin/,
cp prompt.md ~/.local/bin/,.prompt.md
cp config.json ~/.local/bin/,.config.json
```

Installed files:

```
~/.local/bin/
├── ,              # binary
├── ,.config.json  # config (optional, overrides Claude settings)
└── ,.prompt.md    # system prompt (editable)
```

## Usage

```bash
, what is my ip              # one-shot: generate command → confirm/execute
, list files larger than 1G  # generate du/find command
,                            # interactive mode: multi-turn refinement
, -h                         # help
, -V                         # version
```

### One-shot mode

```
$ , find all TODO comments in python files
▸ mimo-v2.5-pro (anthropic)
  tokens: 73in + 263out = 336 | 6124ms
rg -n TODO --type py  # Find TODO comments in Python files
Execute? [y/Ctrl+Enter/N]
```

Type `y` or `Ctrl+Enter` to execute, anything else to cancel.

### Interactive mode

```
$ ,
▸ Interactive mode (model: mimo-v2.5-pro). Tab completes filenames. 'q' to quit, 'x' to exec, 'c' to copy.
> find large files
fd --size +100M  # Find files larger than 100MB
> sort by size descending
fd --size +100M -x ls -lh {} + | sort -k5 -h -r  # Sort large files by size
> x
▸ Running: fd --size +100M -x ls -lh {} + | sort -k5 -h -r
```

Press **Tab** to autocomplete filenames from the current directory.

| Command | Action |
|---------|--------|
| `Tab` | Autocomplete filename |
| `x` / `exec` | Execute current command |
| `c` / `copy` | Copy to clipboard |
| `q` / `quit` / `exit` | Exit |
| Any other text | Send to LLM to refine command |

## Exploration: #EXPLORE:

When the model is unsure about a tool's usage, it returns `#EXPLORE:` to run the help command first:

```
$ , compress video to 10mb using ffmpeg
▸ Model wants to explore: ffmpeg -h
Run to learn usage? [y/N] y
▸ Learning from output...
ffmpeg -i input.mp4 -b:v 8M -b:a 128k output.mp4  # Compress video to ~10MB
Execute? [y/Ctrl+Enter/N]
```

Flow:
1. Model unsure → returns `#EXPLORE: ffmpeg -h`
2. comma-cli asks user to confirm
3. Captures help output, appends to conversation context
4. Model generates the real command with full context

## Tool Discovery: #CHECK:

When unsure what's installed, the model can query tool availability:

```
$ , "compress this image"
▸ Model wants to check: convert magick ffmpeg
  Available: ffmpeg
  Not found: convert, magick
ffmpeg -i input.png -quality 85 output.jpg  # Compress image using ffmpeg
Execute? [y/Ctrl+Enter/N]
```

## Multi-candidate Selection

The model can output multiple candidates separated by `|||`. Use ↑↓/Tab to select:

```
$ , "list files"
▸ ls -la  # Classic listing
  exa -la  # Modern ls replacement
  eza -la --icons  # ls with icons
```

- `↑`/`↓`/`j`/`k` — Move up/down
- `Tab`/`Shift+Tab` — Cycle through
- `Enter` — Confirm selection
- `Esc`/`q` — Cancel

## Config

### Config priority

```
~/.local/bin/,.config.json  >  ~/.claude/settings.json  >  built-in defaults
```

Fields left as empty string `""` fall back to the next source.

### Local config `~/.local/bin/,.config.json`

**Anthropic (Claude) example:**

```json
{
  "base_url": "https://api.anthropic.com",
  "auth_token": "sk-ant-xxx",
  "model": "claude-sonnet-4-20250514",
  "api_style": "anthropic"
}
```

**OpenAI-compatible example (Cerebras, Groq, Ollama, vLLM, etc.):**

```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "csk-xxx",
  "model": "llama-3.3-70b",
  "api_style": "openai"
}
```

| Field | Description | Fallback |
|-------|-------------|----------|
| `base_url` | API endpoint | `ANTHROPIC_BASE_URL` in settings.json |
| `auth_token` | API key | `ANTHROPIC_AUTH_TOKEN` in settings.json |
| `model` | Model name | `ANTHROPIC_MODEL` in settings.json |
| `api_style` | API format (see below) | Auto-detect (URL contains `anthropic` → anthropic, else → openai) |
| `prefer` | Tool preference map | `{}` |
| `cache_size` | Response cache size | `1000` |

### Tool Preferences (`prefer`)

Configure preferred tools — the model will use them preferentially:

```json
{
  "prefer": {
    "editor": ["nvim", "vim", "vi"],
    "list": ["eza", "exa", "ls"],
    "cat": ["bat", "batcat", "cat"],
    "find": ["fd", "find"],
    "grep": ["rg", "grep"],
    "top": ["btop", "htop", "top"]
  }
}
```

Keys are free-form category names, values are tool lists ordered by preference. Shown in prompt as:
```
- editor: nvim > vim > vi
- list: eza > exa > ls
```

### API Format (`api_style`)

| Value | Format | Services |
|-------|--------|----------|
| `openai` | OpenAI Chat Completions | Cerebras, Groq, Ollama, vLLM, Together, Fireworks, DeepSeek, ... |
| `anthropic` | Anthropic Messages | Anthropic Claude, proxy forwarding |

When omitted, auto-detected from URL: contains `anthropic` → `anthropic`, otherwise → `openai`.

URL handling:
- Trailing `/v1` is stripped automatically
- OpenAI: `{base_url}/v1/chat/completions`
- Anthropic: `{base_url}/v1/messages`

### Prompt `~/.local/bin/,.prompt.md`

Edit to customize LLM behavior (preferred tools, output format, platform, etc.). Delete to use built-in defaults.

### Response Cache

Responses are cached and reused when the same request is made again. Only cached when the user chooses to execute (not when declined).

```json
{
  "cache_size": 1000
}
```

Cache stored at `~/.local/bin/,.cache.json`.

## System Context

On each call, comma-cli injects non-private system info into the prompt:

- **Distro**: `/etc/os-release` (`PRETTY_NAME`)
- **Kernel**: `uname -srmo`
- **Arch**: `uname -m`
- **Shell**: `$SHELL`
- **CWD**: current working directory (sanitized)
- **User-installed packages**: detected via package manager

This lets the LLM generate correct commands for your platform (e.g. `apt` vs `pacman`).

## Privacy: Placeholders

**Private data (username, hostname, home path) is never sent to the API.** The prompt instructs the LLM to use placeholders, which are replaced locally after the response:

| Placeholder | Replaced with | Example |
|-------------|---------------|---------|
| `{{USER}}` | Current username | `miuzel` |
| `{{HOSTNAME}}` | Hostname | `myserver` |
| `{{HOME}}` | Home directory | `/home/miuzel` |

Flow:
```
User: "list my home directory"
        ↓
LLM sees: "User: {{USER}}, Home: {{HOME}}"  (no real values)
LLM outputs: "ls -la {{HOME}}"
        ↓
Local replace: "ls -la /home/miuzel"  (local only)
```

## Dangerous Command Detection

These trigger a red `⚠ DANGEROUS COMMAND ⚠` warning and require explicit confirmation:

- `rm -rf /`, `rm -rf ~`
- `dd if=... of=/dev/`
- `mkfs.*`
- Fork bomb `:(){ :|:& };:`
- `chmod -R 777 /`
- `shutdown`, `reboot`
- `curl/wget | sh/bash`
- `sudo rm`
- `git push --force`
- SQL `DROP TABLE` / `DROP DATABASE`

## Verbose Modes

```
, -v  "list files"     # show system prompt and LLM reply
, -vv "list files"     # add request URL, body, status, timing, raw response
```

## Dependencies

- Runtime: none (statically linked)
- Clipboard (optional): `wl-clipboard`, `xclip`, `xsel`, or `pbcopy`
- Build: Rust toolchain (`rustup`)

## Uninstall

```bash
./uninstall.sh
```

Or manually:

```bash
rm ~/.local/bin/, ~/.local/bin/,.config.json ~/.local/bin/,.prompt.md ~/.local/bin/,.cache.json
```

## License

[MIT](LICENSE)
