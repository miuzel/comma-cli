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
▸ Interactive mode (model: mimo-v2.5-pro). Type 'q' to quit, 'x' to execute, 'c' to copy.
> find large files
find . -type f -size +100M -exec ls -lh {} \;
> sort by size descending
find . -type f -size +100M -exec ls -lh {} \; | sort -k5 -h -r
> x
▸ Running: find . -type f -size +100M -exec ls -lh {} \; | sort -k5 -h -r
```

| 命令 | 作用 |
|------|------|
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

```json
{
  "base_url": "https://api.anthropic.com",
  "auth_token": "sk-ant-xxx",
  "model": "claude-sonnet-4-20250514"
}
```

| 字段 | 说明 | 回退来源 |
|------|------|----------|
| `base_url` | API 端点 | `ANTHROPIC_BASE_URL` in settings.json |
| `auth_token` | API 密钥 | `ANTHROPIC_AUTH_TOKEN` in settings.json |
| `model` | 模型名称 | `ANTHROPIC_MODEL` in settings.json |

字段留空字符串 `""` 视为未设置，会回退。

### 提示词 `~/.local/bin/,.prompt.md`

编辑此文件可自定义 LLM 行为（偏好工具、输出格式、平台等）。删除此文件会使用内置默认提示词。

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
