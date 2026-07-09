use crossterm::style::{Color, ResetColor, SetForegroundColor};
use rustyline::completion::{Completer, FilenameCompleter};
use rustyline::config::Configurer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::{self, Write};
use std::path::PathBuf;

// ── Rustyline helper wrapper ────────────────────────────────────────────────

struct FileHelper {
    completer: FilenameCompleter,
}

impl FileHelper {
    fn new() -> Self {
        Self {
            completer: FilenameCompleter::new(),
        }
    }
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

impl Hinter for FileHelper {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for FileHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Borrowed(hint)
    }
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }
}

// ── Verbosity ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Verbosity(u8);

impl Verbosity {
    fn show_prompt(&self) -> bool {
        self.0 >= 1
    }
    fn show_debug(&self) -> bool {
        self.0 >= 2
    }
}

// ── API style ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiStyle {
    OpenAI,
    Anthropic,
}

impl ApiStyle {
    /// Auto-detect from URL. Defaults to OpenAI if not clearly Anthropic.
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

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct LocalConfig {
    base_url: Option<String>,
    auth_token: Option<String>,
    model: Option<String>,
    api_style: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeSettings {
    env: Option<ClaudeEnv>,
}

#[derive(Deserialize)]
struct ClaudeEnv {
    #[serde(rename = "ANTHROPIC_BASE_URL")]
    base_url: Option<String>,
    #[serde(rename = "ANTHROPIC_AUTH_TOKEN")]
    auth_token: Option<String>,
    #[serde(rename = "ANTHROPIC_MODEL")]
    model: Option<String>,
}

struct Config {
    base_url: String,
    auth_token: String,
    model: String,
    api_style: ApiStyle,
}

fn home_dir() -> Result<String, String> {
    std::env::var("HOME").map_err(|_| "HOME not set".into())
}

fn load_config() -> Result<Config, String> {
    let home = home_dir()?;

    let local_path = PathBuf::from(&home).join(".local/bin/,.config.json");
    let local: LocalConfig = match std::fs::read_to_string(&local_path) {
        Ok(data) => serde_json::from_str(&data)
            .map_err(|e| format!("Invalid {}: {}", local_path.display(), e))?,
        Err(_) => LocalConfig::default(),
    };

    let claude_path = PathBuf::from(&home).join(".claude/settings.json");
    let claude_env: Option<ClaudeEnv> = match std::fs::read_to_string(&claude_path) {
        Ok(data) => {
            let settings: ClaudeSettings = serde_json::from_str(&data)
                .map_err(|e| format!("Invalid {}: {}", claude_path.display(), e))?;
            settings.env
        }
        Err(_) => None,
    };

    let non_empty = |o: Option<String>| o.filter(|s| !s.is_empty());

    let base_url = non_empty(local.base_url)
        .or_else(|| claude_env.as_ref().and_then(|e| e.base_url.clone()))
        .unwrap_or_else(|| "https://api.anthropic.com".into());

    let auth_token = non_empty(local.auth_token)
        .or_else(|| claude_env.as_ref().and_then(|e| e.auth_token.clone()))
        .ok_or("No auth_token: set in ,.config.json or ANTHROPIC_AUTH_TOKEN in ~/.claude/settings.json")?;

    let model = non_empty(local.model)
        .or_else(|| claude_env.as_ref().and_then(|e| e.model.clone()))
        .unwrap_or_else(|| "claude-sonnet-4-20250514".into());

    // api_style: explicit > auto-detect from URL
    let api_style = non_empty(local.api_style)
        .and_then(|s| ApiStyle::from_str(&s))
        .unwrap_or_else(|| ApiStyle::from_url(&base_url));

    Ok(Config {
        base_url,
        auth_token,
        model,
        api_style,
    })
}

// ── System context ──────────────────────────────────────────────────────────

fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
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

fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn get_distro() -> String {
    // Try /etc/os-release
    if let Some(content) = read_file("/etc/os-release") {
        let name = content
            .lines()
            .find(|l| l.starts_with("PRETTY_NAME="))
            .and_then(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string());
        if let Some(n) = name {
            return n;
        }
    }
    // Try lsb_release
    run_cmd("lsb_release", &["-ds"]).unwrap_or_else(|| "Linux (unknown distro)".into())
}

fn get_kernel() -> String {
    run_cmd("uname", &["-srmo"]).unwrap_or_else(|| "unknown".into())
}

fn get_arch() -> String {
    run_cmd("uname", &["-m"]).unwrap_or_else(|| "unknown".into())
}

fn get_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

fn get_user() -> String {
    run_cmd("whoami", &[])
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "user".into())
}

fn get_hostname() -> String {
    run_cmd("hostname", &[]).unwrap_or_else(|| "localhost".into())
}

fn get_packages() -> String {
    let mut found: Vec<String> = Vec::new();

    // Detect package manager and list installed packages (truncated)
    let managers: &[(&str, &[&str], usize)] = &[
        ("dpkg", &["-l"], 200),   // Debian/Ubuntu
        ("rpm", &["-qa"], 200),   // RHEL/Fedora
        ("pacman", &["-Q"], 200), // Arch
        ("apk", &["list", "--installed"], 100), // Alpine
        ("xbps-query", &["-l"], 100), // Void
    ];

    for (cmd, args, limit) in managers {
        if let Some(output) = run_cmd(cmd, args) {
            let count = output.lines().count();
            let lines: Vec<&str> = output.lines().take(*limit).collect();
            found.push(format!(
                "[{} ({} packages total, showing first {}):\n{}]",
                cmd,
                count,
                limit,
                lines.join("\n")
            ));
            break; // Use first found package manager
        }
    }

    // Also list key tools
    let tools = &[
        "git", "curl", "wget", "python3", "node", "npm", "cargo", "rustc",
        "docker", "podman", "make", "cmake", "gcc", "clang", "vim", "nvim",
        "jq", "rg", "fd", "fzf", "tmux", "htop", "ssh", "rsync",
    ];
    let available: Vec<&str> = tools
        .iter()
        .filter(|t| run_cmd("which", &[t]).is_some())
        .copied()
        .collect();
    found.push(format!("[Available tools: {}]", available.join(", ")));

    found.join("\n")
}

/// Non-private system context sent to the API.
/// Sanitizes CWD to avoid leaking username/home path.
fn gather_context() -> String {
    let distro = get_distro();
    let kernel = get_kernel();
    let arch = get_arch();
    let shell = get_shell();
    let home = home_dir().unwrap_or_default();
    let user = get_user();

    let cwd_raw = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    // Replace home path and username occurrences in CWD
    let cwd = cwd_raw
        .replace(&home, "{{HOME}}")
        .replace(&user, "{{USER}}");

    let packages = get_packages();

    format!(
        "Distro: {}\nKernel: {}\nArch: {}\nShell: {}\nCWD: {}\n\nInstalled packages & tools:\n{}",
        distro, kernel, arch, shell, cwd, packages
    )
}

/// Private placeholders — never sent to the API, only substituted locally.
struct Placeholders {
    user: String,
    hostname: String,
    home: String,
}

fn collect_placeholders() -> Placeholders {
    Placeholders {
        user: get_user(),
        hostname: get_hostname(),
        home: home_dir().unwrap_or_else(|_| "~".into()),
    }
}

/// Replace {{USER}}, {{HOSTNAME}}, {{HOME}} in LLM output with real values.
fn apply_placeholders(cmd: &str, ph: &Placeholders) -> String {
    cmd.replace("{{USER}}", &ph.user)
        .replace("{{HOSTNAME}}", &ph.hostname)
        .replace("{{HOME}}", &ph.home)
}

// ── Prompt ──────────────────────────────────────────────────────────────────

fn load_prompt() -> String {
    let home = home_dir().unwrap_or_default();
    let path = PathBuf::from(&home).join(".local/bin/,.prompt.md");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_PROMPT.into());

    // Gather non-private context and substitute
    let ctx = gather_context();
    raw.replace("{{SYSTEM_CONTEXT}}", &ctx)
}

const DEFAULT_PROMPT: &str = r#"You are a shell command generator. The user describes intent in natural language; you output the corresponding shell command.

Rules:
- Output exactly ONE shell command that can be executed directly. No explanations.
- The command should be concise, general-purpose, and correct for the user's platform (see system context below).
- If the intent is ambiguous, output the most reasonable default.
- Prefer modern tools (e.g. ripgrep over grep, fd over find) when available on this system.
- If the intent cannot be achieved in one command, output the closest command with a # comment noting the limitation.
- Output ONLY the command, nothing else. No markdown fences, no prose.
- Tailor commands to the installed package manager and available tools.

Exploration:
If you are NOT SURE about a tool's exact usage/flags, prefix your response with #EXPLORE: followed by a help command.
Example: #EXPLORE: openclaw --help
The tool will run it, capture the output, and ask you again with that context.
Use #EXPLORE: ONLY when you genuinely need to learn about a tool. If you already know the command, output it directly.

Private data placeholders — use these when the command references user/host/home:
- {{USER}} for the current username
- {{HOSTNAME}} for the machine hostname
- {{HOME}} for the home directory path
The tool will replace these with real values locally after you respond.

System context:
{{SYSTEM_CONTEXT}}"#;

// ── API ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct Message {
    role: String,
    content: String,
}

// ── OpenAI-compatible types ─────────────────────────────────────────────────

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<OpenAiMessage>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Option<Vec<OpenAiChoice>>,
    error: Option<OpenAiError>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: Option<OpenAiChoiceMessage>,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiError {
    message: Option<String>,
}

// ── Anthropic types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Option<Vec<AnthropicContentBlock>>,
    error: Option<AnthropicApiError>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicApiError {
    message: Option<String>,
}

// ── Normalize base URL ─────────────────────────────────────────────────────

/// Strip trailing slash and known suffixes like /v1, /v1/.
fn normalize_base_url(url: &str) -> String {
    let mut u = url.trim_end_matches('/').to_string();
    // Strip trailing /v1 if present — we'll append the correct path
    if u.ends_with("/v1") {
        u.truncate(u.len() - 3);
    }
    u
}

// ── Call LLM ────────────────────────────────────────────────────────────────

const MAX_RETRIES: usize = 3;

const RETRY_HINT: &str =
    "Your previous response was empty. You MUST output exactly ONE shell command. No explanations, no markdown fences. Just the raw command.";

fn call_llm(config: &Config, system: &str, messages: &[Message], v: Verbosity) -> Result<String, String> {
    match config.api_style {
        ApiStyle::OpenAI => call_openai(config, system, messages, v),
        ApiStyle::Anthropic => call_anthropic(config, system, messages, v),
    }
}

/// Call LLM with retry on empty response. Up to MAX_RETRIES attempts.
fn call_llm_with_retry(
    config: &Config,
    system: &str,
    messages: &[Message],
    v: Verbosity,
) -> Result<String, String> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let result = call_llm(config, system, messages, v)?;
        let trimmed = result.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        if attempts >= MAX_RETRIES {
            return Err(format!(
                "Model returned empty response after {} attempts.",
                MAX_RETRIES
            ));
        }
        print_info(&format!(
            "Empty response, retrying ({}/{})...",
            attempts, MAX_RETRIES
        ));
        // We need to append the retry hint to the conversation.
        // Since we can't mutate `messages`, we build a temporary extended copy.
        let mut retry_msgs = messages.to_vec();
        retry_msgs.push(Message {
            role: "assistant".into(),
            content: String::new(),
        });
        retry_msgs.push(Message {
            role: "user".into(),
            content: RETRY_HINT.to_string(),
        });
        // Re-call with extended messages (only affects this attempt)
        let retry_result = call_llm(config, system, &retry_msgs, v)?;
        if !retry_result.trim().is_empty() {
            return Ok(retry_result.trim().to_string());
        }
        // If still empty, loop will check attempts count
    }
}

fn make_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP client: {}", e))
}

fn call_openai(config: &Config, system: &str, messages: &[Message], v: Verbosity) -> Result<String, String> {
    let base = normalize_base_url(&config.base_url);
    let url = format!("{}/v1/chat/completions", base);

    let mut oai_messages: Vec<OpenAiMessage> = Vec::new();
    oai_messages.push(OpenAiMessage {
        role: "system".into(),
        content: system.to_string(),
    });
    for m in messages {
        oai_messages.push(OpenAiMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }

    let body = OpenAiRequest {
        model: config.model.clone(),
        max_tokens: 1024,
        messages: oai_messages,
    };

    if v.show_debug() {
        print_debug(&format!("POST {}", url));
        if let Ok(json) = serde_json::to_string_pretty(&body) {
            print_debug(&format!("Request body:\n{}", json));
        }
    }

    let client = make_client()?;
    let t0 = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.auth_token))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    let elapsed = t0.elapsed();
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read body: {}", e))?;

    if v.show_debug() {
        print_debug(&format!("Status: {} ({:.1}s)", status, elapsed.as_secs_f64()));
        print_debug(&format!("Response:\n{}", truncate(&text, 2000)));
    }

    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, text));
    }

    let api_resp: OpenAiResponse =
        serde_json::from_str(&text).map_err(|e| format!("Parse response: {}", e))?;
    if let Some(err) = api_resp.error {
        return Err(err.message.unwrap_or_else(|| "Unknown API error".into()));
    }
    let choices = api_resp.choices.ok_or("Empty response: no choices")?;
    let content = choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.as_deref())
        .unwrap_or("")
        .trim();

    if v.show_prompt() {
        print_debug(&format!("LLM reply: {}", content));
    }

    Ok(content.to_string())
}

fn call_anthropic(config: &Config, system: &str, messages: &[Message], v: Verbosity) -> Result<String, String> {
    let base = normalize_base_url(&config.base_url);
    let url = format!("{}/v1/messages", base);

    let body = AnthropicRequest {
        model: config.model.clone(),
        max_tokens: 1024,
        system: system.to_string(),
        messages: messages.to_vec(),
    };

    if v.show_debug() {
        print_debug(&format!("POST {}", url));
        if let Ok(json) = serde_json::to_string_pretty(&body) {
            print_debug(&format!("Request body:\n{}", json));
        }
    }

    let client = make_client()?;
    let t0 = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("x-api-key", &config.auth_token)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    let elapsed = t0.elapsed();
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read body: {}", e))?;

    if v.show_debug() {
        print_debug(&format!("Status: {} ({:.1}s)", status, elapsed.as_secs_f64()));
        print_debug(&format!("Response:\n{}", truncate(&text, 2000)));
    }

    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, text));
    }

    let api_resp: AnthropicResponse =
        serde_json::from_str(&text).map_err(|e| format!("Parse response: {}", e))?;
    if let Some(err) = api_resp.error {
        return Err(err.message.unwrap_or_else(|| "Unknown API error".into()));
    }
    let content = api_resp.content.ok_or("Empty response")?;
    let result: String = content
        .iter()
        .filter_map(|b| b.text.as_deref())
        .collect::<Vec<_>>()
        .join("");
    let trimmed = result.trim();

    if v.show_prompt() {
        print_debug(&format!("LLM reply: {}", trimmed));
    }

    Ok(trimmed.to_string())
}

// ── Exploration: #EXPLORE: prefix ───────────────────────────────────────────

const EXPLORE_PREFIX: &str = "#EXPLORE:";

const EXPLORE_HINT: &str = "\
The help output is shown above. Now generate the ACTUAL shell command the user originally wanted. \
Output ONLY the final command. Do NOT prefix with #EXPLORE: again.";

/// If raw starts with `#EXPLORE:`, extract the command after the prefix.
fn parse_explore(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    trimmed.strip_prefix(EXPLORE_PREFIX).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// Run a command, capture stdout+stderr (up to 4096 chars).
fn run_and_capture(cmd: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| format!("Failed to run: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut result = stdout;
    if !stderr.is_empty() {
        result.push_str("\n[stderr]\n");
        result.push_str(&stderr);
    }
    Ok(truncate(&result, 4096).to_string())
}

/// If the model returned `#EXPLORE: <cmd>`, run it with user permission,
/// feed output back to the LLM, and return the real command.
/// Returns Ok(None) if user declines or no #EXPLORE: prefix.
fn explore_then_generate(
    config: &Config,
    system: &str,
    messages: &[Message],
    raw: &str,
    ph: &Placeholders,
    v: Verbosity,
) -> Result<Option<String>, String> {
    let explore_cmd = match parse_explore(raw) {
        Some(cmd) => cmd,
        None => return Ok(None),
    };
    let cmd = apply_placeholders(explore_cmd, ph);

    print_info(&format!("Model wants to explore: {}", cmd));
    if !prompt_confirm("Run to learn usage?") {
        return Ok(None);
    }

    let help_output = run_and_capture(&cmd)?;
    if help_output.trim().is_empty() {
        print_info("No output from command.");
        return Ok(None);
    }

    if v.show_debug() {
        print_debug(&format!(
            "Captured ({} chars):\n{}",
            help_output.len(),
            truncate(&help_output, 1000)
        ));
    }

    print_info("Learning from output...");

    // Feed help output back: original messages + assistant(#EXPLORE: cmd) + user(hint + output)
    let mut ext = messages.to_vec();
    ext.push(Message {
        role: "assistant".into(),
        content: raw.to_string(),
    });
    ext.push(Message {
        role: "user".into(),
        content: format!("{}\n\nCommand output:\n```\n{}\n```", EXPLORE_HINT, help_output),
    });

    let result = call_llm_with_retry(config, system, &ext, v)?;
    Ok(Some(result))
}

// ── Danger detection ────────────────────────────────────────────────────────

const DANGER_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf /*",
    "dd if=",
    "mkfs.",
    ":(){ :|:& };:",
    "chmod -R 777 /",
    "> /dev/sd",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "| sh",
    "| bash",
    "| zsh",
    "| sudo sh",
    "| sudo bash",
    "sudo rm",
    "git push --force",
    "DROP TABLE",
    "DROP DATABASE",
    "FORMAT ",
    "del /f /s /q",
    "rd /s /q",
];

fn is_dangerous(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    DANGER_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

// ── Display helpers ─────────────────────────────────────────────────────────

fn print_cmd(cmd: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if is_dangerous(cmd) {
        let _ = write!(
            out,
            "{}⚠ DANGEROUS COMMAND ⚠{}",
            SetForegroundColor(Color::Red),
            ResetColor
        );
        let _ = writeln!(out);
    }
    let _ = write!(
        out,
        "{}{}{}",
        SetForegroundColor(Color::Green),
        cmd,
        ResetColor
    );
    let _ = writeln!(out);
}

fn print_info(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(
        out,
        "{}▸ {}{}",
        SetForegroundColor(Color::DarkGrey),
        msg,
        ResetColor
    );
    let _ = writeln!(out);
}

fn print_error(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(
        out,
        "{}✗ {}{}",
        SetForegroundColor(Color::Red),
        msg,
        ResetColor
    );
    let _ = writeln!(out);
}

fn print_debug(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in msg.lines() {
        let _ = write!(
            out,
            "{}│{} {}",
            SetForegroundColor(Color::DarkGrey),
            ResetColor,
            line
        );
        let _ = writeln!(out);
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

fn prompt_confirm(msg: &str) -> bool {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(
        out,
        "{}{}{} [y/N] ",
        SetForegroundColor(Color::Yellow),
        msg,
        ResetColor
    );
    let _ = out.flush();
    let mut input = String::new();
    io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y")
}

fn prompt_input(rl: &mut Editor<FileHelper, DefaultHistory>) -> Option<String> {
    let prompt = format!("{}> {}", SetForegroundColor(Color::Cyan), ResetColor);
    match rl.readline(&prompt) {
        Ok(line) => {
            let trimmed = line.trim().to_string();
            let _ = rl.add_history_entry(&trimmed);
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(rustyline::error::ReadlineError::Interrupted)
        | Err(rustyline::error::ReadlineError::Eof) => None,
        Err(_) => None,
    }
}

fn prompt_input_fallback() -> Option<String> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(out, "{}> {}", SetForegroundColor(Color::Cyan), ResetColor);
    let _ = out.flush();
    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(0) => None,
        Ok(_) => {
            let trimmed = input.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(_) => None,
    }
}

// ── Main logic ──────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("comma {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return;
    }

    if args.iter().any(|a| a == "--test") {
        run_tests();
        return;
    }

    // Count -v flags (supports -v, -vv, -vvv)
    let verbosity = Verbosity(
        args.iter()
            .filter(|a| a.starts_with("-v") && a.chars().skip(1).all(|c| c == 'v'))
            .map(|a| a.len() as u8 - 1)
            .sum(),
    );

    // Filter out -v flags from args (remaining = intent words)
    let args: Vec<&String> = args.iter().filter(|a| {
        !(a.starts_with("-v") && a.chars().skip(1).all(|c| c == 'v'))
    }).collect();

    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            print_error(&format!("Config: {}", e));
            std::process::exit(1);
        }
    };

    let system = load_prompt();

    if args.is_empty() {
        run_interactive(&config, &system, verbosity);
    } else {
        let intent = args.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
        run_oneshot(&config, &system, &intent, verbosity);
    }
}

fn run_tests() {
    println!("Running placeholder tests...\n");
    let mut pass = 0;
    let mut fail = 0;

    let ph = collect_placeholders();
    let ctx = gather_context();

    // Helper
    let mut check = |name: &str, ok: bool| {
        if ok {
            println!("  ✓ {}", name);
            pass += 1;
        } else {
            println!("  ✗ {}", name);
            fail += 1;
        }
    };

    // Test 1: gather_context does NOT contain real username
    check(
        "context does not leak username",
        !ctx.contains(&ph.user),
    );

    // Test 2: gather_context does NOT contain real hostname
    check(
        "context does not leak hostname",
        !ctx.contains(&ph.hostname),
    );

    // Test 3: gather_context does NOT contain real home path
    check(
        "context does not leak home path",
        !ctx.contains(&ph.home),
    );

    // Test 4: apply_placeholders replaces {{USER}}
    let input = "cd /home/{{USER}}/docs";
    let output = apply_placeholders(input, &ph);
    let expected = format!("cd /home/{}/docs", ph.user);
    check(
        &format!("{{USER}} → {} ", ph.user),
        output == expected,
    );

    // Test 5: apply_placeholders replaces {{HOSTNAME}}
    let input = "ssh {{HOSTNAME}}";
    let output = apply_placeholders(input, &ph);
    let expected = format!("ssh {}", ph.hostname);
    check(
        &format!("{{HOSTNAME}} → {} ", ph.hostname),
        output == expected,
    );

    // Test 6: apply_placeholders replaces {{HOME}}
    let input = "ls {{HOME}}/projects";
    let output = apply_placeholders(input, &ph);
    let expected = format!("ls {}/projects", ph.home);
    check(
        &format!("{{HOME}} → {} ", ph.home),
        output == expected,
    );

    // Test 7: multiple placeholders in one string
    let input = "scp {{USER}}@{{HOSTNAME}}:{{HOME}}/file .";
    let output = apply_placeholders(input, &ph);
    let expected = format!("scp {}@{}:{}/file .", ph.user, ph.hostname, ph.home);
    check("multiple placeholders in one string", output == expected);

    // Test 8: no placeholders → unchanged
    let input = "ls -la";
    let output = apply_placeholders(input, &ph);
    check("no placeholders → unchanged", output == input);

    // Test 9: context contains non-private info
    check("context contains distro", ctx.contains("Distro:"));
    check("context contains kernel", ctx.contains("Kernel:"));
    check("context contains arch", ctx.contains("Arch:"));
    check("context contains shell", ctx.contains("Shell:"));
    check("context contains CWD", ctx.contains("CWD:"));
    check("context contains packages", ctx.contains("Installed packages"));

    // Test 10: retry constants are sane
    check("MAX_RETRIES >= 2", MAX_RETRIES >= 2);
    check("MAX_RETRIES <= 5", MAX_RETRIES <= 5);
    check("RETRY_HINT is non-empty", !RETRY_HINT.is_empty());

    // Test 11: #EXPLORE: prefix detection
    check("parse_explore: basic", parse_explore("#EXPLORE: openclaw --help") == Some("openclaw --help"));
    check("parse_explore: with spaces", parse_explore("  #EXPLORE: man ffmpeg  ") == Some("man ffmpeg"));
    check("parse_explore: no prefix", parse_explore("ls -la").is_none());
    check("parse_explore: partial prefix", parse_explore("#EXPLOR ls").is_none());
    check("parse_explore: just prefix", parse_explore("#EXPLORE:").is_none());

    // Summary
    println!("\n{} passed, {} failed", pass, fail);
    if fail > 0 {
        std::process::exit(1);
    }
}

fn print_help() {
    println!("Usage:");
    println!("  , <intent>   Generate shell command from natural language");
    println!("  ,            Interactive mode (refine commands with conversation)");
    println!("  , -h         Show this help");
    println!("  , -v         Verbose: show prompt and LLM reply");
    println!("  , -vv        Very verbose: add request logs and timing");
    println!();
    println!("Interactive commands:");
    println!("  x / exec     Execute the current command");
    println!("  c / copy     Copy current command to clipboard");
    println!("  q / quit     Exit");
    println!("  Tab          Complete filename from current directory");
    println!();
    println!("Config priority: ~/.local/bin/,.config.json > ~/.claude/settings.json");
    println!("Prompt file:     ~/.local/bin/,.prompt.md");
    println!();
    println!("API style (api_style):");
    println!("  openai       OpenAI-compatible (Cerebras, Groq, Ollama, vLLM, ...)");
    println!("  anthropic    Anthropic Messages API");
    println!("  (auto-detected from URL if omitted; anthropic URLs → anthropic, rest → openai)");
}

fn run_oneshot(config: &Config, system: &str, intent: &str, v: Verbosity) {
    let messages = vec![Message {
        role: "user".into(),
        content: intent.to_string(),
    }];
    let ph = collect_placeholders();

    print_info(&format!("{} ({})", config.model, style_label(config.api_style)));
    if v.show_prompt() {
        print_debug(&format!("System prompt:\n{}", truncate(system, 1000)));
        print_debug(&format!("User: {}", intent));
    }
    match call_llm_with_retry(config, system, &messages, v) {
        Ok(raw) => {
            // Try exploration: if LLM returned a help command, learn from it
            let final_raw = match explore_then_generate(config, system, &messages, &raw, &ph, v) {
                Ok(Some(real_cmd)) => real_cmd,
                Ok(None) => raw,
                Err(e) => {
                    print_error(&format!("Explore: {}", e));
                    raw
                }
            };
            let cmd = apply_placeholders(&final_raw, &ph);
            print_cmd(&cmd);
            if is_dangerous(&cmd) {
                if prompt_confirm("Execute this dangerous command?") {
                    execute(&cmd);
                }
            } else if prompt_confirm("Execute?") {
                execute(&cmd);
            }
        }
        Err(e) => print_error(&e),
    }
}

fn run_interactive(config: &Config, system: &str, v: Verbosity) {
    print_info(&format!(
        "{} ({}). Tab completes filenames. 'q' to quit, 'x' to exec, 'c' to copy.",
        config.model,
        style_label(config.api_style),
    ));

    let ph = collect_placeholders();

    let mut rl = Editor::<FileHelper, DefaultHistory>::new().ok();
    if let Some(ref mut editor) = rl {
        editor.set_helper(Some(FileHelper::new()));
        editor.set_completion_type(rustyline::CompletionType::List);
    }

    let mut messages: Vec<Message> = Vec::new();
    let mut current_cmd = String::new();

    loop {
        let input = match rl.as_mut() {
            Some(editor) => prompt_input(editor),
            None => prompt_input_fallback(),
        };
        match input {
            None => continue,
            Some(input) => {
                if input == "q" || input == "quit" || input == "exit" {
                    break;
                }

                if input == "x" || input == "exec" {
                    if current_cmd.is_empty() {
                        print_error("No command to execute.");
                        continue;
                    }
                    if is_dangerous(&current_cmd) {
                        if prompt_confirm("Execute this dangerous command?") {
                            execute(&current_cmd);
                        }
                    } else {
                        execute(&current_cmd);
                    }
                    continue;
                }

                if input == "c" || input == "copy" {
                    if current_cmd.is_empty() {
                        print_error("No command to copy.");
                    } else {
                        copy_to_clipboard(&current_cmd);
                        print_info("Copied to clipboard.");
                    }
                    continue;
                }

                messages.push(Message {
                    role: "user".into(),
                    content: input,
                });

                print_info("Thinking...");
                if v.show_prompt() {
                    print_debug(&format!("User: {}", messages.last().unwrap().content));
                }
                match call_llm_with_retry(config, system, &messages, v) {
                    Ok(raw) => {
                        // Try exploration: if LLM returned a help command, learn from it
                        let final_raw = match explore_then_generate(config, system, &messages, &raw, &ph, v) {
                            Ok(Some(real_cmd)) => real_cmd,
                            Ok(None) => raw,
                            Err(e) => {
                                print_error(&format!("Explore: {}", e));
                                raw
                            }
                        };
                        let cmd = apply_placeholders(&final_raw, &ph);
                        current_cmd = cmd.clone();
                        print_cmd(&current_cmd);
                        messages.push(Message {
                            role: "assistant".into(),
                            content: final_raw, // keep raw in conversation
                        });
                    }
                    Err(e) => {
                        print_error(&e);
                        messages.pop();
                    }
                }
            }
        }
    }
}

fn style_label(style: ApiStyle) -> &'static str {
    match style {
        ApiStyle::OpenAI => "openai",
        ApiStyle::Anthropic => "anthropic",
    }
}

fn execute(cmd: &str) {
    print_info(&format!("Running: {}", cmd));
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status();
    match status {
        Ok(s) if !s.success() => {
            print_error(&format!("Exit code: {}", s.code().unwrap_or(-1)));
        }
        Err(e) => print_error(&format!("Failed to execute: {}", e)),
        _ => {}
    }
}

fn copy_to_clipboard(text: &str) {
    let tools: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
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
    print_error("No clipboard tool found (install wl-clipboard, xclip, or xsel).");
}
