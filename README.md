# `,` — The smallest CLI that changes everything

> **Stop googling shell commands.** Type what you want, get the command, run it.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/miuzel/comma-cli)](https://github.com/miuzel/comma-cli/releases)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macos-lightgrey)]()

```bash
# Install in 10 seconds
curl -sSL https://github.com/miuzel/comma-cli/releases/latest/download/install.sh | bash
```

```bash
# Use it
, find all TODO comments in python files
# → rg -n TODO --type py  # Find TODO comments in Python files
# → [Enter] to execute
```

**That's it.** No sessions, no runtime, no dependencies. Just a 3MB binary that turns intent into shell commands.

---

## The problem

You're in the terminal. You want to:
- Compress a video for Slack
- Find files modified today larger than 100MB
- Check which ports are in use
- Extract audio from a video file

You know *what* you want, but can't remember the exact flags. So you:
1. Open a browser
2. Search "ffmpeg compress video"
3. Read 3 Stack Overflow answers
4. Copy-paste something that might work
5. Debug it for 5 minutes

**Or you could just type:**
```bash
, compress video to 10mb
# → ffmpeg -i input.mp4 -b:v 8M -b:a 128k output.mp4
```

---

## `,` vs ChatGPT / Codex / Claude Code

**The key difference:** `,` is a **command generator**, not an **agent**.

| | `,` | ChatGPT / Codex / Claude Code |
|---|---|---|
| **What it does** | Generates ONE shell command | Has conversations, writes code, executes tasks |
| **State** | Stateless — no memory between calls | Maintains conversation history |
| **Scope** | Single command | Multi-file editing, refactoring, debugging |
| **Size** | 3MB binary | 100MB+ runtime (Node.js, Python) |
| **Startup** | Instant | 2-5s cold start |
| **Dependencies** | None | Node.js, Python, npm, etc. |
| **Privacy** | Placeholders (no personal data sent) | Full context sent |
| **Use case** | "I need a command" | "I need to build a feature" |

### When to use `,`

```bash
# You know what you want, just need the command
, find all TODO comments in python files
, compress video to 10mb
, check which ports are in use
```

### When to use ChatGPT/Claude

```
# You need a conversation, not just a command
"Help me refactor this function to be more efficient"
"Debug why this test is failing"
"Write a Python script that processes CSV files"
```

**Think of it this way:**
- ChatGPT is a **conversation partner** — you talk back and forth
- `,` is a **command translator** — you say what you want, get the command, done

**The `,` philosophy:** The terminal is for *doing*, not *talking*. One intent → one command → execute → done.

---

## Features

### 🔄 Multi-provider fallback

Configure multiple providers with automatic fallback:

```json
{
  "providers": {
    "cerebras": {
      "base_url": "https://api.cerebras.ai/v1",
      "auth_token": "csk-xxx",
      "api_style": "openai"
    },
    "anthropic": {
      "base_url": "https://api.anthropic.com",
      "auth_token": "sk-ant-xxx"
    }
  },
  "models": [
    {"provider": "cerebras", "model": "llama-3.3-70b", "retries": 2},
    {"provider": "anthropic", "model": "claude-sonnet-4-20250514", "retries": 1}
  ]
}
```

### ✏️ Edit before execution

After getting a command, you can:
- **Enter** — Execute as-is
- **e** — Edit inline (pre-filled, use arrow keys)
- **r** — Refine via LLM ("add --dry-run")
- **Esc** — Cancel

### 🤖 Auto-confirm mode

For scripts and agents, add `!` to skip all confirmations:

```bash
, find large files !          # auto-execute
, compress video to 10mb !    # auto-explore + auto-execute
```

### 🔍 Smart tool discovery

The model checks what's installed before suggesting commands:

```
$ , compress this image
▸ Checking: convert magick ffmpeg
  Available: ffmpeg
  Not found: convert, magick
ffmpeg -i input.png -quality 85 output.jpg
```

### 📦 Self-update

Check for updates and update the binary from GitHub releases:

```bash
, --update
# ▸ Checking for updates (current: 0.14.0)...
#   Update available: 0.14.0 → 0.15.0
# ▸ Updated to 0.15.0
```

### 📚 Exploration mode

When unsure about a tool, the model runs help first:

```
$ , compress video using ffmpeg
▸ Exploring: ffmpeg -h
▸ Learning from output...
ffmpeg -i input.mp4 -b:v 8M output.mp4
```

---

## Recommended Models

`,` works with any OpenAI or Anthropic compatible API. Here are some great options:

### 🚀 Fast & Free

| Provider | Model | Speed | Cost | Best for |
|----------|-------|-------|------|----------|
| [Cerebras](https://cerebras.ai) | `gemma-4-31b` | ⚡ Ultra-fast | Free tier | Quick commands, high throughput |
| [Groq](https://groq.com) | `llama-3.1-8b-instant` | ⚡ Ultra-fast | Free tier | Low latency, real-time use |

### 💻 Coding-Optimized

| Provider | Model | Best for |
|----------|-------|----------|
| [Moonshot](https://kimi.moonshot.cn) | `kimi-k2.7-coding` | Shell commands, code generation |
| [DeepSeek](https://deepseek.com) | `deepseek-v4-flash` | Fast inference, coding tasks |

### 🏠 Local (No API key needed)

| Tool | Model | Best for |
|------|-------|----------|
| [Ollama](https://ollama.ai) | `qwen3.6-35b-a3b` | Privacy, offline use |
| [vLLM](https://vllm.ai) | Any model | Self-hosted, high throughput |

### Example configs

**Cerebras (fast, free):**
```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "your-api-key",
  "model": "gemma-4-31b"
}
```

**Ollama (local):**
```json
{
  "base_url": "http://localhost:11434/v1",
  "auth_token": "ollama",
  "model": "qwen3.6-35b-a3b"
}
```

**DeepSeek:**
```json
{
  "base_url": "https://api.deepseek.com/v1",
  "auth_token": "your-api-key",
  "model": "deepseek-v4-flash"
}
```

**Multi-provider fallback:**
```json
{
  "providers": {
    "cerebras": {
      "base_url": "https://api.cerebras.ai/v1",
      "auth_token": "csk-xxx"
    },
    "deepseek": {
      "base_url": "https://api.deepseek.com/v1",
      "auth_token": "sk-xxx"
    },
    "ollama": {
      "base_url": "http://localhost:11434/v1",
      "auth_token": "ollama"
    }
  },
  "models": [
    {"provider": "cerebras", "model": "gemma-4-31b", "retries": 2},
    {"provider": "deepseek", "model": "deepseek-v4-flash", "retries": 1},
    {"provider": "ollama", "model": "qwen3.6-35b-a3b", "retries": 1}
  ]
}
```

---

## Quick start

### One-shot mode

```bash
, find all TODO comments in python files
# → rg -n TODO --type py  # Find TODO comments in Python files

, list files larger than 1G
# → fd --size +1G  # Find files larger than 1GB

, what is my ip
# → curl -s ifconfig.me  # Get public IP address
```

### Interactive mode

```bash
,
> find large files
fd --size +100M  # Find files larger than 100MB
> sort by size descending
fd --size +100M -x ls -lh {} + | sort -k5 -h -r
> x  # execute
```

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Tab` | Autocomplete filename |
| `↑`/`↓` | Select candidate |
| `Enter` | Confirm / Execute |
| `Esc` | Cancel |
| `e` | Edit command |
| `r` | Refine via LLM |
| `x` | Execute (interactive mode) |
| `c` | Copy to clipboard |
| `q` | Quit |

---

## Configuration

### Priority

```
COMMA_* environment variables
  ↓
~/.local/bin/,.config.json
  ↓
~/.claude/settings.json
  ↓
Built-in defaults
```

### Environment variables

```bash
export COMMA_BASE_URL="https://api.cerebras.ai/v1"
export COMMA_API_KEY="csk-xxx"
export COMMA_MODEL="llama-3.3-70b"
export COMMA_API_STYLE="openai"
```

### Minimal config

```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "csk-xxx",
  "model": "llama-3.3-70b"
}
```

### Tool preferences

```json
{
  "prefer": {
    "editor": ["nvim", "vim"],
    "list": ["eza", "ls"],
    "grep": ["rg", "grep"],
    "find": ["fd", "find"]
  }
}
```

---

## Privacy

**No personal data is sent to the API.** The model uses placeholders:

```
User: "list my home directory"
        ↓
LLM sees: "User: {{USER}}, Home: {{HOME}}"  (no real values)
LLM outputs: "ls -la {{HOME}}"
        ↓
Local replace: "ls -la /home/miuzel"  (local only)
```

---

## System context

On each call, comma-cli injects:
- Distro, kernel, architecture
- Shell, current directory
- User-installed packages

This ensures correct commands for your platform (`apt` vs `pacman`, `brew` vs `port`).

---

## Install

```bash
curl -sSL https://github.com/miuzel/comma-cli/releases/latest/download/install.sh | bash
```

Update to the latest version:

```bash
, --update
```

Or build from source:

```bash
git clone https://github.com/miuzel/comma-cli.git
cd comma-cli
./build.sh
```

### First-time setup

After install, configure a model. Edit `~/.local/bin/,.config.json`:

```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "your-api-key-here",
  "model": "gemma-4-31b"
}
```

Or use environment variables:

```bash
export COMMA_BASE_URL="https://api.cerebras.ai/v1"
export COMMA_API_KEY="your-api-key-here"
export COMMA_MODEL="gemma-4-31b"
```

**Free options to get started:**
- [Cerebras](https://cerebras.ai) — Free tier, ultra-fast, no credit card needed
- [Groq](https://groq.com) — Free tier, low latency
- [Ollama](https://ollama.ai) — Local, no API key, requires 8GB+ RAM

### Uninstall

```bash
./uninstall.sh
```

---

## Who needs this?

- **Sysadmins**: Quick one-liners without man page archaeology
- **Developers**: Convert intent to `ffmpeg`, `find`, `tar` commands
- **DevOps**: Check ports, processes, disk usage
- **Anyone** who uses the terminal and hates memorizing flags

---

## License

[MIT](LICENSE)

---

> **Small is big.** A comma is the smallest punctuation — yet it changes everything.
