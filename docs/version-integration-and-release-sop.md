# 版本集成分支管理与发布 SOP

> **文件路径**：`/home/miuzel/workspace/personal/comma-cli/docs/version-integration-and-release-sop.md`
> **依据**：本工作区 2026-08 / 2026-09 的真实执行记录 —— v0.27.1 正式发布，以及 v0.28.0 的 g-005 / g-011 / g-012 / g-013 四卡并发开发、集成（15 处冲突）与「已验」构建。
> **适用**：本工作区后续所有「多目标并行开发 → 版本集成分支 → 负责人验收 → 正式发布」。**照此执行。**
> **语言约定**：正文中文；分支名、tag 名、命令、路径、配置键一律保留原文。

---

## 0. 术语与一次性环境准备

| 名称 | 含义 |
|---|---|
| `main` | 已交付主干 |
| `vX.Y.Z-test` | **版本集成分支**（本版本多卡集成与验收场） |
| `g-XXX-att-0N` | 单个 goal 的 attempt 分支（子代理执行用） |
| `.worktrees/<goal>-att-0N` | attempt 的独立工作树 |
| `vX.Y.Z` | 正式发布 tag（**唯一**允许以 `v` 开头的 tag） |
| `<version>-verified` | 「已验」内部 tag（**绝不以 `v` 开头**） |

```bash
# 本手册统一使用该变量；命令中的绝对路径也可直接复制执行
REPO=/home/miuzel/workspace/personal/comma-cli
```

**ssh 在本机不可用**（详见第 4 节），所有远端操作统一用 gh 凭据走 HTTPS：

```bash
cd /home/miuzel/workspace/personal/comma-cli
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' ls-remote https://github.com/miuzel/comma-cli.git refs/heads/main
```

---

## 1. 分支模型

### 1.1 三层分支

```
main                         ← 只放「已交付」内容
 ├─ vX.Y.Z-test              ← 版本集成分支（从当时的 main 拉出）
 │   ├─ g-005-att-01         ← 单卡 attempt 分支
 │   ├─ g-012-att-01
 │   └─ g-013-att-01
 └─ vX.Y.Z (tag)             ← 发布点
```

**规则**
1. **单卡永不直接进 `main`**：一律先合入当版本的集成分支 `vX.Y.Z-test`。
2. **集成分支从「当时的 main」拉出**，因此天然包含此前已并入 main 的改动。
   - 实例：`v0.28.0-test` 从 `6fb6c5f` 拉出，而 `6fb6c5f` 已含 g-011 的 `|||` 修复。
3. 正式发布时，把集成分支内容落到 `main`（或等价地确认 main 已含全部交付内容）后再打 `vX.Y.Z`。

### 1.2 工作树（worktree）规则

- **子代理一律在独立 worktree 内开发**，绝不直接改 `main` 工作树。
- worktree 由 `graph_start_attempt` **自动创建**：路径 `/home/miuzel/workspace/personal/comma-cli/.worktrees/<goal>-att-0N`，分支 `g-XXX-att-0N`，基线为派发时的 `main`。
  实例（本会话现状）：
  ```
  /home/miuzel/workspace/personal/comma-cli                      6fb6c5f [main]
  /home/miuzel/workspace/personal/comma-cli/.worktrees/g-005-att-01  8c20c14 [g-005-att-01]
  /home/miuzel/workspace/personal/comma-cli/.worktrees/g-012-att-01  1e4ba03 [g-012-att-01]
  /home/miuzel/workspace/personal/comma-cli/.worktrees/g-013-att-01  03caa0e [g-013-att-01]
  /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test  d5d53b0 [v0.28.0-test]
  ```
- **集成分支也建议开独立 worktree**，这样 main 工作树不被切换、构建产物路径稳定：
  ```bash
  cd /home/miuzel/workspace/personal/comma-cli
  git worktree add .worktrees/v0.28.0-test v0.28.0-test
  ```
- `.worktrees/` 已在 `.gitignore` 中，不会污染仓库。
- **`.dsh-graph/` 看板数据只在主工作树写**（worktree 分支不隔离它）。同时**禁止**用 `git add -f` / `git rm --cached` 把 `.dsh-graph` 塞进父代码仓库（`.gitignore` 用 `**/.dsh-graph` 通配兜底）——它归内层独立仓库管理。

---

## 2. 集成 SOP：从「开发完成」到「可测构建」

> 本会话实际路径（v0.28.0：g-005 / g-012 / g-013 三卡 → `v0.28.0-test`）。

### 步骤 1 · 逐卡复核（**不采信子代理报告**）

对每张声明完成的卡：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/g-012-att-01
git branch --show-current          # 确认在正确分支
git status --short                 # 应为空（工作树干净）
git log --oneline -2               # 记下 commit
git show <commit> --stat           # 改动范围是否与声明一致
git show <commit> --name-only      # 有无越界改写无关文件
```

要点：
- **读实际 diff**（关键实现逐行看），按判据**逐条**验证；
- 涉及隐私/安全红线的，确认断言是**真断言**（能否在旧代码上 FAIL），不是空转；
- 独立复跑门禁（见步骤 2），**不要**采信子代理贴出的 PASS 文本。

### 步骤 2 · 门禁命令（固定四连 + 一条 shell 套件）

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/g-012-att-01
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo build --release
./target/release/comma --test
./scripts/release-notes-test.sh        # CI gate 5：仅 shell + git，三平台都跑
```

- 四项 Rust 门禁全过才算通过；`release-notes-test.sh` 只在改动 `scripts/release-notes.sh` 或
  `ci.yml` 时才需要单独复跑（CI 三平台都会跑它，见 4.6/4.7 的平台预演）。
- **断言数校准**：`--test` 的总数应等于「基线 + 新增断言数」，可反证「测的确实是这个提交」。
  本会话数据：基线 **228** → g-012 后 **263**（+35）→ g-005 后 **244**（单卡，+16）→ g-013 后 **338**（单卡，+110）→ **三卡集成后 389** → 0.28.0 全量 **542**。

### 步骤 3 · 把已验卡按顺序合入**集成分支**（不是 main）

```bash
cd /home/miuzel/workspace/personal/comma-cli
# 1) 建集成分支（从当时 main 拉出，天然含此前已入 main 的修复）
git branch v0.28.0-test main
# 2) 给它开 worktree（不打扰 main 工作树，且构建路径稳定）
git worktree add .worktrees/v0.28.0-test v0.28.0-test

# 3) 逐卡合入（--no-ff 保留合并点，便于回溯「哪张卡带来什么」）
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
git merge --no-ff g-012-att-01 -m "Merge g-012: REPL next-step hint after each generated command"
git merge --no-ff g-005-att-01 -m "Merge g-005: opt-in REPL input history (default off)"
git merge --no-ff g-013-att-01 -m "Merge g-013: auto-refine after a failed command + /refine entry"

# 4) 查看冲突
git status --short | grep -E '^(UU|AA|DD|AU|UA|DU|UD)'
```

> 本会话三卡合并时，g-012 与 g-005 **自动合并无冲突**；g-013 产生 **15 处冲突**：
> `locales/*.toml`（9）、`README.md`、`README.zh-CN.md`、`AGENTS.md`、`src/main.rs`、`src/setup.rs`、`src/tests.rs`。

### 步骤 4 · 解冲突（本仓库实战规则）

#### 4.1 `locales/*.toml` —— 按键名有序并集

多卡各自**新增不同键**（也可能同时改写同一键，如 `welcome`）。规则：
**按键名做有序并集，同键取较新一侧（theirs）的值；遇到纯 `|`/空行丢弃。**

本会话的脚本化做法（比手改 9 个文件可靠）：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
python3 - <<'PY'
import glob

def resolve(text):
    out, lines, i = [], text.split('\n'), 0
    while i < len(lines):
        if lines[i].startswith('<<<<<<<'):
            ours, theirs = [], []
            i += 1
            while not lines[i].startswith('======='): ours.append(lines[i]); i += 1
            i += 1
            while not lines[i].startswith('>>>>>>>'): theirs.append(lines[i]); i += 1
            i += 1
            # 有序并集：按键名去重，theirs 覆盖同键值但保留首次出现位置
            order, val = [], {}
            for ln in ours + theirs:
                if '=' not in ln: continue
                k = ln.split('=', 1)[0].strip()
                if k not in val: order.append(k)
                val[k] = ln
            out.extend(val[k] for k in order)
        else:
            out.append(lines[i]); i += 1
    return '\n'.join(out)

for f in sorted(glob.glob('locales/*.toml')):
    t = open(f, encoding='utf-8').read()
    if '<<<<<<<' not in t: continue
    r = resolve(t)
    assert '<<<<<<<' not in r and '>>>>>>>' not in r, f
    open(f, 'w', encoding='utf-8').write(r)
    print('resolved', f)
PY
```

**解完必须校验**（否则可能出现某语言缺键 → 运行时静默回落英文）：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
python3 - <<'PY'
import glob, tomllib

def flat(f):
    d = tomllib.load(open(f, 'rb')); out = set()
    for sec, kv in d.items():
        if isinstance(kv, dict):
            for k in kv: out.add(f"{sec}.{k}")
        else: out.add(sec)
    return out

en = flat('locales/en.toml')
ok = True
for f in sorted(glob.glob('locales/*.toml')):
    s = flat(f)
    if s != en:
        ok = False
        print(f"DIFF {f}: missing={sorted(en - s)} extra={sorted(s - en)}")
print("9 个 locale 键集合与 en 完全一致" if ok else "MISMATCH —— 必须修")
PY
```

本会话结果：9 个文件各 **186** 键，键集合与 `en.toml` 完全一致，`tomllib` 解析通过。

#### 4.2 `src/setup.rs` —— `--setup` 菜单抢索引

多卡都往 `--setup` 主菜单加项时，会**抢同一个索引**。本会话 g-005（history）与 g-013（auto_refine）**都拿到了 `Some(2)`**。

必须重排为：

```
0 = llm        1 = search     2 = history
3 = auto_refine               4 = save        5 = discard
```

改索引时**必须同步顺延后续分支**（`save`、`discard` 都要 +1），否则会串到别的操作。

同时守住**合并式保存的显式写入规则**：保存是「合并进 existing」，**每个受管键都必须显式写入**，否则用户的选择会被静默丢弃。
本会话的处理：`entries_to_json` 用 4 参版（负责写 `history`），调用点再显式补 `auto_refine`：

```rust
let mut json = entries_to_json(&entries, &search, history, &existing);
json["auto_refine"] = json!(auto_refine);
```

#### 4.3 `src/main.rs` —— 一卡重构 + 另一卡加行为

若一卡做了**重构**（g-013 把 refine 抽成 `do_refine(...)`），另一卡在**旧代码**上加行为（g-012 加 `print_cmd_hint()`），
合并必须 **取重构版并保留新行为**。

本会话结果：采用 `do_refine` 版本，并把 `print_cmd_hint()` **补回重构后的 3 条 refine 路径**
（`x`→`r` 精炼、失败自动 refine、主提示符 `/refine`），加上原有的「新意图生成」路径，共 **4 个调用点**，保持「每条新生成命令后恰好一行提示」的一致语义。

#### 4.4 散文文件（`AGENTS.md` / `README.md` / `README.zh-CN.md`）

- 双方**各自新增**的内容**都保留**；
- 若一方是**超集改写**，要把它**漏掉的另一方内容补回**。
  本会话实例：g-013 的 `AGENTS.md` 中 `src/main.rs` 条目是超集（补了 `execute` 契约与 auto-refine），但**漏掉了 g-005 的 history 段**，需手工补回；`run_tests` 描述同理。

#### 4.5 判据：**编译通过才算解对**

括号/花括号计数**不可靠**（字符串字面量里含 `{}`、`%{name}` 等）。
本会话直接以编译器为准：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
grep -rE '^(<<<<<<<|=======|>>>>>>>)' src/ locales/ AGENTS.md README.md README.zh-CN.md   # 必须为空
cargo build --release                                                                       # 编译器是最终裁判
```

> 特别注意 `src/tests.rs` 这类「循环体内插断言」的冲突：一侧的 `if` 可能**借走**公共的 `}`，
> 导致另一侧的断言被嵌进 `if` 里（编译通过但覆盖静默缩小）。解冲突时要看**花括号归属**，而非只求能编译。

### 步骤 5 · 集成分支上复跑全部门禁

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo build --release
./target/release/comma --test
```

本会话结果：四项全过，**389 passed, 0 failed**。提交合并：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
git add -A && git commit        # 完成 merge 提交
```

### 步骤 6 · 把集成分支版本改成 `X.Y.Z-alpha`（一眼区分测试构建）

**`Cargo.toml` 与 `Cargo.lock` 都要改**（后者是 `[[package]] name = "comma"` 条目）：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
# Cargo.toml:  version = "0.28.0-alpha"
# Cargo.lock:  name = "comma"  → version = "0.28.0-alpha"
cargo build --release
./target/release/comma --version      # 应打印 comma 0.28.0-alpha
cargo fmt --check && cargo clippy --release --all-targets -- -D warnings && ./target/release/comma --test
git add Cargo.toml Cargo.lock
git commit -m "chore: set integration version to 0.28.0-alpha (test build)"
```

### 步骤 7 · 打「已验」标签（⚠️ 本会话最大的坑）

> **`release.yml` 的触发规则是 `on.push.tags: 'v*'`。**
> **任何以字母 `v` 开头的 tag 被 push 都会自动走正式发布流程并建 Release。**

所以测试标签**绝不能以 `v` 开头**：

| tag 名 | 是否安全 | 说明 |
|---|---|---|
| `v0.28.0-test-verified` | ❌ **危险** | `v` 开头 → push 即意外发版（本会话最初误用，已删除） |
| `verified-0.28.0-alpha` | ❌ **危险** | `verified` 也以 `v` 开头 |
| `alpha-0.28.0-verified` | ✅ 安全 | 中途使用过 |
| **`0.28.0-alpha-verified`** | ✅ 安全 | **本会话最终采用**，且与「`<version>-verified`」规范一致 |

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
git tag -a 0.28.0-alpha-verified -m "已验：v0.28.0-alpha 集成分支测试构建

包含：g-011 #CHECK: ||| 修复 + g-005 可选 REPL 历史（默认关）+ g-012 命令后提示 + g-013 失败自动 refine（默认开）
门禁：fmt --check / clippy -D warnings / build --release 全干净；comma --test = 389 passed, 0 failed

命名说明：tag 不以字母 v 开头（release.yml 触发规则为 tags: 'v*'）。请勿 push 本 tag。"
```

**打完后自检**（必须做）：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
tag=0.28.0-alpha-verified
case "$tag" in v*) echo "危险：会触发发布流程";; *) echo "安全";; esac
git tag -l 'v0.28*'          # 不应出现任何测试标签
```

### 步骤 8 · 产出可跑二进制交给负责人体验

```bash
cd /home/miuzel/workspace/personal/comma-cli
cp .worktrees/v0.28.0-test/target/release/comma tmp/comma-0.28.0-alpha
ls -l tmp/comma-0.28.0-alpha
./tmp/comma-0.28.0-alpha --version      # comma 0.28.0-alpha
```

- `./tmp/` 已在 `.gitignore` 中（`/tmp/`），**临时产物与测试二进制一律放这里**，不要写系统临时目录。
- 交付话术要给全：**集成分支与 commit、已验 tag、二进制绝对路径、门禁结果、值得体验的要点、集成中超出机械合并的裁定、已知代价与退路**。

### 步骤 9 · 人工 gate 与收尾

- 负责人体验后给 **verdict**，通过才 `review→delivered`（**`review→delivered` 是人工 gate，主管/执行方都不得自行完成**）。
- 交付后清理**已合并**的 worktree（**保留分支**，零数据丢失）：

```bash
cd /home/miuzel/workspace/personal/comma-cli
git worktree remove .worktrees/g-003       # 重复至所有已合并 worktree
git worktree list                          # 确认只剩需要的
git branch                                 # 分支仍在（未删除）
```

> 本会话已清理 g-003 / g-006-ci / g-007 / g-008 / g-011-att-01；
> `g-005-att-01` / `g-012-att-01` / `g-013-att-01` / `v0.28.0-test` 因**尚未获得 verdict** 而保留。

---

## 3. 正式发布 SOP（v0.27.1 实际流程）

### 3.1 版本 bump 与提交

```bash
cd /home/miuzel/workspace/personal/comma-cli
# Cargo.toml:  version = "0.27.1"
# Cargo.lock:  name = "comma" → version = "0.27.1"
cargo build --release
./target/release/comma --version          # comma 0.27.1
./target/release/comma --test             # 全绿
git add Cargo.toml Cargo.lock             # 只暂存版本文件，别带上本地临时物
git commit -m "chore: release v0.27.1 (bump version)"
```

### 3.2 推 `main`（HTTPS + gh 凭据，ssh 在本机不可用）

```bash
cd /home/miuzel/workspace/personal/comma-cli
# 先看落后多少
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' ls-remote https://github.com/miuzel/comma-cli.git refs/heads/main
# 推送
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' push https://github.com/miuzel/comma-cli.git main
```

### 3.3 打并推正式 tag（触发 release.yml）

```bash
cd /home/miuzel/workspace/personal/comma-cli
git tag -a v0.27.1 -m "v0.27.1"
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' push https://github.com/miuzel/comma-cli.git v0.27.1
```

### 3.4 等待 Release workflow

```bash
gh run list --limit 5
gh run watch 32884995494 --exit-status      # 本会话 v0.27.1 的 run id；换成实际 id
```

`release.yml` 会为 **5 个平台**构建：`x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`（`cross`）、
`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-pc-windows-msvc`。
本会话结果：run `32884995494` **success**（2m52s）。

### 3.5 校验资产（应为 7 个）

```bash
gh release view v0.27.1
gh release view v0.27.1 --json assets --jq '.assets[].name'
```

期望 7 项：`comma-linux-x86_64.tar.gz`、`comma-linux-aarch64.tar.gz`、`comma-macos-x86_64.tar.gz`、
`comma-macos-aarch64.tar.gz`、`comma-windows-x86_64.zip`、`install.sh`、`sha256sums.txt`。

### 3.6 写真实 release notes（必做）

> `release.yml` 的 `generate_release_notes: true` **只产出一条 compare 链接**；
> 而 `, --update` / 自动更新会**把 release 正文展示给用户**再征求同意，所以正文必须首段就讲清「这个版本做了什么」。

```bash
cd /home/miuzel/workspace/personal/comma-cli
./scripts/release-notes.sh v0.27.1 --output ./tmp/rn-0.27.1.md
# 人工补 1–3 行「用户可见要点」到首段，compare 链接保持在最后
gh release edit v0.27.1 --repo miuzel/comma-cli --notes-file ./tmp/rn-0.27.1.md
```

- **必须用 `--notes-file`，不要用 `--notes` 内联**（引号/换行容易翻车）。
- 本会话 v0.27.1 的首段示例：*「Comma is now a 2.5MB binary (down from ~3.4MB)…」*。

### 3.7 更新 Homebrew tap

```bash
cd /home/miuzel/workspace/personal/comma-cli
./scripts/update-tap.sh v0.27.1 --tap-dir <homebrew-tap clone 路径> [--sums <本地 sha256sums.txt>]
```

脚本逻辑（`scripts/update-tap.sh`）：从 release 的 `sha256sums.txt`（**pin-tag URL**，规避 CDN 滞后）读取 4 个包的哈希，
重写 `Formula/comma-cli.rb` 的 **`version` 行** 与 **4 组 URL + sha256**（`comma-macos-aarch64`、`comma-macos-x86_64`、
`comma-linux-x86_64`、`comma-linux-aarch64`），然后打印可提交的 `git diff`。
它**刻意不碰 `.github/workflows/release.yml` 的产物命名**（归档名必须与 workflow 的 `archive:` 键保持一致）。

**tap 写盘需要提权**：tap clone 在 workspace 之外（`../homebrew-tap`），`workspace-write` 下只读，脚本的 `mktemp` 会报
`Read-only file system`。此**已被真实拒绝过**，可直接用 `sandbox_permissions: danger-full-access` 重试同一条命令
（是 workspace-write 之外唯一够用的模式），并在理由里写明「写 workspace 外的 tap 目录」。

**tap 的 remote 是 SSH，本机不可用**（见 4.1）——推送必须显式换成 HTTPS + `gh` 凭据助手：

```bash
# 1) 改公式（脚本自己会用 pin-tag URL curl 拉 sha256sums，本环境该 URL 可用）
./scripts/update-tap.sh v0.28.0 --tap-dir ../homebrew-tap
# 2) 逐项核对（version + 4 组 URL/sha256），并**独立抽验一个真实压缩包**：
#    curl -sSL <pin-tag 包 URL> -o tmp/v.tar.gz && sha256sum tmp/v.tar.gz   # 应与公式一致
# 3) 提交（沿用 tap 的既有信息风格）并推送
git -C ../homebrew-tap commit -am "comma-cli 0.28.0"
GIT_TERMINAL_PROMPT=0 git -C ../homebrew-tap -c credential.helper='!gh auth git-credential' \
  push https://github.com/miuzel/homebrew-tap.git main
```

> 本会话实测（v0.28.0）：tap 当时仍停在 **0.26.2**（说明 0.27.1 的公式更新从未被应用）→ 一步跨到 0.28.0；
> `update-tap.sh` 会自动处理任意跨版本。抽验了两个包（linux-x86_64、macos-x86_64）实算哈希均与公式一致。

### 3.8 CDN ~10 分钟滞后（必须写进交付说明）

> push tag 后 **等约 10 分钟**再测 `, --update` / `install.sh`：
> `releases/latest/download` 可能短暂把**上一版的 `sha256sums.txt`** 与**新压缩包**配对，导致**假校验失败**。
> 这是安全校验在正常工作，不是发布坏了。**pin-tag URL**（`releases/download/vX.Y.Z/...`）不受影响。

### 3.9 发布后检查清单

- [ ] `main` 已推送，远端 `refs/heads/main` = 本地 HEAD
- [ ] 正式 tag `vX.Y.Z` 已推送（远端可见）
- [ ] Release run 全绿（5 平台 + release job）
- [ ] 资产齐全：5 个平台包 + `install.sh` + `sha256sums.txt`
- [ ] **真实 release notes 已写入**（首段讲效果、compare 链接在末）
- [ ] Homebrew formula 已 bump（version + 4 组 URL/hash）**并已推送到 tap**，且**抽验过至少一个真实压缩包的 sha256**
      （写 tap 需要 `danger-full-access` 提权；推送用 HTTPS + `gh` 凭据助手，见 3.7）
- [ ] 交付说明里写了「CDN ~10 分钟后再测 `--update`/`install.sh`」

---

## 4. 环境坑（本会话真实遇到的，逐条给出解法）

### 4.1 `git` over ssh 在本机不可用

- **现象**：任何远端操作报
  `Bad owner or permissions on /etc/ssh/ssh_config.d/20-systemd-ssh-proxy.conf` +
  `fatal: Could not read from remote repository`。
- **根因**：该文件是符号链接且属主/权限异常 —— 实测 `777 nobody:nobody`
  （`stat -c '%a %U:%G %n' /etc/ssh/ssh_config.d/20-systemd-ssh-proxy.conf`），ssh 拒绝读取。
- **解法**：改用 **HTTPS + `gh` 凭据助手**（`gh` 本身工作正常）：

```bash
cd /home/miuzel/workspace/personal/comma-cli
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' push https://github.com/miuzel/comma-cli.git main
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' push https://github.com/miuzel/comma-cli.git v0.27.1
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' ls-remote https://github.com/miuzel/comma-cli.git refs/heads/main
```

### 4.2 沙箱 `workspace-write`：workspace 之外只读

- 只能写会话 workspace（`/home/miuzel/workspace/personal/comma-cli`）。
- `../homebrew-tap` 在 workspace **之外** → **只读**；`update-tap.sh` 的 `mktemp` 报 `Read-only file system`。
- **解法**：产出**更新后的 formula 文件**交负责人应用，或请负责人授予写权限。
  本会话的做法：把等价公式写到 workspace 内（如 `./tmp/comma-cli-0.27.1.rb`），
  并在交付说明中**明确标注「此步骤受阻于沙箱，需人工应用/授权」**。

```bash
cd /home/miuzel/workspace/personal/comma-cli
# 手工等价更新（version + 4 组 URL/hash，逐一对齐 release 的 sha256sums.txt）
# 产物放 ./tmp/，例如 ./tmp/comma-cli-0.27.1.rb
```

### 4.3 临时文件一律放 `./tmp/`

```bash
cd /home/miuzel/workspace/personal/comma-cli
mkdir -p ./tmp
grep -n '^/tmp/$' .gitignore || echo '/tmp/' >> .gitignore
```

本会话 `./tmp/` 内容示例：`comma-0.28.0-alpha`（测试二进制）、`comma-cli-0.27.1.rb`、`rn-0.27.1-draft.md`、`sha256sums.txt`。
`.gitignore` 现有内容：
```
/target
.worktrees/
.claude/worktrees/
.cargo/
**/.dsh-graph
.tmp-dsh
/tmp/
```

### 4.4 子代理**禁止**跑 `build.sh` / `install.sh`

- `build.sh` 会 release 构建并**安装到 `~/.local/bin`**，覆盖系统 PATH 下的 `comma`（开发版一旦被装上，后续排查全乱）。
- 需要冒烟测试就用全路径二进制：`/home/miuzel/workspace/personal/comma-cli/target/release/comma`（或 worktree 内的对应路径）。
- 同理 `install.sh` 也不要跑。

### 4.5 其它真实坑（同一批，务必记住）

| 坑 | 后果 | 对策 |
|---|---|---|
| **测试 tag 以 `v` 开头** | `release.yml` 的 `tags: 'v*'` 被触发 → **意外正式发版** | 测试 tag 绝不以 `v` 开头；push 前用 `case` 自检（见步骤 7） |
| **`--setup` 菜单抢索引** | 两卡都拿到 `Some(2)`，串到别的操作 | 重排索引并顺延后续分支（见 4.2 节） |
| **合并式保存不显式写键** | 用户选择被**静默丢弃** | 每个受管键都显式写入（`entries_to_json` + 调用点） |
| **locale 解冲突后缺键** | 运行时**静默回落英文**，`--test` 也不一定报 | 解完校验 9 文件键集与 `en.toml` 完全一致 |
| **判据不可判定** | 与代码现状矛盾，永远过不了 | 写判据前先核对代码（实例：g-005 原判据假设 Ctrl-D 是退出路径，实际 `ui.rs` 把 `Interrupted`/`Eof` 映射为 `None`、`main.rs` 是 `None => continue`，**Ctrl-D 并不退出**） |
| **只看断言 PASS 不看代码** | 声明与实现不符 | 逐行读 diff；断言要在旧代码上会 FAIL 才算真断言 |
| **`~/.cargo/registry` 只读** | 构建失败 | 在 worktree 内建可写 `CARGO_HOME`（`.cargo/`，已入 `.gitignore`） |
| **多卡改同一批文件却不用 worktree** | 互相踩提交、merge 地狱 | 重叠文件必须 worktree 隔离，冲突留到集成阶段按第 2 节解 |
| **只在本机（Linux）跑门禁** | **平台专属 lint 漏检** → `windows-latest` CI 红、发版被卡 | 集成后**必须**用 Windows 侧工具链预演一次 CI（见 4.6） |

### 4.6 Windows 侧预演 CI（本会话实测的解法，强烈建议纳入常规流程）

本机是 WSL，但 **PowerShell interop 可直接调用 Windows 侧工具链**，等价于在本地预演 `windows-latest` CI：

```bash
# Windows 侧已装 rustc/cargo 1.96.0 + clippy（0.1.96）；host triple = x86_64-pc-windows-msvc
powershell.exe -NoProfile -Command "cargo --version; rustc -vV | Select-String '^host'"

# Windows 能看到 WSL 树：\\wsl.localhost\<distro>\...
#   本会话 distro = archlinux（echo $WSL_DISTRO_NAME 可查）
# 做法：robocopy 到 Windows 本地临时目录再构建（**不要**直接在 \\wsl.localhost\ 下构建：
#   9p 慢，且 Windows 产物会污染 Linux 树）。脚本模板：./tmp/win-check.ps1 / win-ci.ps1 / win-clippy.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass \
  -File '\\wsl.localhost\archlinux\home\miuzel\workspace\personal\comma-cli\tmp\win-clippy.ps1'
```

脚本要点（`tmp/win-*.ps1`）：`robocopy <worktree> $env:TEMP\comma-win-* /E /XD target .git .dsh-graph .worktrees`
→ `Set-Location` 到 Windows 本地目录 → 跑 `cargo fmt --check` / `cargo clippy --release --all-targets -- -D warnings`
/ `cargo build --release` / `.\target\release\comma.exe --test`。

**平台专属断言数不同属正常**：本会话 Windows `comma.exe --test` = **482**，Linux = **508**
（差 26 条是 `cfg(windows)` 排除的 Unix-only 断言）。判据只看 `0 failed` 与退出码。

**本会话靠它抓到两个只在 Windows 出现的门禁失败**（Linux 侧全绿也照样会红 CI）：
1. `src/pty.rs` `unused_mut` —— `mut command` 只被 `#[cfg(unix)]` 路径用；修法 `#[cfg_attr(not(unix), allow(unused_mut))]` + 注释。
2. `src/history.rs` `clippy::needless_return` —— 早退 `return;` 之后的 `#[cfg(unix)]` 块在 Windows 被剔除，`return` 成了函数尾语句；修法是把「成功」表达成显式条件而非早退。

> 结论：**「Linux 门禁全绿」≠「CI 会绿」**。只要改动涉及 `#[cfg(...)]` 分支，就必须做 4.6 的预演。

### 4.7 macOS 也要预演（本机无法编译，但 CI 会教你怎么改）

v0.28.0 **首次发布时被 macOS 挡住**：Linux + Windows 全绿，但 `v0.28.0` 的 Release 两个 macOS 架构全在 `Build (native)` 失败（cargo exit 101），CI 的 `macos-latest` 也在 `clippy -D warnings` 失败。根因是**平台专属常量类型差异**：

```
error[E0308]: mismatched types
   --> src/pty.rs:501:42
501 |   if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) == -1 {
    |                  ^^^^^^^^^^^^^^^ expected `u64`, found `u32`
note: pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
```
`libc::ioctl` 的 `request` 在 BSD/macOS 是 `c_ulong`，但 **Apple 的 `TIOCSCTTY` 是 `c_uint`**（Linux 下是 `c_ulong`）→ 只在 macOS 编译失败。修法：`libc::TIOCSCTTY as libc::c_ulong`（Linux 上是同型 cast，故加**精确的** `#[allow(clippy::unnecessary_cast)]`）。

**能做的本机预演**：
1. **逐平台核对 `libc` 常量/函数的签名与类型**：直接从 `~/.cargo/registry/src/*/libc-*/src/unix/bsd/apple/` 与 `.../linux_like/` 对比你要用的符号（本会话正是这样定位到 `TIOCSCTTY` 与 `SA_RESTART`/`ptsname_r` 的平台差异）。注意 **`unix/bsd/` 下还有 freebsd/netbsd 子目录**，按名字 grep 容易误判 —— 只认 `unix/bsd/mod.rs`（BSD 公共）+ `unix/bsd/apple/**`。
2. 想真编译 macOS：需要 Apple SDK；**跨 target `cargo check --target x86_64-apple-darwin` 在本机行不通**（`ring` 的构建脚本找不到 C 编译器）。所以 **macOS 的真验证只能交给 CI** —— 因此平台专属改动必须先想清楚、再推。

### 4.8 本环境取不到 job 日志 / 发布资产时的取用路线

- **CI job 日志**：`gh run view --log[-failed]` 与 `gh api .../jobs/<id>/logs` 都失败 —— 它们会 302 到 `results-receiver.actions.githubusercontent.com` / `*.blob.core.windows.net`，本机 **connection refused**。
  **可达的替代**：**注解（annotations）走 api.github.com**，是可读的。要拿到编译器原文，可临时开一个**诊断分支**，在失败步骤里把输出塞进注解：
  ```yaml
  - name: clippy -D warnings
    shell: bash
    run: |
      set -o pipefail
      if ! cargo clippy --all-targets -- -D warnings 2>&1 | tee /tmp/c.log; then
        echo "::error title=clippy-error::$(tail -c 1600 /tmp/c.log | awk '{printf "%s%%0A", $0}')"
        exit 1
      fi
  ```
  推该分支 → CI 失败 → `gh api repos/<o>/<r>/check-runs/<id>/annotations` 读原文 → **立刻删除诊断分支**（本会话就是这么拿到 E0308 原文的）。
- **发布资产 / sha256sums**：`gh release download` 走 `release-assets.githubusercontent.com`（被拒）；但 **`curl` 打 `browser_download_url` 可用**：
  ```bash
  gh api repos/<o>/<r>/releases/tags/vX.Y.Z --jq '.assets[] | select(.name=="sha256sums.txt") | .browser_download_url'
  curl -sSL --max-time 60 -o ./tmp/sha256sums.txt "<that url>"
  ```
  ⚠️ 注意 `gh release download` 失败时**不会覆盖**已存在的旧文件 —— 本会话曾因此拿着**上一版的 sha256** 生成 Homebrew 公式（差点发错）。**生成公式后必须抽查一个真实压缩包**：`curl` 下载后 `sha256sum` 与公式里的值比对。

### 4.9 录演示 GIF（VHS）与本沙箱的通用坑（v0.29.0 g-019 实测）

演示流水线：`demo/*.tape`（VHS）→ 同名 `.gif`，由两个 README 的 `## Demo` 引用；复现步骤在 `demo/README.md`。

- **`/tmp` 是每次 bash 调用一份私有 tmpfs**：跨调用不保留。所以**温缓存与录制必须在同一个脚本里跑完**（否则录到空/未预热状态）。同理，任何"先建临时状态、下一条命令再用"的流程在此沙箱都不可靠 —— 一律写成一个脚本。
- **Go 的网络栈绕过 proxychains**（只有 libc 客户端如 curl/git/python 能出网）→ `go install <x>@latest` 报 `lookup proxy.golang.org: no such host`。**替代：直接下 GitHub release 的静态二进制**（`curl` 可用）解压到 `./tmp/bin`，免提权。这也解释了为什么本环境里 `curl` 打 github.com 常常比 `gh` 好用。
- **vhs 0.12.0 有上游 bug**：`evaluator.go` 里 `ctx` 被 `context.WithCancel` 遮蔽，`teardown()` 先 cancel，`Render(ctx)` 拿到已取消的 ctx → ffmpeg 永不启动，**打印 `Creating …gif…` 后静默不写文件且 exit 0**（最恶心的失败模式：看起来成功）。**pin vhs 0.11.0**。
- **无头浏览器**：VHS/rod 先找 `google-chrome`；WSL 下 `/usr/local/bin/google-chrome` 常是指向 Windows `chrome.exe` 的软链 → 用 `./tmp/bin/google-chrome` wrapper `exec /usr/bin/chromium`，并设 `VHS_NO_SANDBOX=1`。
- **录的必须是新构建**：tapes 里敲的是 `,`，所以要 `ln -sf "$PWD/target/release/comma" ./tmp/bin/,` 并把 `./tmp/bin` 前置到 `PATH`，录前先 `, --version` 确认版本号。
- **确定性靠温缓存**：cache key 含工作目录 → 必须在 tape 使用的同一目录里预热；演示沙箱 `HOME` 用 `/tmp/comma-demo-home`（复制真实 config **并删掉 `lang`**，否则真实配置的 `lang` 会压过 `COMMA_LANG`）。**绝不把含 API key 的 config 提交进仓库**。
- **想看某帧**：`ffmpeg -v error -i demo/x.gif -vf "select=eq(n\,470)" -vframes 1 out.png -y`，然后直接看图核验（比读 PASS 文本可靠）。核验要点：模型名不是过期的、有 welcome/历史提示行、菜单无「提示行 + 按 x」残留。

---

## 5. 与 dsh-graph 看板的配合

### 5.1 状态流转

```
draft → planning → collecting → ready → in_progress → review → delivered
                                                 ↘ blocked ↗（必须给 reason）
```

- **判据先于执行**：`ready→in_progress` 前判据必须已登记确认（引擎强制）。
- `review→delivered` 是**人工 gate**：负责人给 verdict 才可迁移；主管与执行方都不得自行完成。
- 执行派发用 `graph_start_attempt`：**自动建 worktree、自动落 `in_progress`、自动绑子代理**。
- 执行子代理自己维护 lane 与 status（`graph_report_status`，每动作一句 ≤20 字）；主管**不代报**。

### 5.2 六个数据面相互独立

- 看板数据在 `/home/miuzel/workspace/personal/comma-cli/.dsh-graph/`，与代码分支**相互独立**。
- **集成与发布都不重置看板数据**；合并代码时别用会重置 `.dsh-graph` 的操作。
- 派发时若发现子代理 cwd 落在包目录，可能误建 `.dsh-graph` 骨架 → 及时纠正，统一在仓库根读写看板。

---

## 6. 检查清单（可直接勾选）

### 6.1 集成前（每张卡）

- [ ] 子代理在**独立 worktree**、工作树干净、commit 已记录
- [ ] `--stat` / `--name-only` 与声明的改动范围一致（无越界改写）
- [ ] 关键实现**逐行读过**；隐私/安全红线有**真断言**
- [ ] 我**自己复跑**四连门禁，且断言数 = 基线 + 新增
- [ ] 复核结论写入该 goal 的评论

### 6.2 集成后

- [ ] 无残留冲突标记（`grep -rE '^(<<<<<<<|=======|>>>>>>>)'` 为空）
- [ ] 9 个 locale 键集合与 `en.toml` **完全一致**，`tomllib` 可解析
- [ ] `--setup` 菜单索引已重排且**后续分支已顺延**
- [ ] 重构卡的新行为已补回（如 `print_cmd_hint()`）
- [ ] 散文文件双方内容都在（超集方漏掉的部分已补）
- [ ] 集成分支上四连门禁全绿，**通过数已记录**（本会话 389）

### 6.3 打标签前

- [ ] `Cargo.toml` **与** `Cargo.lock` 版本均为 `X.Y.Z-alpha`
- [ ] `comma --version` 输出 `X.Y.Z-alpha`
- [ ] 改版本后**重新 build 并复跑门禁**
- [ ] 测试 tag 名**不以 `v` 开头**（`case "$tag" in v*)` 自检通过）
- [ ] `git tag -l 'vX.Y*'` 无测试标签
- [ ] tag message 含「包含哪些卡 + 门禁结果 + 请勿 push」
- [ ] 测试二进制已放到 `./tmp/` 并给出绝对路径

### 6.4 发布前

- [ ] 集成分支内容已确认全部落到 `main`（或 main 已含全部交付）
- [ ] 负责人已给 verdict（`review→delivered`）
- [ ] `Cargo.toml` / `Cargo.lock` 已 bump 到**正式号**并单独提交
- [ ] 本地 `main` 领先远端的提交数已确认
- [ ] 发布授权已取得（发布是人工 gate）

### 6.5 发布后

见 §3.9。

---

## 7. 回滚与补救

### 7.1 集成分支出问题

集成分支是**内部场**，可自由重置重建：

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
git log --oneline -5                       # 找到要回退到的点
git reset --hard <commit>                  # 只在集成分支上做
# 或彻底重建
cd /home/miuzel/workspace/personal/comma-cli
git worktree remove .worktrees/v0.28.0-test
git branch -D v0.28.0-test && git branch v0.28.0-test main
git worktree add .worktrees/v0.28.0-test v0.28.0-test
```

### 7.2 「已验」标签打错/要改名

```bash
cd /home/miuzel/workspace/personal/comma-cli/.worktrees/v0.28.0-test
git tag -d 0.28.0-alpha-verified                       # 删本地标签
git tag -a 0.28.0-alpha-verified -m "..."              # 重打
# 自检不以 v 开头
case 0.28.0-alpha-verified in v*) echo 危险;; *) echo 安全;; esac
```

改一行代码后重出构建：回到「步骤 6 → 7 → 8」重跑即可（集成分支上不需要重建整个分支）。

### 7.3 误推了 `v*` 测试标签（**会触发正式发版**）

补救顺序：**先删远端 tag（止损）→ 再处理已生成的 Release**。

```bash
# 1) 立刻删远端 tag（阻止/结束以该 tag 为名的发布语义）
GIT_TERMINAL_PROMPT=0 git -c credential.helper='!gh auth git-credential' \
  push https://github.com/miuzel/comma-cli.git --delete <误推的tag名>

# 2) 查看是否已生成 Release
gh release list --limit 5
gh release view <误推的tag名>

# 3) 若已生成：删除该 Release（或改回 draft 并修正 notes）
gh release delete <误推的tag名> --yes
#    或：gh release edit <误推的tag名> --draft

# 4) 删本地标签，避免再次误推
git tag -d <误推的tag名>
```

补充要点：
- 若 workflow 仍在跑，可在 Actions 页面 **Cancel**；删 tag 不会撤销已上传的资产，需按上面第 3 步处理 Release。
- 发布后短时间内 `releases/latest` 可能已被指向该误发版本 → 修正后**等 CDN 冷却（~10 分钟）**再验证。
- **预防**：打任何测试标签前执行 §6.3 的自检；**绝不使用 `git push --tags`**（会一次性推送全部本地标签，包括测试标签）。

### 7.4 发布后发现严重缺陷

- 走一次**新版本**（`vX.Y.(Z+1)`）修复并发布；不要改写已发布的 `vX.Y.Z` 与其资产（用户可能已按 `sha256sums.txt` 校验安装）。
- 若必须说明，用 release notes 追加「已知问题 + 修复版本号」，而不是删除原 release。

---

## 附：本会话关键事实速查

| 项 | 值 |
|---|---|
| 集成分支 | `v0.28.0-test` @ `d5d53b0`（版本 `0.28.0-alpha`） |
| 已验标签 | `0.28.0-alpha-verified`（**不以 v 开头**） |
| 集成分支基线 | `6fb6c5f`（含 g-011 的 `#CHECK:` `|||` 修复） |
| 集成卡 | g-005（可选 REPL 历史，默认关）、g-012（命令后提示）、g-013（失败自动 refine，默认开） |
| 冲突数 | 15（9 locale + 2 双语 README + AGENTS.md + `main.rs` + `setup.rs` + `tests.rs`） |
| 门禁结果 | 集成分支 **389 passed, 0 failed**；二进制 `./tmp/comma-0.28.0-alpha` |
| v0.27.1 发布 | run `32884995494` success；7 资产；notes 用 `gh release edit --notes-file` 写入 |
| `main` 状态 | 本地 `6fb6c5f`（g-011 修复**未推**）；远端 `refs/heads/main` = `978545c`；远端已有 `v0.27.1` |
| 未决项 | ~~Homebrew tap 公式受阻于沙箱只读~~ → **v0.28.0 已解决**：提权 `danger-full-access` 写入 + HTTPS 推送 tap（`comma-cli 0.28.0`，commit `0b25550`），并抽验 linux-x86_64 / macos-x86_64 两个真实压缩包哈希一致 |
