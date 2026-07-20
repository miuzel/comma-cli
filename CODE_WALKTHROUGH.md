# comma-cli Rust 代码走读文档

> 逐行走读 comma-cli 的 Rust 实现，讲解语言特性与工程写法。已知知识点不重复。

---

## 目录

1. [Cargo.toml — 项目配置](#1-cargotoml--项目配置)
2. [main.rs — 程序入口与主循环](#2-mainrs--程序入口与主循环)
3. [config.rs — 配置加载](#3-configrs--配置加载)
4. [llm.rs — LLM 调用](#4-llmrs--llm-调用)
5. [ui.rs — 终端交互](#5-urs--终端交互)
6. [cache.rs — 响应缓存](#6-cachers--响应缓存)
7. [protocol.rs — 协议处理 (#CHECK/#EXPLORE)](#7-protocolrs--协议处理)
8. [context.rs — 系统上下文](#8-contextrs--系统上下文)
9. [prompt.rs — 提示词模板](#9-promptrs--提示词模板)
10. [danger.rs — 危险命令检测](#10-dangerrs--危险命令检测)
11. [update.rs — 自动更新](#11-updaters--自动更新)
12. [tests.rs — 内置测试](#12-testsrs--内置测试)

---

## 1. Cargo.toml — 项目配置

```toml
[package]
name = "comma"
version = "0.17.5"
edition = "2021"
```

- `edition = "2021"`：Rust 版本代际。2021 edition 引入了更智能的闭包捕获、`IntoIterator for arrays` 等特性。

### 依赖声明

```toml
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
```

- **`default-features = false`**：禁用默认特性，按需开启。这是 Rust 的特性（feature）系统——一个 crate 可以编译出不同功能组合的版本。
- **`blocking`**：reqwest 默认是异步的（基于 tokio），开启 `blocking` 后提供同步 API。
- **`rustls-tls`**：使用纯 Rust 的 TLS 实现，而非系统 OpenSSL。减少跨平台编译问题。

```toml
serde = { version = "1", features = ["derive"] }
```

- **`derive` feature**：启用 `#[derive(Serialize, Deserialize)]` 宏，自动为结构体生成序列化/反序列化代码。

### Release profile

```toml
[profile.release]
strip = true      # 剥离调试符号
opt-level = "z"   # 优化体积（而非速度）
lto = true        # Link-Time Optimization，跨 crate 内联
```

- `opt-level = "z"` 比 `opt-level = "s"` 更激进地压缩体积。
- `lto = true` 在链接阶段做全局优化，编译更慢但产物更小更快。

---

## 2. main.rs — 程序入口与主循环

### 模块声明

```rust
mod cache;
mod config;
mod context;
// ...
```

- `mod` 声明子模块。Rust 的模块系统：每个 `.rs` 文件是一个模块，`mod foo;` 查找 `foo.rs` 或 `foo/mod.rs`。

### 参数解析（手写，无 clap）

```rust
let args: Vec<String> = std::env::args().skip(1).collect();
```

- `std::env::args()` 返回迭代器，`skip(1)` 跳过程序名。
- `.collect()` 消费迭代器收集为 `Vec<String>`——类型推断由左侧 `Vec<String>` 决定。

```rust
let mut flags: Vec<&str> = Vec::new();
let mut rest: &[String] = &[];
```

- `&[String]`：对 `Vec<String>` 的切片引用（slice reference）。零开销的视图，不拥有数据。
- 初始值 `&[]` 是空切片。

```rust
for (i, a) in args.iter().enumerate() {
```

- `.enumerate()` 产生 `(index, &item)` 元组。Rust 中迭代器链是惰性的，`enumerate` 是适配器（adapter）。

```rust
let is_flag = matches!(s, "-h" | "--help" | "-V" | "--version" | "--update" | "--test" | "-f")
    || (s.starts_with("-v") && s.chars().skip(1).all(|c| c == 'v'));
```

- **`matches!` 宏**：模式匹配的快捷写法，返回 `bool`。等价于 `match s { "-h" | "--help" | ... => true, _ => false }`。
- **闭包 `|c| c == 'v'`**：`.all()` 接受 `FnMut(char) -> bool`，闭包是 Rust 的一等公民。
- 这段逻辑同时处理 `-v`、`-vv`、`-vvv` 等叠加 verbose 标志。

```rust
if s == "--" {
    rest = &args[i + 1..];
    break;
}
```

- `&args[i + 1..]`：切片语法。`..` 是 range，`i+1..` 从 i+1 到末尾。这是借用（borrow），不移动数据。

### 版本号内省

```rust
println!("comma {}", env!("CARGO_PKG_VERSION"));
```

- **`env!` 宏**：编译时读取环境变量。`CARGO_PKG_VERSION` 由 Cargo 自动设置，等于 `Cargo.toml` 中的 `version`。编译时常量，零运行时开销。

### Verbosity 解析

```rust
let verbosity = Verbosity(
    flags
        .iter()
        .filter(|a| a.starts_with("-v") && a.chars().skip(1).all(|c| c == 'v'))
        .map(|a| a.len() as u8 - 1)
        .sum(),
);
```

- 迭代器链：`filter` → `map` → `sum`。经典的函数式管道。
- `.map(|a| a.len() as u8 - 1)`：`-v` 长度 2 → 1 个 v，`-vv` 长度 3 → 2 个 v。
- `Verbosity(u8)` 是 newtype pattern——用元组结构体包装基础类型，增加类型安全性。

### match 错误处理

```rust
let config = match load_config() {
    Ok(c) => c,
    Err(e) => {
        print_error(&format!("Config: {}", e));
        std::process::exit(1);
    }
};
```

- `Result<T, E>` 的 `match` 解包。Rust 没有异常，错误通过 `Result` 传播。
- `format!` 宏：类似 `println!` 但返回 `String` 而非打印。

### 流程分支

```rust
if rest.is_empty() {
    if !atty::is(atty::Stream::Stdin) {
        // 管道输入
        match read_stdin_intent() {
            Some(intent) => run_oneshot(...),
            None => return,
        }
    } else {
        run_interactive(...);
    }
} else if rest.len() == 1 && rest[0] == "!" && !atty::is(atty::Stream::Stdin) {
    // 自动确认模式
    ...
} else {
    // 普通单次模式
    ...
}
```

- `atty::is(atty::Stream::Stdin)`：检测 stdin 是否为 TTY（交互式终端）。管道重定向时返回 `false`。
- 这里体现了 Unix 哲学：同一工具既可以交互使用，也可以管道组合。

### `Option` 的 `?` 操作符

```rust
fn read_stdin_intent() -> Option<String> {
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok()?;
    let intent = input.trim();
    if intent.is_empty() {
        None
    } else {
        Some(intent.to_string())
    }
}
```

- `.ok()?`：`Result` → `Option` 的转换后跟 `?` 提前返回。如果 `read_line` 失败，整个函数返回 `None`。
- `?` 是 Rust 的错误传播语法糖：成功则解包，失败则提前返回错误/None。

### `run_oneshot` — 单次执行模式

```rust
fn run_oneshot(config: &Config, system: &str, intent: &str, v: Verbosity, auto_confirm: bool, force_refresh: bool) {
```

- 参数全是引用：`&Config`、`&str`——借用，不转移所有权。这是 Rust 的核心设计。

```rust
let mut messages = vec![Message {
    role: "user".into(),
    content: intent.to_string(),
}];
```

- **`vec!` 宏**：创建 `Vec` 的字面量语法。
- **`.into()`**：`From`/`Into` trait 的转换。`"user".into()` 将 `&str` 转为 `String`。
- **`.to_string()`**：`ToString` trait 的方法，效果同上但来源不同（`to_string` 基于 `Display`，`into` 基于 `From`）。

```rust
let ph = collect_placeholders();
let mut cache = ResponseCache::load(config.cache_size);
```

- `collect_placeholders()` 返回拥有所有权的 `Placeholders` 结构体。`ph` 拥有它。

```rust
let mut rl = Editor::<FileHelper, DefaultHistory>::new().ok();
```

- **泛型参数** `Editor::<FileHelper, DefaultHistory>`：turbofish 语法 `::<>` 指定泛型参数。
- `.ok()`：`Result` → `Option`，丢弃错误信息。这里因为编辑器初始化失败不是致命错误。

### loop + match 控制流

```rust
loop {
    let candidates: Vec<String> = parse_candidates(&current_raw)
        .into_iter()
        .map(|c| apply_placeholders(&c, &ph))
        .collect();

    let cmd = if candidates.len() > 1 {
        if auto_confirm {
            candidates[0].clone()
        } else {
            match select_command(&candidates) {
                Some(i) => candidates[i].clone(),
                None => break,
            }
        }
    } else {
        candidates[0].clone()
    };

    // ...

    match action {
        EditAction::Execute(final_cmd) => {
            execute(&final_cmd);
            break;
        }
        EditAction::Refine(text) => {
            // 追加消息，重新调用 LLM
            messages.push(Message { role: "assistant".into(), content: current_raw.clone() });
            messages.push(Message { role: "user".into(), content: text });
            // ...
        }
        EditAction::Cancel => break,
    }
}
```

- `loop {}` 是 Rust 的无限循环，通过 `break` 退出。可以返回值（`break value`）。
- `if` 是表达式，可以赋值。Rust 中几乎一切都是表达式。
- `.clone()`：显式深拷贝。Rust 的所有权系统要求明确：要么转移（move），要么借用（&），要么克隆（clone）。

### `run_interactive` — 交互模式

```rust
let mut messages: Vec<Message> = Vec::new();
let mut current_cmd = String::new();
let mut current_raw = String::new();
let mut current_cache_key: Option<String> = None;
let mut current_cache_entry: Option<CacheEntry> = None;
```

- `Option<String>`：可选值。Rust 没有 null，用 `Option<T>`（`Some(T)` | `None`）表达"可能不存在"。

```rust
match input {
    None => continue,
    Some(input) => {
        if input == "q" || input == "quit" || input == "exit" {
            break;
        }
        // ...
    }
}
```

- `Option` 的 `match` 解构。`None` 分支 `continue` 跳过本次循环迭代。

### `execute` 函数

```rust
pub(crate) fn execute(cmd: &str) {
```

- `pub(crate)`：可见性限定为当前 crate（包）内可见。Rust 的默认是私有，需要显式声明公开范围。

```rust
if let Ok(path) = std::env::var("COMMA_EVAL_FILE") {
    if !path.is_empty() {
        use std::io::Write;
        // ...
        return;
    }
}
```

- **`if let`**：模式匹配的简写。只关心一个模式时比 `match` 简洁。
- **函数内 `use`**：局部导入，限定作用域。

```rust
let (prog, args) = shell_interp();
let status = std::process::Command::new(prog)
    .args(args)
    .arg(command)
    .status();
```

- **Builder 模式**：`Command::new().args().arg().status()` 链式调用配置命令。
- `.status()` 执行命令并等待完成，返回 `Result<ExitStatus>`。

```rust
match status {
    Ok(s) if !s.success() => {
        print_error(&format!("Exit code: {}", s.code().unwrap_or(-1)));
    }
    Err(e) => print_error(&format!("Failed to execute: {}", e)),
    _ => {}
}
```

- **match guard**：`if !s.success()` 是模式守卫，在模式匹配后附加额外条件。
- `_ => {}`：通配模式，捕获所有未匹配的情况。`{}` 是空表达式（unit `()`）。

---

## 3. config.rs — 配置加载

### 枚举与 derive

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    OpenAI,
    Anthropic,
}
```

- **`#[derive(...)]`**：自动实现 trait。`Debug` 调试打印，`Clone` 克隆，`Copy` 隐式复制（仅适用于栈上小类型），`PartialEq`/`Eq` 相等比较。
- C-like 枚举，无数据载荷。

### impl 块与 `Self`

```rust
impl ApiStyle {
    fn from_url(url: &str) -> Self {
        let lower = url.to_lowercase();
        if lower.contains("anthropic") {
            ApiStyle::Anthropic
        } else {
            ApiStyle::OpenAI
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" | "open_ai" | "oai" => Some(ApiStyle::OpenAI),
            "anthropic" | "claude" => Some(ApiStyle::Anthropic),
            _ => None,
        }
    }
}
```

- `Self`：当前实现类型的别名（此处即 `ApiStyle`）。
- `Option<Self>`：返回可能失败的构造结果。比 `panic!` 优雅。

### Serde 反序列化

```rust
#[derive(Deserialize, Default)]
struct ProviderConfig {
    base_url: Option<String>,
    auth_token: Option<String>,
    api_style: Option<String>,
}
```

- `#[derive(Deserialize)]`：自动生成从 JSON 等格式反序列化的代码。
- `Default`：自动生成默认值实现（所有 `Option` 字段为 `None`）。
- 所有字段 `Option<T>`：JSON 中可缺失。

### `#[serde(rename = "...")]`

```rust
#[derive(Deserialize)]
struct ClaudeEnv {
    #[serde(rename = "ANTHROPIC_BASE_URL")]
    base_url: Option<String>,
    // ...
}
```

- `rename`：JSON 中的 key 与 Rust 字段名不同时，用 `rename` 映射。

### 嵌套结构体

```rust
#[derive(Deserialize, Default)]
struct LocalConfig {
    base_url: Option<String>,              // 旧版单模型格式
    // ...
    providers: Option<HashMap<String, ProviderConfig>>,  // 新版多 provider
    models: Option<Vec<LocalModelEntry>>,
    // ...
}
```

- `HashMap<String, ProviderConfig>`：标准库的哈希映射。key 是 provider 名，value 是配置。
- 两种配置格式（旧单模型/新多 provider）共存于同一结构，用 `Option` 区分。

### 函数式闭包与优先级链

```rust
let non_empty = |o: Option<String>| o.filter(|s| !s.is_empty());
let env_or = |key: &str| non_empty(std::env::var(key).ok());
```

- **闭包赋值**：闭包可以绑定到变量。`|参数| 表达式`。
- `.filter()`：`Option` 的方法，条件为假时将 `Some` 变为 `None`。
- `env_or` 组合了 `non_empty`，形成可复用的"环境变量读取 → 空值过滤"管道。

```rust
let base_url = env_or("COMMA_BASE_URL")
    .or_else(|| non_empty(local.base_url.clone()))
    .or_else(|| env_or("ANTHROPIC_BASE_URL"))
    .or_else(|| claude_env.as_ref().and_then(|e| e.base_url.clone()))
    .unwrap_or_else(|| "https://api.anthropic.com".into());
```

- **`.or_else(|| ...)`**：惰性 fallback。前一个为 `None` 时才计算闭包。
- **`.and_then(|e| ...)`**：`Option` 的 flatmap——如果 `Some`，对内部值应用函数，函数本身也返回 `Option`。
- **`.unwrap_or_else(|| ...)`**：最终默认值，惰性计算。
- 这整段是配置优先级链：环境变量 > 本地配置 > Claude 配置 > 硬编码默认值。

### `Vec<ModelEntry>` 构建

```rust
let entries = if let Some(models) = local.models {
    // 新版多 provider 格式
    let providers = local.providers.unwrap_or_default();
    let mut entries = Vec::new();
    for (i, m) in models.iter().enumerate() {
        // ...
        entries.push(ModelEntry { ... });
    }
    if entries.is_empty() {
        return Err("models list is empty".into());
    }
    entries
} else {
    // 旧版单模型格式
    vec![ModelEntry { ... }]
};
```

- `if let` 用于解构 `Option`。
- `.unwrap_or_default()`：`None` 时使用 `Default` trait 的默认值（`HashMap` 默认为空 map）。
- `return Err("...".into())`：提前返回错误。`.into()` 将 `&str` 转为 `String`。
- `vec![...]` 与 `if`/`else` 都是表达式，最后一个表达式的值作为块的返回值。

---

## 4. llm.rs — LLM 调用

### 序列化/反序列化结构体

```rust
#[derive(Serialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Default, Debug)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    // ...
}
```

- `Serialize` 用于发送 JSON 请求，`Deserialize` 用于解析 JSON 响应。同一类型可以同时 derive 两者。
- `#[derive(Default)]` 给 `Usage` 零值初始化能力。

### Anthropic 特有的 serde 属性

```rust
#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
}
```

- **`skip_serializing_if = "Option::is_none"`**：当字段为 `None` 时，JSON 中完全省略该 key（而非输出 `null`）。这是 API 兼容性要求。

```rust
#[derive(Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    thinking_type: String,
    budget_tokens: u32,
}
```

- `type` 是 Rust 关键字，不能用作字段名。`rename` 让 JSON 中输出 `"type"` 而 Rust 内部用 `thinking_type`。

### 嵌套解构链

```rust
let content = choices
    .first()
    .and_then(|c| c.message.as_ref())
    .and_then(|m| m.content.as_deref())
    .unwrap_or("")
    .trim();
```

- `.first()` 返回 `Option<&T>`（数组可能为空）。
- `.and_then()` 链式处理多层嵌套的 `Option`。
- `.as_deref()`：`Option<String>` → `Option<&str>` 的协变转换（自动 deref）。
- 整条链是"空安全导航"——任何一环为 `None` 就短路返回默认值。

### 计时与耗时

```rust
let t0 = std::time::Instant::now();
let resp = client.post(&url)...send()...;
let elapsed = t0.elapsed();
```

- `Instant::now()` 取高精度时间戳，`.elapsed()` 返回 `Duration`。
- `as_secs_f64()` 转浮点秒，`as_millis() as u64` 转毫秒整数。

### HTTP 客户端构建

```rust
pub fn make_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client: {}", e))
}
```

- `.map_err()`：转换错误类型。reqwest 的 `Error` → 自定义 `String` 错误。
- Builder 模式：配置完成后 `.build()` 创建实例。

### `call_llm_with_retry` — 重试与 fallback

```rust
pub fn call_llm_with_retry(
    config: &Config,
    system: &str,
    messages: &[Message],
    v: Verbosity,
    cache: &ResponseCache,
) -> Result<LlmResponse, String> {
    // Cache-first pass
    for entry in &config.entries {
        if let Some(resp) = cached_response(entry, system, messages, v, cache) {
            return Ok(resp);
        }
    }

    let mut last_err = String::new();
    for (idx, entry) in config.entries.iter().enumerate() {
        // ...
        let mut msgs = messages.to_vec();
        let mut attempt = 0;
        while attempt < entry.retries {
            attempt += 1;
            match call_llm(entry, system, &msgs, v, cache, config.reasoning) {
                Ok(resp) if !resp.content.is_empty() => return Ok(resp),
                Ok(_) => {
                    // 空响应 → 注入重试提示
                    if attempt < entry.retries {
                        msgs.push(Message { role: "assistant".into(), content: "(no response)".into() });
                        msgs.push(Message { role: "user".into(), content: RETRY_HINT.into() });
                    }
                }
                Err(e) => {
                    last_err = e;
                    break; // 跳到下一个 model entry
                }
            }
        }
    }
    // ...
}
```

- `.to_vec()`：从切片创建新的 `Vec`（深拷贝）。
- 嵌套循环：外层遍历 model entries（fallback），内层重试。
- `Ok(resp) if !resp.content.is_empty()`：match guard，只匹配非空响应。
- 重试策略：空响应时注入"你的上一条回复为空"的提示，让 LLM 重新生成。

---

## 5. ui.rs — 终端交互

### Trait 实现（impl Trait for Type）

```rust
pub struct FileHelper {
    completer: FilenameCompleter,
}

impl Helper for FileHelper {}
impl Validator for FileHelper {}

impl Completer for FileHelper {
    type Candidate = <FilenameCompleter as Completer>::Candidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        self.completer.complete(line, pos, ctx)
    }
}
```

- **`impl Trait for Type`**：为类型实现 trait（接口）。rustyline 要求 Helper 实现多个 trait。
- **关联类型** `type Candidate = ...`：trait 中的类型别名。`<FilenameCompleter as Completer>::Candidate` 是完全限定语法（Fully Qualified Syntax），当有歧义时使用。
- 空 `impl Helper for FileHelper {}`：使用默认实现。

### `Verbosity` newtype

```rust
#[derive(Clone, Copy)]
pub struct Verbosity(pub u8);

impl Verbosity {
    pub fn show_prompt(&self) -> bool { self.0 >= 1 }
    pub fn show_debug(&self) -> bool { self.0 >= 2 }
}
```

- **Newtype pattern**：用元组结构体包装基础类型。`(pub u8)` 表示第一个字段公开。
- 给基础类型赋予语义：`Verbosity(2)` 比 `2u8` 自解释得多。

### `enum` 作为代数数据类型

```rust
pub enum EditAction {
    Execute(String),
    Refine(String),
    Cancel,
}
```

- 带数据的枚举变体：`Execute` 和 `Refine` 携带 `String`，`Cancel` 无数据。这是 Rust 的 tagged union（标记联合），也叫 sum type。

### 字节级字符串解析

```rust
pub fn split_comment(raw: &str) -> (&str, Option<&str>) {
    let bytes = raw.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut prev = b'\0';
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double && prev != b'\\' => in_single = !in_single,
            b'"' if !in_single && prev != b'\\' => in_double = !in_double,
            b'#' if !in_single && !in_double => {
                let comment = raw[i + 1..].trim();
                if comment.is_empty() {
                    return (raw.trim(), None);
                }
                return (raw[..i].trim(), Some(comment));
            }
            _ => {}
        }
        prev = b;
    }
    (raw.trim(), None)
}
```

- `b'\''`、`b'#'`：字节字面量（`u8`），比 `'\''` as u8 更明确。
- `&b`：迭代器解构，`&` 模式匹配引用的值。
- match guard 组合多个条件：引号内不处理注释，转义字符不翻转引号状态。
- 返回值是两个切片引用（`&str`），指向原始字符串的子区间，零拷贝。

### crossterm 终端操作

```rust
let _ = crossterm::execute!(
    io::stdout(),
    crossterm::cursor::MoveUp(rows),
    crossterm::cursor::MoveToColumn(0),
    crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
);
```

- **`execute!` 宏**：向终端写入转义序列。`let _ =` 忽略返回值（显式丢弃，编译器不会警告 unused result）。
- `crossterm` 是跨平台终端操作库，替代 `termion`（仅 Unix）。

### `select_command` — 交互式选择器

```rust
let _ = crossterm::terminal::enable_raw_mode();

let result = loop {
    if let Ok(Event::Key(KeyEvent { code, modifiers, kind, .. })) = event::read() {
        if kind != KeyEventKind::Press {
            continue;
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if selected > 0 { selected -= 1; }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if selected < candidates.len() - 1 { selected += 1; }
            }
            KeyCode::Enter => {
                // ...
                return Some(selected);
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break None,
            _ => {}
        }
        // 重绘...
    }
};

let _ = crossterm::terminal::disable_raw_mode();
result
```

- **raw mode**：禁用终端的行缓冲和信号处理，程序直接接收每个按键。
- **`..` 省略符**：解构时忽略未命名的字段。
- `loop` 可以返回值：`break None` 退出循环并返回 `None`。
- `KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL)`：Ctrl+C 检测。

### `Spinner` — 多线程动画

```rust
pub struct Spinner {
    handle: Option<std::thread::JoinHandle<()>>,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Spinner {
    pub fn start(msg: &str) -> Self {
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let running_clone = running.clone();

        let handle = std::thread::spawn(move || {
            let mut i = 0;
            while running_clone.load(std::sync::atomic::Ordering::Relaxed) {
                // 打印动画帧
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        });

        Self { handle: Some(handle), running }
    }

    pub fn stop(&mut self) {
        self.running.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
```

- **`Arc<AtomicBool>`**：线程间共享的原子布尔。`Arc`（Atomic Reference Counting）允许跨线程的所有权共享。
- **`.clone()`**：`Arc::clone` 只增加引用计数，不深拷贝数据。
- **`move ||`**：`move` 闭包，捕获变量的所有权。闭包需要 `move` 才能将 `running_clone` 带入新线程。
- **`Ordering::Relaxed`**：最弱的内存序。对于简单的标志位足够，不需要 `SeqCst`。
- **`handle.take()`**：`Option::take()` 取出值并留下 `None`，确保 `join` 只调用一次。

### 截断函数的 UTF-8 安全性

```rust
pub fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= max)
            .last()
            .unwrap_or(0);
        &s[..end]
    }
}
```

- `s.len()` 是字节长度，不是字符长度。中文字符占 3 字节，emoji 占 4 字节。
- `.char_indices()`：产生 `(byte_offset, char)` 的迭代器。
- 这个函数确保切片不会在多字节字符中间断开——Rust 的 `&str` 切片如果不在 UTF-8 边界会 panic。

### 剪贴板工具探测

```rust
pub fn copy_to_clipboard(text: &str) {
    let tools: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
        ("clip", &[]),
    ];
    for (cmd, args) in tools {
        if std::process::Command::new(cmd)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.as_mut().unwrap().write_all(text.as_bytes())?;
                child.wait()?;
                Ok(())
            })
            .is_ok()
        {
            return;
        }
    }
    // ...
}
```

- `&[(&str, &[&str])]`：嵌套的静态切片引用。整个数组在编译时确定。
- `.and_then()` 链式处理 `Result`：成功时继续，失败时短路。
- `Stdio::piped()`：创建管道，允许向子进程的 stdin 写入。
- `child.stdin.as_mut().unwrap()`：`Option<&mut T>` → `&mut T`。piped 模式下 stdin 一定存在，所以 `unwrap` 安全。

---

## 6. cache.rs — 响应缓存

### `DefaultHasher` 与缓存键

```rust
pub fn cache_key(model: &str, system: &str, messages: &[Message]) -> String {
    let mut h = DefaultHasher::new();
    model.hash(&mut h);
    system.hash(&mut h);
    for m in messages {
        m.role.hash(&mut h);
        m.content.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}
```

- `Hash` trait：Rust 的哈希协议。`String` 和 `&str` 自动实现。
- `.hash(&mut h)`：向 hasher 喂数据。
- `format!("{:016x}", ...)`：16 位十六进制格式化，补零。

### `HashMap` + 文件持久化

```rust
pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_size: usize,
    path: PathBuf,
    dirty: bool,
}
```

- `dirty: bool`：脏标志，避免不必要的磁盘写入。典型的"延迟写回"策略。

### `impl From<&LlmResponse> for CacheEntry`

```rust
impl From<&LlmResponse> for CacheEntry {
    fn from(resp: &LlmResponse) -> Self {
        Self {
            content: resp.content.clone(),
            usage: CacheUsage { /* ... */ },
            ts: now_ts(),
        }
    }
}
```

- **`From` trait 实现**：定义类型间的转换。实现了 `From` 就自动获得 `Into`。
- `CacheEntry::from(&resp)` 或 `(&resp).into()` 都可以调用。

### 原子写入（rename 策略）

```rust
pub fn save(&self) {
    if !self.dirty { return; }
    if let Ok(json) = serde_json::to_string(&self.entries) {
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}
```

- **先写临时文件，再 rename**：rename 在同一文件系统内是原子操作。写到一半崩溃不会损坏原文件。
- `&&` 短路：只有写入成功才尝试 rename。

### LRU 淘汰

```rust
pub fn put(&mut self, key: String, entry: CacheEntry) {
    if self.max_size == 0 { return; }
    self.entries.insert(key, entry);
    self.dirty = true;
    if self.entries.len() > self.max_size {
        let mut oldest_key = String::new();
        let mut oldest_ts = u64::MAX;
        for (k, v) in &self.entries {
            if v.ts < oldest_ts {
                oldest_ts = v.ts;
                oldest_key = k.clone();
            }
        }
        if !oldest_key.is_empty() {
            self.entries.remove(&oldest_key);
        }
    }
}
```

- 线性扫描找最老条目。对于 1000 条以内的缓存够用。工业级缓存会用 `linked-hash-map` 或 `lru` crate。

---

## 7. protocol.rs — 协议处理

### `#CHECK:` / `#EXPLORE:` 前缀解析

```rust
pub fn parse_check(raw: &str) -> Option<Vec<&str>> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix(CHECK_PREFIX)?.trim();
    if rest.is_empty() { return None; }
    let (tool_str, _) = split_comment(rest);
    let tools: Vec<&str> = tool_str.split_whitespace().collect();
    if tools.is_empty() { None } else { Some(tools) }
}
```

- `.strip_prefix()`：移除前缀，返回 `Option<&str>`（可能没有该前缀）。
- `?` 操作符链式传播 `None`。
- `.split_whitespace()`：按空白字符分割，自动忽略连续空格。

### `pipe_reader` — 并发管道读取

```rust
fn pipe_reader(pipe: Option<impl Read + Send + 'static>, tx: mpsc::Sender<Vec<u8>>) {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
}
```

- **`impl Read + Send + 'static`**：trait bound。要求参数实现 `Read`（可读）、`Send`（可跨线程转移）、`'static`（无短生命周期引用）。
- `mpsc::channel`：多生产者单消费者通道。`tx.send()` 发送数据，`rx.recv()` 接收。
- `move ||`：闭包捕获 `tx` 的所有权。

### 超时控制

```rust
fn run_and_capture(cmd: &str) -> Result<String, String> {
    let mut child = Command::new(prog)...spawn()?;

    let start = Instant::now();
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= EXPLORE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => { /* ... */ }
        }
    }
    // ...
}
```

- `.try_wait()`：非阻塞检查子进程是否退出。`Ok(Some(_))` 已退出，`Ok(None)` 仍在运行。
- 轮询 + sleep：简单的超时实现。工业级会用 `select!` 或异步 I/O。

### `process_response` — 协议链

```rust
pub fn process_response(..., raw: &str, ...) -> String {
    let mut current = raw.to_string();
    let mut explored = false;

    for _ in 0..5 {  // 最多 5 轮
        let after_check = match check_then_generate(...) {
            Ok(Some(cmd)) => cmd,
            Ok(None) => current.clone(),
            Err(e) => { /* ... */ current.clone() }
        };

        if explored { return after_check; }

        match explore_then_generate(...) {
            Ok(Some(cmd)) => { explored = true; current = cmd; }
            // ...
        }
    }
    current
}
```

- `for _ in 0..5`：固定次数循环。`_` 表示不使用循环变量。
- 状态机：`#CHECK` 可以多轮（多工具探测），`#EXPLORE` 只执行一次。

---

## 8. context.rs — 系统上下文

### `run_cmd` — 命令执行辅助

```rust
pub fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}
```

- `.ok()`：`Result` → `Option`，丢弃错误。
- `.and_then()` 链：成功执行 → 检查退出码 → 转 UTF-8 → 去空白。
- `String::from_utf8()`：`Vec<u8>` → `Result<String, FromUtf8Error>`。输出可能不是合法 UTF-8。

### 平台信息采集

```rust
fn get_distro() -> String {
    if let Some(content) = read_file("/etc/os-release") {
        let name = content
            .lines()
            .find(|l| l.starts_with("PRETTY_NAME="))
            .and_then(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string());
        if let Some(n) = name { return n; }
    }
    run_cmd("lsb_release", &["-ds"]).unwrap_or_else(|| "Linux (unknown distro)".into())
}
```

- `.lines()`：按换行分割字符串。
- `.find()`：找到第一个满足条件的行。
- `.trim_matches('"')`：移除两端的引号。

### 隐私保护 — placeholder 系统

```rust
pub struct Placeholders {
    pub user: String,
    pub hostname: String,
    pub home: String,
}

pub fn apply_placeholders(cmd: &str, ph: &Placeholders) -> String {
    cmd.replace("{{USER}}", &ph.user)
        .replace("{{HOSTNAME}}", &ph.hostname)
        .replace("{{HOME}}", &ph.home)
}
```

- LLM 输出中的 `{{USER}}` 等占位符，在本地替换为真实值。
- **隐私设计**：系统上下文（发送给 API）中已经把 CWD 中的用户名和 home 路径替换为占位符，真实值只在本地替换，永不发送到 API。

```rust
let mut cwd = cwd_raw;
if !home.is_empty() {
    cwd = cwd.replace(&home, "{{HOME}}");
}
if !user.is_empty() {
    cwd = cwd.replace(&user, "{{USER}}");
}
```

- `!home.is_empty()` 检查：`str::replace` 用空字符串作 needle 会在每个字符间插入替换值。

---

## 9. prompt.rs — 提示词模板

### 模板替换

```rust
pub fn load_prompt(config: &Config) -> String {
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_PROMPT.into());
    let ctx = gather_context();
    let prefs = format_preferences(&config.prefer);

    raw.replace("{{SYSTEM_CONTEXT}}", &ctx)
        .replace("{{PREFERENCES}}", &prefs)
}
```

- `.unwrap_or_else(|_| DEFAULT_PROMPT.into())`：文件不存在时使用内嵌默认提示词。
- 模板系统用 `.replace()` 实现，简单但有效。

### `format_preferences` — 排序与格式化

```rust
fn format_preferences(prefer: &HashMap<String, Vec<String>>) -> String {
    if prefer.is_empty() { return "(none configured)".to_string(); }
    let mut lines: Vec<String> = Vec::new();
    let mut keys: Vec<&String> = prefer.keys().collect();
    keys.sort();
    for key in keys {
        if let Some(tools) = prefer.get(key) {
            lines.push(format!("- {}: {}", key, tools.join(" > ")));
        }
    }
    lines.join("\n")
}
```

- `.keys().collect()`：收集所有 key 为 `Vec<&String>`。
- `.sort()`：原地排序。`&String` 实现了 `Ord`。
- `.join(" > ")`：用分隔符连接切片。

---

## 10. danger.rs — 危险命令检测

### 静态模式数组

```rust
const DANGER_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    // ...
];
```

- `const`：编译时常量。`&[&str]` 是静态字符串切片的切片。

### 空白归一化 + 大小写不敏感匹配

```rust
pub fn is_dangerous(cmd: &str) -> bool {
    let (command, _) = split_comment(cmd);
    let lower = command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if DANGER_PATTERNS.iter().any(|p| lower.contains(&p.to_lowercase())) {
        return true;
    }
    lower.split('|').any(segment_runs_shell)
}
```

- `split_whitespace().collect().join(" ")`：将连续空白归一化为单个空格。
- `.any()`：迭代器短路检查，任一匹配即返回 `true`。
- `.split('|')`：按管道符分割，逐段检查是否是危险的 pipe-to-shell。

### `segment_runs_shell` — 精确 token 匹配

```rust
fn segment_runs_shell(segment: &str) -> bool {
    let mut tokens = segment
        .split(|c: char| c.is_whitespace() || c == ';' || c == '&')
        .filter(|t| !t.is_empty());
    match tokens.next() {
        Some(shell) if PIPE_SHELLS.contains(&shell) => true,
        Some("sudo") => matches!(tokens.next(), Some(t) if PIPE_SUDO_SHELLS.contains(&t)),
        _ => false,
    }
}
```

- `.split(|c: char| ...)`：自定义分隔符的闭包。按空白、`;`、`&` 分割。
- `PIPE_SHELLS.contains(&shell)`：切片的 `.contains()` 方法。
- `matches!()` 宏用于 match guard 中。

---

## 11. update.rs — 自动更新

### GitHub API 调用

```rust
fn get_latest_version() -> Result<(String, String), String> {
    let client = make_client()?;
    let resp = client
        .get("https://api.github.com/repos/miuzel/comma-cli/releases/latest")
        .header("User-Agent", format!("comma/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .map_err(|e| format!("GitHub API: {}", e))?;
    // ...
}
```

- GitHub API 要求 `User-Agent` header。
- `?` 链式传播错误。

### 版本比较

```rust
fn version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.').filter_map(|s| s.parse().ok()).collect()
    };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv > cv { return true; }
        if lv < cv { return false; }
    }
    false
}
```

- `.filter_map(|s| s.parse().ok())`：`filter_map` = `filter` + `map`。解析失败的段被跳过。
- `.copied()`：`Option<&u32>` → `Option<u32>`，复制值而非引用。

### 平台检测与内存泄漏

```rust
fn detect_platform() -> Option<&'static str> {
    // ...
    Some(Box::leak(format!("{}-{}", os, arch).into_boxed_str()))
}
```

- **`Box::leak`**：将 `Box<str>` 泄漏为 `&'static str`。动态构建的字符串获得静态生命周期。
- 这是有意的内存泄漏——程序只有一个平台字符串，泄漏一次无所谓。

### SHA256 校验

```rust
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}
```

- `.finalize()` 消耗 hasher，产出哈希值。
- `.iter().map(...).collect()`：字节数组 → 十六进制字符串。`.collect()` 从 `Iterator<Item = String>` 收集为 `String`（`FromIterator` trait 的魔法）。

### 条件编译

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&extracted_binary, std::fs::Permissions::from_mode(0o755));
}
```

- `#[cfg(unix)]`：条件编译属性。只在 Unix 平台编译这个块。
- `0o755`：八进制字面量（owner rwx, group rx, other rx）。

### 错误恢复

```rust
if let Err(_e) = std::fs::rename(&exe_path, &old_path) {
    // Rename 失败 → 直接 copy 覆盖
    if let Err(e) = std::fs::copy(&extracted_binary, &exe_path) {
        print_error(&format!("Replace binary: {}", e));
        return;
    }
} else {
    // Rename 成功 → copy 新文件到位
    if let Err(e) = std::fs::copy(&extracted_binary, &exe_path) {
        // copy 失败 → 恢复旧文件
        let _ = std::fs::rename(&old_path, &exe_path);
        print_error(&format!("Replace binary: {}", e));
        return;
    }
}
```

- `if let Err(_e)`：`_e` 带前缀下划线表示"有意不使用"，抑制编译器警告。
- Windows 上运行中的 exe 被锁，rename 后再 copy 可以绕过。

---

## 12. tests.rs — 内置测试

### 测试辅助闭包

```rust
let mut check = |name: &str, ok: bool| {
    if ok {
        println!("  ✓ {}", name);
        pass += 1;
    } else {
        println!("  ✗ {}", name);
        fail += 1;
    }
};
```

- **可变闭包**：`|name, ok|` 修改外部的 `pass`/`fail`，需要 `mut`。
- 闭包捕获了 `&mut pass` 和 `&mut fail`。

### 环境变量保存/恢复

```rust
let saved_home = std::env::var("HOME").ok();
std::env::set_var("HOME", "");
// ... 测试 ...
match &saved_home {
    Some(h) => std::env::set_var("HOME", h),
    None => std::env::remove_var("HOME"),
}
```

- 测试修改全局状态（环境变量）后必须恢复，避免污染后续测试。
- `match &saved_home`：引用匹配，不转移 `saved_home` 的所有权。

### 隐私泄露检测

```rust
check(
    "context does not leak username",
    !ctx.contains(&ph.user),
);
```

- 验证发送给 API 的系统上下文不包含真实用户名。这是安全测试。

### `truncate` 的边界安全性测试

```rust
let s = "héllo 🌍";
let mut boundary_ok = true;
for m in 0..s.len() {
    let t = truncate(s, m);
    if t.len() > m || !s.starts_with(t) {
        boundary_ok = false;
    }
}
check("truncate: never splits a char", boundary_ok);
```

- 遍历所有可能的 `max` 值，验证截断结果永远不会超过 `max` 字节，且始终是原字符串的前缀。
- 这是 property-based testing 的简化版。

---

## 跨模块设计模式总结

### 所有权与借用

整个项目大量使用引用（`&Config`、`&str`、`&[Message]`）传递数据，只在必要时 `.clone()`。这是 Rust 的核心设计哲学：默认借用，显式拥有。

### `Result` + `?` 错误传播

错误通过 `Result<T, String>` 逐层传播，`?` 操作符简化了代码。`String` 作为错误类型虽然不如 `thiserror`/`anyhow` 专业，但对小项目足够。

### 迭代器链

`filter` → `map` → `collect`、`any`、`find` 等组合贯穿全项目。Rust 的迭代器是零开销抽象（编译后与手写循环等价）。

### Builder 模式

`Command::new().args().arg().status()`、`Client::builder().timeout().build()` 等链式构建。

### Newtype Pattern

`Verbosity(u8)` 包装基础类型，增加语义和类型安全。

### 条件编译

`#[cfg(unix)]`、`#[cfg(target_os = "windows")]` 处理平台差异。

---

*文档生成于 2026-07-20，基于 comma-cli v0.17.5 源码。*
