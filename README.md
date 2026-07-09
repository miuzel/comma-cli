# `,` — LLM 驱动的 Shell 命令生成器

输入自然语言意图，调用 LLM 生成可执行的 shell 命令。支持交互式改进、危险命令检测。

## 安装

```bash
cargo build --release
cp target/release/comma ~/.local/bin/,
cp prompt.md ~/.local/bin/,.prompt.md
cp config.json ~/.local/bin/,.config.json
```

安装后文件布局：

```
~/.local/bin/
├── ,              # 二进制
├── ,.config.json  # 配置（可选，优先级高于 Claude 设置）
└── ,.prompt.md    # 系统提示词（可编辑）
```

## 用法

```bash
, what is my ip              # 一次性：生成命令 → 确认/执行
, list files larger than 1G  # 生成 du/find 命令
,                            # 交互模式：多轮对话改进命令
, -h                         # 帮助
```

### 一次性模式

```
$ , find all TODO comments in python files
▸ Model: mimo-v2.5-pro
grep -rn "TODO" --include="*.py" .
Execute? [y/N]
```

输入 `y` 执行，其他任意输入取消。

### 交互模式

```
$ ,
▸ Interactive mode (model: mimo-v2.5-pro). Tab completes filenames. 'q' to quit, 'x' to exec, 'c' to copy.
> find large files
find . -type f -size +100M -exec ls -lh {} \;
> sort by size descending
find . -type f -size +100M -exec ls -lh {} \; | sort -k5 -h -r
> x
▸ Running: find . -type f -size +100M -exec ls -lh {} \; | sort -k5 -h -r
```

输入时按 **Tab** 键可自动补全当前目录下的文件/目录名。支持路径补全（如 `./src/m` → `./src/main.rs`）。

| 命令 | 作用 |
|------|------|
| `Tab` | 补全文件名 |
| `x` / `exec` | 执行当前命令 |
| `c` / `copy` | 复制到剪贴板 |
| `q` / `quit` / `exit` | 退出 |
| 其他任意文本 | 发送给 LLM 改进命令 |

## 配置

### 配置优先级

```
~/.local/bin/,.config.json  >  ~/.claude/settings.json  >  内置默认值
```

只有当本地配置文件中某个字段为空字符串或缺失时，才回退到 Claude 的设置。

### 本地配置 `~/.local/bin/,.config.json`

**Anthropic（Claude）示例：**

```json
{
  "base_url": "https://api.anthropic.com",
  "auth_token": "sk-ant-xxx",
  "model": "claude-sonnet-4-20250514",
  "api_style": "anthropic"
}
```

**OpenAI 兼容示例（Cerebras、Groq、Ollama、vLLM 等）：**

```json
{
  "base_url": "https://api.cerebras.ai/v1",
  "auth_token": "csk-xxx",
  "model": "llama-3.3-70b",
  "api_style": "openai"
}
```

| 字段 | 说明 | 回退来源 |
|------|------|----------|
| `base_url` | API 端点 | `ANTHROPIC_BASE_URL` in settings.json |
| `auth_token` | API 密钥 | `ANTHROPIC_AUTH_TOKEN` in settings.json |
| `model` | 模型名称 | `ANTHROPIC_MODEL` in settings.json |
| `api_style` | API 格式（见下文） | 自动检测（含 `anthropic` 的 URL → anthropic，其余 → openai） |

字段留空字符串 `""` 视为未设置，会回退。

### API 格式 (`api_style`)

| 值 | 格式 | 适用服务 |
|----|------|----------|
| `openai` | OpenAI Chat Completions | Cerebras, Groq, Ollama, vLLM, Together, Fireworks, DeepSeek, ... |
| `anthropic` | Anthropic Messages | Anthropic Claude, 代理转发 |

省略时根据 URL 自动判断：URL 包含 `anthropic` → `anthropic`，否则 → `openai`。

URL 处理规则：
- 自动去除末尾 `/v1`，由程序拼接正确路径
- OpenAI：`{base_url}/v1/chat/completions`
- Anthropic：`{base_url}/v1/messages`

### 提示词 `~/.local/bin/,.prompt.md`

编辑此文件可自定义 LLM 行为（偏好工具、输出格式、平台等）。删除此文件会使用内置默认提示词。

#### 系统上下文

每次调用 LLM 时，程序会自动采集以下**非私密**信息并注入提示词：

- **发行版**：`/etc/os-release` (`PRETTY_NAME`)
- **内核**：`uname -srmo`
- **架构**：`uname -m`
- **Shell**：`$SHELL`
- **当前目录**：`cwd`
- **已安装包列表**：自动检测包管理器（dpkg/rpm/pacman/apk）并列出前 100-200 个包
- **可用工具**：检测 git、curl、python3、node、docker、rustc 等常用工具

这些信息让 LLM 能根据你的实际环境生成正确的命令（例如用 `apt` 而非 `pacman`）。

#### 隐私保护：占位符

**私密信息（用户名、主机名、家目录）不会发送给 API。** 提示词指示 LLM 在命令中使用占位符，程序收到响应后在本地替换为真实值：

| 占位符 | 替换为 | 示例 |
|--------|--------|------|
| `{{USER}}` | 当前用户名 | `miuzel` |
| `{{HOSTNAME}}` | 主机名 | `myserver` |
| `{{HOME}}` | 家目录路径 | `/home/miuzel` |

流程：
```
用户: "list my home directory"
        ↓
LLM 看到: "User: {{USER}}, Home: {{HOME}}"  (不包含真实值)
LLM 输出: "ls -la {{HOME}}"
        ↓
本地替换: "ls -la /home/miuzel"  (仅在本机发生)
```

提示词中可使用 `{{SYSTEM_CONTEXT}}` 注入完整系统信息块。

## 危险命令检测

以下命令会触发红色 `⚠ DANGEROUS COMMAND ⚠` 警告，执行前需要明确输入 `y` 确认：

- `rm -rf /`、`rm -rf ~`
- `dd if=... of=/dev/`
- `mkfs.*`
- Fork bomb `:(){ :|:& };:`
- `chmod -R 777 /`
- `shutdown`、`reboot`
- `curl/wget | sh/bash`
- `sudo rm`
- `git push --force`
- SQL `DROP TABLE` / `DROP DATABASE`

## 无状态设计

- 不保存任何会话、历史、日志
- 每次调用都是独立的 HTTP 请求
- 交互模式的对话仅存在于内存中，退出即消失
- 不写入临时文件，不创建 session 目录

## 依赖

- 运行时：无（静态链接）
- 剪贴板功能（可选）：`wl-clipboard`、`xclip`、`xsel` 或 `pbcopy`
- 编译时：Rust toolchain（`rustup`）

## 卸载

```bash
rm ~/.local/bin/, ~/.local/bin/,.config.json ~/.local/bin/,.prompt.md
```
