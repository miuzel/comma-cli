# `,` — 最小的 CLI，改变一切

> **别再搜 shell 命令了。** 输入你想要的，得到命令，执行。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/miuzel/comma-cli)](https://github.com/miuzel/comma-cli/releases)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey)]()

```bash
# Linux / macOS — 10 秒安装
curl -sSL https://github.com/miuzel/comma-cli/releases/latest/download/install.sh | bash
```

```powershell
# Windows (PowerShell) — 安装到 D:\tools\bin
$dir = "D:\tools\bin"; Invoke-WebRequest -Uri "https://github.com/miuzel/comma-cli/releases/latest/download/comma-windows-x86_64.zip" -OutFile "$dir\comma.zip"; Expand-Archive -Path "$dir\comma.zip" -DestinationPath $dir -Force; Rename-Item "$dir\comma.exe" "c.exe"; Remove-Item "$dir\comma.zip"
```

```bash
# 使用
, find all TODO comments in python files
# → rg -n TODO --type py  # Find TODO comments in Python files
# → [Enter] 执行
```

**就这样。** 无会话、无运行时、无依赖。只有一个 3MB 的二进制，把意图变成 shell 命令。

---

## 问题

你在终端里。你想：
- 把视频压缩到适合 Slack 发送
- 找出今天修改的大于 100MB 的文件
- 检查哪些端口被占用
- 从视频中提取音频

你知道*想要什么*，但记不住具体参数。所以你：
1. 打开浏览器
2. 搜索 "ffmpeg 压缩视频"
3. 读 3 个 Stack Overflow 回答
4. 复制粘贴一个可能能用的命令
5. 调试 5 分钟

**或者你可以直接输入：**
```bash
, compress video to 10mb
# → ffmpeg -i input.mp4 -b:v 8M -b:a 128k output.mp4
```

---

## `,` vs ChatGPT / Codex / Claude Code

**核心区别：** `,` 是**命令生成器**，不是 **Agent**。

| | `,` | ChatGPT / Codex / Claude Code |
|---|---|---|
| **做什么** | 生成一条 shell 命令 | 对话、写代码、执行任务 |
| **状态** | 无状态 — 每次调用独立 | 维护对话历史 |
| **范围** | 单条命令 | 多文件编辑、重构、调试 |
| **体积** | 3MB 二进制 | 100MB+ 运行时（Node.js、Python） |
| **启动** | 即时 | 2-5 秒冷启动 |
| **依赖** | 无 | Node.js、Python、npm 等 |
| **隐私** | 占位符（不发送个人数据） | 发送完整上下文 |
| **使用场景** | "我需要一条命令" | "我需要构建一个功能" |

### 什么时候用 `,`

```bash
# 你知道要什么，只需要命令
, find all TODO comments in python files
, compress video to 10mb
, check which ports are in use
```

### 什么时候用 ChatGPT/Claude

```
# 你需要对话，不只是命令
"帮我重构这个函数，让它更高效"
"调试为什么这个测试失败了"
"写一个处理 CSV 文件的 Python 脚本"
```

**这样理解：**
- ChatGPT 是**对话伙伴** — 你来我往地交流
- `,` 是**命令翻译器** — 你说想要什么，得到命令，完事

**`,` 的哲学：** 终端是用来*做事*的，不是*聊天*的。一个意图 → 一条命令 → 执行 → 完成。

---

## 功能

### 🔄 多 Provider Fallback

配置多个 provider，自动 fallback：

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

### ✏️ 执行前编辑

得到命令后，你可以：
- **Enter** — 直接执行
- **e** — 内联编辑（预填充，用方向键修改）
- **r** — 通过 LLM 微调（"加 --dry-run"）
- **Esc** — 取消

### 🤖 自动确认模式

用于脚本和 Agent，加 `!` 跳过所有确认：

```bash
, find large files !          # 自动执行
, compress video to 10mb !    # 自动探索 + 自动执行
```

### 🔍 智能工具发现

模型会先检查装了什么工具：

```
$ , compress this image
▸ Checking: convert magick ffmpeg
  Available: ffmpeg
  Not found: convert, magick
ffmpeg -i input.png -quality 85 output.jpg
```

### 📦 自动更新

检查更新并从 GitHub releases 更新二进制：

```bash
, --update
# ▸ Checking for updates (current: 0.14.0)...
#   Update available: 0.14.0 → 0.15.0
# ▸ Updated to 0.15.0
```

下载的压缩包会先对照 release 的 `sha256sums.txt` 校验，然后才替换二进制。

### 📚 探索模式

不确定工具用法时，模型会先运行帮助：

```
$ , compress video using ffmpeg
▸ Exploring: ffmpeg -h
▸ Learning from output...
ffmpeg -i input.mp4 -b:v 8M output.mp4
```

探测命令运行前总会询问确认（单个探测也一样），除非加 `!`。

---

## 推荐模型

`,` 支持任何 OpenAI 或 Anthropic 兼容 API。以下是一些推荐：

### 🚀 快速 & 免费

| 提供商 | 模型 | 速度 | 费用 | 适用场景 |
|--------|------|------|------|----------|
| [Cerebras](https://cerebras.ai) | `gemma-4-31b` | ⚡ 超快 | 免费额度 | 快速命令、高吞吐 |
| [Groq](https://groq.com) | `llama-3.1-8b-instant` | ⚡ 超快 | 免费额度 | 低延迟、实时使用 |

### 💻 编程优化

| 提供商 | 模型 | 适用场景 |
|--------|------|----------|
| [Moonshot](https://kimi.moonshot.cn) | `kimi-k2.7-coding` | Shell 命令、代码生成 |
| [DeepSeek](https://deepseek.com) | `deepseek-v4-flash` | 快速推理、编程任务 |

### 🏠 本地运行（无需 API Key）

| 工具 | 模型 | 适用场景 |
|------|------|----------|
| [Ollama](https://ollama.ai) | `qwen3.6-35b-a3b` | 隐私保护、离线使用 |
| [vLLM](https://vllm.ai) | 任意模型 | 自托管、高吞吐 |

### 配置示例

**Cerebras（快速、免费）：**
```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "your-api-key",
  "model": "gemma-4-31b"
}
```

**Ollama（本地）：**
```json
{
  "base_url": "http://localhost:11434/v1",
  "auth_token": "ollama",
  "model": "qwen3.6-35b-a3b"
}
```

**DeepSeek：**
```json
{
  "base_url": "https://api.deepseek.com/v1",
  "auth_token": "your-api-key",
  "model": "deepseek-v4-flash"
}
```

**多 Provider Fallback：**
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

## 快速开始

### 一次性模式

```bash
, find all TODO comments in python files
# → rg -n TODO --type py  # Find TODO comments in Python files

, list files larger than 1G
# → fd --size +1G  # Find files larger than 1GB

, what is my ip
# → curl -s ifconfig.me  # Get public IP address
```

只有第一个意图词*之前*的参数会被解析为 flag — 之后的内容原样作为意图文本，`--` 可显式结束 flag 解析。所以包含 `-` 词的意图可以直接用：

```bash
, grep -v pattern        # 意图："grep -v pattern"
, use curl -V            # 意图："use curl -V"
```

### 管道模式

```bash
echo "find large files" | ,     # 生成命令，然后从 stdin 读一行 "y" 确认
echo "find large files" | , !   # 跳过确认（脚本 / Agent 用）
```

stdin 为管道时，`,` 绝不会自动执行：它会从 stdin 读一行，只有该行是 `y` 才运行命令。

### 交互模式

```bash
,
> find large files
fd --size +100M  # Find files larger than 100MB
> sort by size descending
fd --size +100M -x ls -lh {} + | sort -k5 -h -r
> x  # 执行
```

### 快捷键

| 按键 | 作用 |
|------|------|
| `Tab` | 补全文件名 |
| `↑`/`↓` | 选择候选 |
| `Enter` | 确认 / 执行 |
| `Esc` | 取消 |
| `e` | 编辑命令 |
| `r` | 通过 LLM 微调 |
| `x` | 执行（交互模式） |
| `c` | 复制到剪贴板 |
| `q` | 退出 |

---

## 配置

### 优先级

```
COMMA_* 环境变量
  ↓
~/.local/bin/,.config.json
  ↓
~/.claude/settings.json
  ↓
内置默认值
```

### 环境变量

```bash
export COMMA_BASE_URL="https://api.cerebras.ai/v1"
export COMMA_API_KEY="csk-xxx"
export COMMA_MODEL="llama-3.3-70b"
export COMMA_API_STYLE="openai"
```

### 最小配置

```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "csk-xxx",
  "model": "llama-3.3-70b"
}
```

### 工具偏好

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

### 响应缓存

重复的意图会从 `~/.local/bin/,.cache.json` 直接回答（默认上限 1000 条）。在任何网络请求之前，缓存会按回退顺序对所有已配置的模型依次检查，因此命中回退模型的缓存可以避免等待缓慢或不可达的主模型调用。在配置中设 `"cache_size": 0` 可完全禁用缓存。

### Reasoning（Anthropic）

对 Anthropic 模型，`"reasoning": <tokens>` 按给定预算启用 extended thinking。`max_tokens` 会自动提高，因此 ≥ 1024 的预算可以正常使用。

---

## 隐私

**不发送个人数据。** 模型使用占位符：

```
用户: "list my home directory"
        ↓
LLM 看到: "User: {{USER}}, Home: {{HOME}}"  (不包含真实值)
LLM 输出: "ls -la {{HOME}}"
        ↓
本地替换: "ls -la /home/miuzel"  (仅在本机发生)
```

---

## 系统上下文

每次调用，comma-cli 会注入：
- 发行版、内核、架构
- Shell、当前目录
- 用户安装的包

确保生成适合你平台的命令（`apt` vs `pacman`，`brew` vs `port`）。

---

## 安装

### Linux / macOS（自动检测）

```bash
curl -sSL https://github.com/miuzel/comma-cli/releases/latest/download/install.sh | bash
```

安装脚本会在可用时对照 release 的 `sha256sums.txt` 校验压缩包的 SHA-256。

### Windows (PowerShell)

```powershell
# 安装到 D:\tools\bin（可自行修改路径）
$dir = "$env:USERPROFILE\.local\bin"; New-Item -ItemType Directory -Force -Path $dir | Out-Null; Invoke-WebRequest -Uri "https://github.com/miuzel/comma-cli/releases/latest/download/comma-windows-x86_64.zip" -OutFile "$dir\comma.zip"; Expand-Archive -Path "$dir\comma.zip" -DestinationPath $dir -Force; Remove-Item "$dir\comma.zip"; Write-Host "已安装到 $dir\comma.exe — 将 $dir 加入 PATH，然后使用: comma <intent>"
```

> **注意：** PowerShell 中 `,` 是保留关键字。如需更短的名字，可将 exe 重命名（如 `c.exe`）。

### 手动下载

从 [releases](https://github.com/miuzel/comma-cli/releases/latest) 下载对应平台的压缩包：

| 平台 | 压缩包 |
|------|--------|
| Linux x86_64 | `comma-linux-x86_64.tar.gz` |
| Linux aarch64 | `comma-linux-aarch64.tar.gz` |
| macOS x86_64 | `comma-macos-x86_64.tar.gz` |
| macOS aarch64 (Apple Silicon) | `comma-macos-aarch64.tar.gz` |
| Windows x86_64 | `comma-windows-x86_64.zip` |

```bash
# 示例：Linux x86_64
tar xzf comma-linux-x86_64.tar.gz
mv comma ~/.local/bin/,
```

### 更新

```bash
, --update
```

### 从源码构建

```bash
git clone https://github.com/miuzel/comma-cli.git
cd comma-cli
./build.sh
```

### 首次配置

安装后需要配置模型。编辑 `~/.local/bin/,.config.json`：

```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "your-api-key-here",
  "model": "gemma-4-31b"
}
```

或使用环境变量：

```bash
export COMMA_BASE_URL="https://api.cerebras.ai/v1"
export COMMA_API_KEY="your-api-key-here"
export COMMA_MODEL="gemma-4-31b"
```

**免费选项：**
- [Cerebras](https://cerebras.ai) — 免费额度，超快，无需信用卡
- [Groq](https://groq.com) — 免费额度，低延迟
- [Ollama](https://ollama.ai) — 本地运行，无需 API Key，需要 8GB+ 内存

### 卸载

```bash
./uninstall.sh
```

---

## 谁需要这个？

- **运维**：快速一行命令，不用翻 man 手册
- **开发者**：把意图转成 `ffmpeg`、`find`、`tar` 命令
- **DevOps**：检查端口、进程、磁盘使用
- **任何人**：用终端但讨厌记参数

---

## 许可证

[MIT](LICENSE)

---

> **小就是大。** 逗号是最小的标点 — 却能改变一切。
