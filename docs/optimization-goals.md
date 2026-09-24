# comma-cli 优化目标卡片

> 小型工具（~5.8k 行 Rust、单二进制）的优化清单。每张卡片：目标、影响、工作量、验收标准。
> 状态：⬜ 待办 · 🔄 进行中 · ✅ 完成

---

## 🟥 P0-1 · 消除全部 clippy 警告（当前 20 条）

- **类别**：代码质量
- **影响**：中 — 消除潜在 bug 信号与坏味道，14 条可一键自动修复
- **工作量**：小（`cargo clippy --fix` + 手动修 6 条）
- **现状**（`cargo clippy --release --all-targets` 实测）：
  - `manual_contains` ×5（main.rs 里 `flags.iter().any(|a| *a == "--test")` 等 → `flags.contains(&"--test")`）
  - `let_and_return`（main.rs:580）
  - `map_or` 可简化（context.rs:75）
  - `too_many_arguments` ×2
  - `&mut Vec` → `&mut [_]` ×2
  - `div_ceil` 手写重复、`saturating_sub` 隐式、冗余闭包、无用 `format!`、多余 `return` ×2、doc 列表缩进 ×2
- **验收标准**：`cargo clippy --release --all-targets` 输出 **0 warnings**；`./target/release/comma --test` 仍 222 全绿；`--update`/搜索等路径无行为变化

---

## 🟥 P0-2 · 用 `std::io::IsTerminal` 替换已弃用的 `atty` 依赖

- **类别**：依赖 / 现代化
- **影响**：中 — `atty 0.2` 已归档（RUSTSEC-2021-0145，musl 上有未定义行为），且少一个依赖、体积略减
- **工作量**：小（9 处调用点，机械替换）
- **现状**：`src/{main,ui,setup,update}.rs` 共 9 处 `atty::is(atty::Stream::Stdin/Stdout)`；rustc 1.70+ 自带 `std::io::IsTerminal`
- **注意**：`update.rs:350` 同时检查 stdin+stdout，替换时保持语义一致
- **验收标准**：`Cargo.toml` 移除 atty；`cargo build --release` 通过；TTY / 管道 / `echo x | , !` 行为与之前一致（`--test` + 手动管道冒烟）

---

## 🟥 P0-3 · 交互式历史记录跨会话持久化

- **类别**：UX
- **影响**：高 — 当前 `add_history_entry` 只写内存（ui.rs:554/569/596），**每次启动历史全丢**；`↑` 翻不到上次输入是日常体验痛点
- **工作量**：小 — rustyline 内置 `load_history`/`save_history`，只需选一个落盘路径（如 `$XDG_STATE_HOME/comma/history` 或 `~/.local/state/comma/history`，Windows 用 `%APPDATA%\comma\history`）
- **注意**：历史含用户意图文本，落盘位置应遵守既有 xdg_or_legacy 解析习惯；REPL 会话退出时保存，`q`/Ctrl-D 路径都要覆盖
- **验收标准**：交互模式输入几条命令后退出重启，`↑` 能翻到上次输入；`--test` 通过

---

## 🟥 P1-4 · 添加 CI workflow（当前完全没有 CI）

- **类别**：工程化 / 可靠性
- **影响**：高 — 仓库只有 `release.yml`，无任何 PR/push 检查；`--test` 222 条断言只在本地跑
- **工作量**：小 — 新增 `.github/workflows/ci.yml`
- **内容**：`cargo fmt --check` → `cargo clippy --all-targets -D warnings` → `cargo build --release` → 运行 `./target/release/comma --test`（矩阵：ubuntu/macos/windows）
- **验收标准**：push 后 Actions 全绿；故意引入一条 clippy 警告时 CI 红

---

## 🟡 P2-5 · 二进制继续瘦身（当前 3.4MB，README 宣称 3MB）

- **类别**：性能 / 体积
- **影响**：低-中 — 安装体积与启动感知；已开 `strip` + `lto` + `opt-level="z"`
- **工作量**：小（profile 微调）/ 大（换 HTTP 客户端，可选探索）
- **可做**：`[profile.release]` 加 `panic = "abort"`、`codegen-units = 1`
- **探索项（不承诺）**：`reqwest`（rustls，依赖树大）→ `ureq`/`minreq` 可显著减体积，但重写 `llm.rs`/`search.rs`/`update.rs` 的网络层，改动大、收益需实测
- **验收标准**：strip 后体积下降 ≥10% 且 `--test`、真实 API 调用冒烟通过

---

## 🟡 P2-6 · Rust edition 2021 → 2024 + 引入 rustfmt 规范

- **类别**：现代化
- **影响**：低 — 无用户可见变化，属工具链卫生
- **工作量**：小 — 改 `Cargo.toml` edition 并处理编译差异（需 rustc ≥ 1.85）；新增 `rustfmt.toml` 并把 `cargo fmt --check` 并入 CI 卡
- **注意**：先跑 `cargo fmt` 全仓格式化，避免 2024 edition 与未格式化代码混在一起
- **验收标准**：`cargo build --release` + `--test` 通过，无行为变化

---

## 🟡 P2-7 · 发布流程半自动化（Homebrew formula + release notes 草稿）

- **类别**：工程化 / 运维
- **影响**：中 — AGENTS.md 明确记录了每次发布要**手动**更新 `miuzel/homebrew-tap` 的 4 个 URL + sha256，且 release notes 靠手写；这是最容易出错的重复劳动
- **工作量**：中 — 新增 `scripts/update-tap.sh`：从 `sha256sums.txt` 读哈希、替换 formula 中 version/URL/hash 四组；另加 `scripts/release-notes.sh` 从 git log 生成 notes 草稿文件（供 `gh release edit --notes-file` 使用）
- **注意**：保持与 AGENTS.md「wait ~10 min for CDN」一致的文档提醒；不要碰 release.yml 的产物命名
- **验收标准**：发布 vX.Y.Z 后跑一个脚本即可产出可提交的 formula diff；notes 草稿含功能要点 + compare 链接

---

## 🟢 P3-8 · 小项清理（攒够一起做）

- **类别**：代码卫生
- **影响**：低
- **候选**：
  - `process::exit` 在 7 处直接退出、跳过析构——可统一为 `fn main() -> Result` 风格（改动面大，收益小，排最后）
  - `cache.rs` 每次 `save()` 全量重写 JSON——`cache_size` 默认值小则无所谓，可加「仅在有新条目且超阈值时写」的微优化
  - `cache_key` 用固定种子的 `DefaultHasher`（SipHash）——本地缓存文件非安全边界，可留注释说明即可
- **验收标准**：每个子项有独立小 commit，`--test` 保持全绿

---

## 建议执行顺序

1. **P0-1 + P0-2 一起做**（同一轮 `cargo clippy --fix` + 依赖替换，风险最小、见效最快）
2. **P0-3 历史持久化**（纯增量功能，独立小 PR）
3. **P1-4 CI**（把以上成果固化，防止回归）
4. P2 系列按需挑选；P3 攒批清理

> 数据来源：`cargo clippy --release --all-targets`（20 警告）、`wc -l src/*.rs`（5846 行）、`ls -lh target/release/comma`（3.4MB）、`grep` 全仓审计（atty 9 处 / history 无持久化 / process::exit 7 处 / reqwest 超时已配 60s+10s）。commit `dfff4f9`（v0.26.2）基线。
