use crossterm::style::{Color, ResetColor, SetForegroundColor};
use rustyline::completion::{Completer, FilenameCompleter};
use rustyline::config::Configurer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
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

// ── Edit action ─────────────────────────────────────────────────────────────

enum EditAction {
    Execute(String),
    Refine(String),
    Cancel,
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
struct ProviderConfig {
    base_url: Option<String>,
    auth_token: Option<String>,
    api_style: Option<String>,
}

#[derive(Deserialize)]
struct LocalModelEntry {
    provider: String,
    model: String,
    retries: Option<usize>,
}

#[derive(Clone)]
struct ModelEntry {
    base_url: String,
    auth_token: String,
    model: String,
    api_style: ApiStyle,
    retries: usize,
}

#[derive(Deserialize, Default)]
struct LocalConfig {
    // Legacy single-model format (still supported)
    base_url: Option<String>,
    auth_token: Option<String>,
    model: Option<String>,
    api_style: Option<String>,
    // New multi-provider format
    providers: Option<HashMap<String, ProviderConfig>>,
    models: Option<Vec<LocalModelEntry>>,
    // Shared settings
    prefer: Option<HashMap<String, Vec<String>>>,
    cache_size: Option<usize>,
    reasoning: Option<u32>,
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
    entries: Vec<ModelEntry>,
    prefer: HashMap<String, Vec<String>>,
    cache_size: usize,
    reasoning: u32,
}

impl Config {
    fn primary(&self) -> &ModelEntry {
        &self.entries[0]
    }
    fn model(&self) -> &str {
        &self.primary().model
    }
    fn api_style(&self) -> ApiStyle {
        self.primary().api_style
    }
}

fn home_dir() -> Result<String, String> {
    std::env::var("HOME").map_err(|_| "HOME not set".into())
}

fn load_config() -> Result<Config, String> {
    let home = home_dir()?;

    // Read config files
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
    let env_or = |key: &str| non_empty(std::env::var(key).ok());

    let prefer = local.prefer.unwrap_or_default();
    let cache_size = local.cache_size.unwrap_or(1000);
    let reasoning = local.reasoning.unwrap_or(0);

    // Build model entries
    // Priority: COMMA_* env > ,.config.json legacy > ,.config.json providers/models > claude settings
    let entries = if let Some(models) = local.models {
        // New providers/models format — provider fields are required, no claude fallback
        let providers = local.providers.unwrap_or_default();
        let mut entries = Vec::new();
        for m in models {
            let p = providers.get(&m.provider)
                .ok_or(format!("Provider '{}' not found in providers", m.provider))?;
            let base_url = env_or("COMMA_BASE_URL")
                .or_else(|| non_empty(p.base_url.clone()))
                .ok_or(format!("Provider '{}' missing base_url", m.provider))?;
            let auth_token = env_or("COMMA_API_KEY")
                .or_else(|| non_empty(p.auth_token.clone()))
                .ok_or(format!("Provider '{}' missing auth_token", m.provider))?;
            let api_style = env_or("COMMA_API_STYLE")
                .and_then(|s| ApiStyle::from_str(&s))
                .or_else(|| non_empty(p.api_style.clone()).and_then(|s| ApiStyle::from_str(&s)))
                .unwrap_or_else(|| ApiStyle::from_url(&base_url));
            let model = env_or("COMMA_MODEL")
                .unwrap_or(m.model.clone());
            entries.push(ModelEntry {
                base_url,
                auth_token,
                model,
                api_style,
                retries: m.retries.unwrap_or(1),
            });
        }
        if entries.is_empty() {
            return Err("models list is empty".into());
        }
        entries
    } else {
        // Legacy single-model format — falls back to claude settings
        let base_url = env_or("COMMA_BASE_URL")
            .or_else(|| non_empty(local.base_url.clone()))
            .or_else(|| claude_env.as_ref().and_then(|e| e.base_url.clone()))
            .unwrap_or_else(|| "https://api.anthropic.com".into());
        let auth_token = env_or("COMMA_API_KEY")
            .or_else(|| non_empty(local.auth_token.clone()))
            .or_else(|| claude_env.as_ref().and_then(|e| e.auth_token.clone()))
            .ok_or("No auth_token: set in ,.config.json or ANTHROPIC_AUTH_TOKEN in ~/.claude/settings.json")?;
        let model = env_or("COMMA_MODEL")
            .or_else(|| non_empty(local.model.clone()))
            .or_else(|| claude_env.as_ref().and_then(|e| e.model.clone()))
            .unwrap_or_else(|| "claude-sonnet-4-20250514".into());
        let api_style = env_or("COMMA_API_STYLE")
            .and_then(|s| ApiStyle::from_str(&s))
            .or_else(|| non_empty(local.api_style.clone()).and_then(|s| ApiStyle::from_str(&s)))
            .unwrap_or_else(|| ApiStyle::from_url(&base_url));
        vec![ModelEntry {
            base_url,
            auth_token,
            model,
            api_style,
            retries: MAX_RETRIES,
        }]
    };

    Ok(Config {
        entries,
        prefer,
        cache_size,
        reasoning,
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
    let mut sections: Vec<String> = Vec::new();

    // Detect package manager
    let managers: &[&str] = &["apt", "dnf", "yum", "pacman", "apk", "xbps-install", "zypper", "eopkg"];
    let pkg_mgr = managers.iter().find(|m| run_cmd("which", &[m]).is_some());
    if let Some(mgr) = pkg_mgr {
        sections.push(format!("[Package manager: {}]", mgr));
    }

    // List user-installed packages (non-auto, not part of base system)
    // This is much smaller than listing all PATH executables.
    let user_pkgs = get_user_packages();
    if !user_pkgs.is_empty() {
        sections.push(format!("[User-installed packages: {}]", user_pkgs.join(", ")));
    }

    sections.join("\n")
}

/// Get packages explicitly installed by the user (not auto-installed deps).
fn get_user_packages() -> Vec<String> {
    // Try apt-mark showmanual (Debian/Ubuntu)
    if let Some(output) = run_cmd("apt-mark", &["showmanual"]) {
        let pkgs: Vec<String> = output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if !pkgs.is_empty() {
            return pkgs;
        }
    }
    // Try dnf/yum (RHEL/Fedora)
    if let Some(output) = run_cmd("dnf", &["repoquery", "--userinstalled", "--qf", "%{name}"]) {
        let pkgs: Vec<String> = output.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if !pkgs.is_empty() {
            return pkgs;
        }
    }
    // Try pacman (Arch)
    if let Some(output) = run_cmd("pacman", &["-Qe"]) {
        let pkgs: Vec<String> = output
            .lines()
            .filter_map(|l| l.split_whitespace().next().map(|s| s.to_string()))
            .collect();
        if !pkgs.is_empty() {
            return pkgs;
        }
    }
    Vec::new()
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

fn load_prompt(config: &Config) -> String {
    let home = home_dir().unwrap_or_default();
    let path = PathBuf::from(&home).join(".local/bin/,.prompt.md");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_PROMPT.into());

    let ctx = gather_context();
    let prefs = format_preferences(&config.prefer);

    raw.replace("{{SYSTEM_CONTEXT}}", &ctx)
        .replace("{{PREFERENCES}}", &prefs)
}

fn format_preferences(prefer: &HashMap<String, Vec<String>>) -> String {
    if prefer.is_empty() {
        return "(none configured)".to_string();
    }
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

const DEFAULT_PROMPT: &str = r#"You are a shell command generator. The user describes intent in natural language; you output the corresponding shell command.

Rules:
- Output exactly ONE shell command that can be executed directly. No explanations.
- The command should be concise, general-purpose, and correct for the user's platform (see system context below).
- If the intent is ambiguous, output the most reasonable default.
- If the intent cannot be achieved in one command, output the closest command with a # comment noting the limitation.
- Output ONLY the command, nothing else. No markdown fences, no prose.
- Tailor commands to the installed package manager and available tools.
- Respect the user's tool preferences below. Use their preferred tools when possible.
- ALWAYS append a short # comment after the command explaining what it does (in the user's language).
  Example: find . -name "*.log" -delete # Delete all .log files recursively
  For ||| candidates, each candidate gets its own comment.
  Keep comments concise (one line, under 60 chars).

Multiple candidates:
When there are genuinely different approaches (e.g. different tools or styles), you may output up to 3 alternatives separated by |||.
Example: ls -la # List all files ||| exa -la # Modern ls with colors ||| eza -la --icons # ls with icons
The user will pick one. Only use ||| when alternatives are meaningfully different.
If there's one clear best command, output it alone without |||.

Tool discovery:
When you recommend a command, consider which tools are BEST for the job.
If you are unsure what's installed, use #CHECK: followed by candidate tool names.
Example: #CHECK: ripgrep fd bat jq yq
The tool will report which are available, then you generate the final command.
If you need to learn a tool's flags, use #EXPLORE: <help-cmd>.

IMPORTANT: When the user mentions a specific tool by name (e.g. "openclaw", "ffmpeg", "rg"),
and you are NOT 100% certain about its exact usage/flags/subcommands, use #EXPLORE: to learn it first.
Example: #EXPLORE: openclaw --help
NEVER assume a tool's package manager (pip, npm, cargo, etc.) without verifying.
Always explore unfamiliar tools before suggesting install or usage commands.

User tool preferences (ordered by preference, leftmost is most preferred):
{{PREFERENCES}}

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

#[derive(Default, Debug)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
    cache_read: u32,
    cache_creation: u32,
    total_tokens: u32,
    duration_ms: u64,
    from_cache: bool,
}

struct LlmResponse {
    content: String,
    usage: Usage,
    cache_key: Option<String>,
}

// ── Response cache ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    content: String,
    usage: CacheUsage,
    ts: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct CacheUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_read: u32,
    cache_creation: u32,
    total_tokens: u32,
}

struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_size: usize,
    path: PathBuf,
    dirty: bool,
}

fn cache_key(model: &str, system: &str, messages: &[Message]) -> String {
    let mut h = DefaultHasher::new();
    model.hash(&mut h);
    system.hash(&mut h);
    for m in messages {
        m.role.hash(&mut h);
        m.content.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

impl ResponseCache {
    fn load(max_size: usize) -> Self {
        let home = home_dir().unwrap_or_default();
        let path = PathBuf::from(&home).join(".local/bin/,.cache.json");
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str::<HashMap<String, CacheEntry>>(&data).ok())
            .unwrap_or_default();
        Self {
            entries,
            max_size,
            path,
            dirty: false,
        }
    }

    fn get(&self, key: &str) -> Option<&CacheEntry> {
        self.entries.get(key)
    }

    fn put(&mut self, key: String, entry: CacheEntry) {
        self.entries.insert(key, entry);
        self.dirty = true;
        // Evict oldest if over capacity
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

    fn save(&self) {
        if !self.dirty {
            return;
        }
        if let Ok(json) = serde_json::to_string(&self.entries) {
            let _ = std::fs::write(&self.path, json);
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl From<&LlmResponse> for CacheEntry {
    fn from(resp: &LlmResponse) -> Self {
        Self {
            content: resp.content.clone(),
            usage: CacheUsage {
                input_tokens: resp.usage.input_tokens,
                output_tokens: resp.usage.output_tokens,
                cache_read: resp.usage.cache_read,
                cache_creation: resp.usage.cache_creation,
                total_tokens: resp.usage.total_tokens,
            },
            ts: now_ts(),
        }
    }
}

impl CacheEntry {
    fn to_response(&self) -> LlmResponse {
        LlmResponse {
            content: self.content.clone(),
            usage: Usage {
                input_tokens: self.usage.input_tokens,
                output_tokens: self.usage.output_tokens,
                cache_read: self.usage.cache_read,
                cache_creation: self.usage.cache_creation,
                total_tokens: self.usage.total_tokens,
                duration_ms: 0,
                from_cache: true,
            },
            cache_key: None,
        }
    }
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
    usage: Option<OpenAiUsage>,
    error: Option<OpenAiError>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
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
struct ThinkingConfig {
    #[serde(rename = "type")]
    thinking_type: String,
    budget_tokens: u32,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Option<Vec<AnthropicContentBlock>>,
    usage: Option<AnthropicUsage>,
    error: Option<AnthropicApiError>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    cache_creation_input_tokens: Option<u32>,
    cache_read_input_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: Option<String>,
    text: Option<String>,
    thinking: Option<String>,
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

fn call_llm(
    entry: &ModelEntry,
    system: &str,
    messages: &[Message],
    v: Verbosity,
    cache: &ResponseCache,
    reasoning: u32,
) -> Result<LlmResponse, String> {
    let key = cache_key(&entry.model, system, messages);

    // Check cache — only return non-empty cached responses
    if let Some(cached) = cache.get(&key) {
        if !cached.content.is_empty() {
            if v.show_debug() {
                print_debug(&format!("Cache hit: {}", &key[..8]));
            }
            let mut resp = cached.to_response();
            resp.cache_key = Some(key);
            return Ok(resp);
        }
    }

    // Call API
    if v.show_debug() {
        print_debug(&format!("Cache miss: {}", &key[..8]));
    }
    let mut result = match entry.api_style {
        ApiStyle::OpenAI => call_openai(entry, system, messages, v),
        ApiStyle::Anthropic => call_anthropic(entry, system, messages, v, reasoning),
    };

    // Attach cache key (caller decides whether to store)
    if let Ok(ref mut resp) = result {
        resp.cache_key = Some(key);
    }

    result
}

fn print_usage(u: &Usage) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(out, "{}", SetForegroundColor(Color::DarkGrey));
    if u.from_cache {
        let _ = write!(out, "  tokens: {}in + {}out (from cache)", u.input_tokens, u.output_tokens);
    } else {
        let total = if u.total_tokens > 0 {
            u.total_tokens
        } else {
            u.input_tokens + u.output_tokens
        };
        let _ = write!(out, "  tokens: {}in + {}out = {}", u.input_tokens, u.output_tokens, total);
        if u.cache_read > 0 {
            let _ = write!(out, " (cached: {})", u.cache_read);
        }
        if u.cache_creation > 0 {
            let _ = write!(out, " (cache_write: {})", u.cache_creation);
        }
        let _ = write!(out, " | {}ms", u.duration_ms);
    }
    let _ = write!(out, "{}", ResetColor);
    let _ = writeln!(out);
}

/// Call LLM with retry on empty response. Up to MAX_RETRIES attempts.
fn call_llm_with_retry(
    config: &Config,
    system: &str,
    messages: &[Message],
    v: Verbosity,
    cache: &ResponseCache,
) -> Result<LlmResponse, String> {
    let mut last_err = String::new();
    for (idx, entry) in config.entries.iter().enumerate() {
        if idx > 0 {
            print_info(&format!("Trying fallback: {} ({})...", entry.model, style_label(entry.api_style)));
        }
        for attempt in 0..entry.retries {
            let result = call_llm(entry, system, messages, v, cache, config.reasoning);
            match result {
                Ok(resp) if !resp.content.is_empty() => return Ok(resp),
                Ok(_) => {
                    // Empty response — retry with hint
                    if attempt + 1 < entry.retries {
                        print_info(&format!(
                            "Empty response from {}, retrying ({}/{})...",
                            entry.model, attempt + 1, entry.retries
                        ));
                        let mut retry_msgs = messages.to_vec();
                        retry_msgs.push(Message {
                            role: "assistant".into(),
                            content: "(no response)".to_string(),
                        });
                        retry_msgs.push(Message {
                            role: "user".into(),
                            content: RETRY_HINT.to_string(),
                        });
                        let retry_result = call_llm(entry, system, &retry_msgs, v, cache, config.reasoning);
                        if let Ok(resp) = retry_result {
                            if !resp.content.is_empty() {
                                return Ok(resp);
                            }
                        }
                    }
                }
                Err(e) => {
                    last_err = e;
                    print_info(&format!("{} failed: {}", entry.model, last_err));
                    break; // Move to next model entry
                }
            }
        }
    }
    if last_err.is_empty() {
        Err("All models returned empty responses.".into())
    } else {
        Err(format!("All models failed. Last error: {}", last_err))
    }
}

fn make_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP client: {}", e))
}

fn call_openai(entry: &ModelEntry, system: &str, messages: &[Message], v: Verbosity) -> Result<LlmResponse, String> {
    let base = normalize_base_url(&entry.base_url);
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
        model: entry.model.clone(),
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
        .header("Authorization", format!("Bearer {}", entry.auth_token))
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

    let usage = api_resp.usage.as_ref().map(|u| Usage {
        input_tokens: u.prompt_tokens.unwrap_or(0),
        output_tokens: u.completion_tokens.unwrap_or(0),
        total_tokens: u.total_tokens.unwrap_or(0),
        duration_ms: elapsed.as_millis() as u64,
        ..Usage::default()
    }).unwrap_or(Usage { duration_ms: elapsed.as_millis() as u64, ..Usage::default() });

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

    Ok(LlmResponse { content: content.to_string(), usage, cache_key: None })
}

fn call_anthropic(entry: &ModelEntry, system: &str, messages: &[Message], v: Verbosity, reasoning: u32) -> Result<LlmResponse, String> {
    let base = normalize_base_url(&entry.base_url);
    let url = format!("{}/v1/messages", base);

    let thinking = if reasoning > 0 {
        Some(ThinkingConfig {
            thinking_type: "enabled".to_string(),
            budget_tokens: reasoning,
        })
    } else {
        None
    };

    let body = AnthropicRequest {
        model: entry.model.clone(),
        max_tokens: 1024,
        system: system.to_string(),
        messages: messages.to_vec(),
        thinking,
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
        .header("x-api-key", &entry.auth_token)
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

    let usage = api_resp.usage.as_ref().map(|u| Usage {
        input_tokens: u.input_tokens.unwrap_or(0),
        output_tokens: u.output_tokens.unwrap_or(0),
        cache_read: u.cache_read_input_tokens.unwrap_or(0),
        cache_creation: u.cache_creation_input_tokens.unwrap_or(0),
        duration_ms: elapsed.as_millis() as u64,
        ..Usage::default()
    }).unwrap_or(Usage { duration_ms: elapsed.as_millis() as u64, ..Usage::default() });

    let content = api_resp.content.ok_or("Empty response")?;

    // Show thinking blocks in verbose mode
    if v.show_prompt() {
        for block in &content {
            if block.block_type.as_deref() == Some("thinking") {
                if let Some(ref t) = block.thinking {
                    print_debug(&format!("Thinking:\n{}", truncate(t, 500)));
                }
            }
        }
    }

    // Only use text blocks for the response
    let result: String = content
        .iter()
        .filter(|b| b.block_type.as_deref().unwrap_or("text") == "text")
        .filter_map(|b| b.text.as_deref())
        .collect::<Vec<_>>()
        .join("");
    let trimmed = result.trim();

    if v.show_prompt() {
        print_debug(&format!("LLM reply: {}", trimmed));
    }

    Ok(LlmResponse { content: trimmed.to_string(), usage, cache_key: None })
}

// ── #CHECK: tool availability query ─────────────────────────────────────────

const CHECK_PREFIX: &str = "#CHECK:";

const CHECK_HINT: &str = "\
Here is which tools are available on this system. \
Now generate the best shell command using what's actually installed. \
Output ONLY the final command. Do NOT prefix with #CHECK: or #EXPLORE:.";

/// If raw starts with `#CHECK:`, extract the tool names.
fn parse_check(raw: &str) -> Option<Vec<&str>> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix(CHECK_PREFIX)?.trim();
    if rest.is_empty() {
        return None;
    }
    // Strip # comment before parsing tool names
    let (tool_str, _) = split_comment(rest);
    let tools: Vec<&str> = tool_str.split_whitespace().collect();
    if tools.is_empty() {
        None
    } else {
        Some(tools)
    }
}

/// Check which tools are available, return a report string.
fn check_tools(tools: &[&str]) -> String {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for tool in tools {
        if run_cmd("which", &[tool]).is_some() {
            found.push(*tool);
        } else {
            missing.push(*tool);
        }
    }
    let mut parts = Vec::new();
    if !found.is_empty() {
        parts.push(format!("Available: {}", found.join(", ")));
    }
    if !missing.is_empty() {
        parts.push(format!("Not found: {}", missing.join(", ")));
    }
    parts.join("\n")
}

/// If the model returned `#CHECK: t1 t2 t3`, check availability,
/// feed results back to the LLM, and return the real command.
fn check_then_generate(
    config: &Config,
    system: &str,
    messages: &[Message],
    raw: &str,
    v: Verbosity,
    cache: &ResponseCache,
) -> Result<Option<String>, String> {
    let tools = match parse_check(raw) {
        Some(t) => t,
        None => return Ok(None),
    };

    print_info(&format!("Checking tools: {}", tools.join(", ")));
    let report = check_tools(&tools);
    print_info(&report);

    let mut ext = messages.to_vec();
    ext.push(Message {
        role: "assistant".into(),
        content: raw.to_string(),
    });
    ext.push(Message {
        role: "user".into(),
        content: format!("{}\n\nTool availability:\n{}", CHECK_HINT, report),
    });

    let resp = call_llm_with_retry(config, system, &ext, v, cache)?;
    Ok(Some(resp.content))
}

// ── Exploration: #EXPLORE: prefix ───────────────────────────────────────────

const EXPLORE_PREFIX: &str = "#EXPLORE:";

const EXPLORE_HINT: &str = "\
The command output is shown above. You have already explored this tool. \
DO NOT use #EXPLORE: or #CHECK: again. \
Now generate the FINAL shell command the user originally wanted. \
Output ONLY the command, nothing else.";

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

/// Chain: #CHECK → #EXPLORE → final command.
/// #CHECK can loop (to handle #CHECK after #EXPLORE), but #EXPLORE runs only once.
fn process_response(
    config: &Config,
    system: &str,
    messages: &[Message],
    raw: &str,
    ph: &Placeholders,
    v: Verbosity,
    cache: &ResponseCache,
) -> String {
    let mut current = raw.to_string();
    let mut explored = false;

    for _ in 0..5 {
        let after_check = match check_then_generate(config, system, messages, &current, v, cache) {
            Ok(Some(cmd)) => cmd,
            Ok(None) => current.clone(),
            Err(e) => {
                print_error(&format!("Check: {}", e));
                current.clone()
            }
        };

        if explored {
            // Already explored once, stop here
            return after_check;
        }

        match explore_then_generate(config, system, messages, &after_check, ph, v, cache) {
            Ok(Some(cmd)) => {
                explored = true;
                current = cmd;
            }
            Ok(None) => {
                // Explore was attempted (or not applicable), mark as explored
                if parse_explore(&after_check).is_some() {
                    explored = true;
                }
                if after_check == current {
                    return current; // No change from either step
                }
                current = after_check;
            }
            Err(e) => {
                print_error(&format!("Explore: {}", e));
                return after_check;
            }
        }
    }
    current
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
    cache: &ResponseCache,
) -> Result<Option<String>, String> {
    // Handle multiple #EXPLORE candidates separated by |||
    let candidates: Vec<&str> = raw.split("|||")
        .map(|s| s.trim())
        .filter(|s| parse_explore(s).is_some())
        .collect();

    let explore_cmds: Vec<&str> = if candidates.len() > 1 {
        // Multiple explore candidates — show them and ask to run all
        print_info("Model wants to explore:");
        for c in &candidates {
            print_cmd(parse_explore(c).unwrap_or(c));
        }
        if !prompt_confirm("Run all to learn usage?") {
            return Ok(None);
        }
        candidates.iter()
            .map(|c| parse_explore(c).unwrap_or(c))
            .collect()
    } else {
        match parse_explore(raw) {
            Some(cmd) => vec![cmd],
            None => return Ok(None),
        }
    };

    // Run all explore commands and collect outputs
    let mut all_output = String::new();
    for cmd_str in &explore_cmds {
        let cmd = apply_placeholders(cmd_str, ph);
        print_info(&format!("Exploring: {}", cmd));
        match run_and_capture(&cmd) {
            Ok(output) => {
                if !output.trim().is_empty() {
                    if !all_output.is_empty() {
                        all_output.push_str("\n\n");
                    }
                    all_output.push_str(&format!("$ {}\n{}", cmd, output));
                }
            }
            Err(e) => {
                print_error(&format!("Explore failed: {}", e));
            }
        }
    }

    if all_output.trim().is_empty() {
        print_info("No output from explore commands.");
        return Ok(None);
    }

    if v.show_debug() {
        print_debug(&format!(
            "Captured ({} chars):\n{}",
            all_output.len(),
            truncate(&all_output, 1000)
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
        content: format!("{}\n\nCommand output:\n```\n{}\n```", EXPLORE_HINT, all_output),
    });

    let resp = call_llm_with_retry(config, system, &ext, v, cache)?;
    Ok(Some(resp.content))
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
    let (command, _) = split_comment(cmd);
    let lower = command.to_lowercase();
    DANGER_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

// ── Display helpers ─────────────────────────────────────────────────────────

/// Split "command # comment" into (command, Some(comment)) or (cmd, None).
/// Handles cases where # appears inside quotes or is the comment marker.
fn split_comment(raw: &str) -> (&str, Option<&str>) {
    // Find the first unquoted #
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

fn print_cmd(cmd: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let (command, comment) = split_comment(cmd);

    if is_dangerous(command) {
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
        command,
        ResetColor
    );
    if let Some(cmt) = comment {
        let _ = write!(
            out,
            "  {}# {}{}",
            SetForegroundColor(Color::DarkGrey),
            cmt,
            ResetColor
        );
    }
    let _ = writeln!(out);
}

/// Split LLM output by ||| delimiter into candidate commands.
fn parse_candidates(raw: &str) -> Vec<String> {
    let candidates: Vec<String> = raw
        .split("|||")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if candidates.is_empty() {
        vec![raw.trim().to_string()]
    } else {
        candidates
    }
}

/// Interactive selector for multiple candidates.
/// Returns the index of the selected candidate, or None if cancelled.
fn select_command(candidates: &[String]) -> Option<usize> {
    if candidates.len() <= 1 {
        return Some(0);
    }

    // Non-interactive (piped): just pick the first candidate
    if !atty::is(atty::Stream::Stdin) {
        print_cmd(&candidates[0]);
        return Some(0);
    }

    let mut selected: usize = 0;

    // Print initial candidates, save cursor row
    draw_candidates(candidates, selected);
    let _ = io::stdout().flush();

    // Get cursor position AFTER printing — this is the row below the last candidate
    let (_, end_row) = crossterm::cursor::position().unwrap_or((0, 0));
    let start_row = end_row.saturating_sub(candidates.len() as u16);

    let _ = crossterm::terminal::enable_raw_mode();

    let result = loop {
        if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if selected > 0 {
                        selected -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected < candidates.len() - 1 {
                        selected += 1;
                    }
                }
                KeyCode::Tab => {
                    selected = (selected + 1) % candidates.len();
                }
                KeyCode::BackTab => {
                    selected = if selected == 0 {
                        candidates.len() - 1
                    } else {
                        selected - 1
                    };
                }
                KeyCode::Enter => {
                    let _ = crossterm::execute!(
                        io::stdout(),
                        crossterm::cursor::MoveTo(0, start_row),
                        crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
                    );
                    let _ = crossterm::terminal::disable_raw_mode();
                    return Some(selected);
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Esc | KeyCode::Char('q') => break None,
                _ => {}
            }
            // Move to saved row, column 0, clear and redraw
            let _ = crossterm::execute!(
                io::stdout(),
                crossterm::cursor::MoveTo(0, start_row),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
            );
            draw_candidates(candidates, selected);
            let _ = io::stdout().flush();
        }
    };

    let _ = crossterm::terminal::disable_raw_mode();
    result
}

fn draw_candidates(candidates: &[String], selected: usize) {
    let mut out = io::stdout().lock();
    for (i, cmd) in candidates.iter().enumerate() {
        let (command, comment) = split_comment(cmd);
        let marker = if i == selected { "▸" } else { " " };
        let color = if is_dangerous(command) {
            Color::Red
        } else if i == selected {
            Color::Green
        } else {
            Color::DarkGrey
        };
        let _ = write!(out, "\r{}{} ", SetForegroundColor(Color::Cyan), marker);
        let _ = write!(out, "{}{}{}", SetForegroundColor(color), command, ResetColor);
        if let Some(cmt) = comment {
            let _ = write!(out, "  {}# {}{}", SetForegroundColor(Color::DarkGrey), cmt, ResetColor);
        }
        if is_dangerous(command) {
            let _ = write!(out, " {}⚠{}", SetForegroundColor(Color::Red), ResetColor);
        }
        let _ = writeln!(out);
    }
    let _ = out.flush();
}

// ── Spinner ─────────────────────────────────────────────────────────────────

struct Spinner {
    handle: Option<std::thread::JoinHandle<()>>,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Spinner {
    fn start(msg: &str) -> Self {
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let msg = msg.to_string();
        let running_clone = running.clone();

        let handle = std::thread::spawn(move || {
            let mut i = 0;
            while running_clone.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = crossterm::execute!(
                    io::stdout(),
                    crossterm::cursor::SavePosition,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                );
                let _ = write!(
                    io::stdout(),
                    "\r{}{} {}{}",
                    SetForegroundColor(Color::Cyan),
                    frames[i % frames.len()],
                    msg,
                    ResetColor,
                );
                let _ = io::stdout().flush();
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            // Clear the spinner line
            let _ = crossterm::execute!(
                io::stdout(),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            );
            let _ = write!(io::stdout(), "\r");
            let _ = io::stdout().flush();
        });

        Self {
            handle: Some(handle),
            running,
        }
    }

    fn stop(&mut self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
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
        "{}{}{} [Ctrl+Enter/N] ",
        SetForegroundColor(Color::Yellow),
        msg,
        ResetColor
    );
    let _ = out.flush();
    drop(out);

    // Fallback to line-based input when stdin is not a TTY (piped)
    if !atty::is(atty::Stream::Stdin) {
        let mut input = String::new();
        return io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y");
    }

    let _ = crossterm::terminal::enable_raw_mode();
    let result = loop {
        if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
            match code {
                KeyCode::Enter if modifiers.contains(KeyModifiers::CONTROL) => break true,
                KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                _ => break false,
            }
        }
    };
    let _ = crossterm::terminal::disable_raw_mode();
    result
}

fn edit_or_execute(cmd: &str, rl: &mut Editor<FileHelper, DefaultHistory>) -> EditAction {
    print_cmd(cmd);

    if !atty::is(atty::Stream::Stdin) {
        return EditAction::Execute(cmd.to_string());
    }

    let prompt_text = if is_dangerous(cmd) {
        "Execute this dangerous command? [Ctrl+Enter] exec / [e]dit / [r]efine / [Enter] cancel "
    } else {
        "Execute? [Ctrl+Enter] exec / [e]dit / [r]efine / [Enter] cancel "
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(
        out,
        "{}{}{}",
        SetForegroundColor(Color::Yellow),
        prompt_text,
        ResetColor
    );
    let _ = out.flush();
    drop(out);

    let _ = crossterm::terminal::enable_raw_mode();
    let action = loop {
        if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
            match code {
                KeyCode::Enter if modifiers.contains(KeyModifiers::CONTROL) => {
                    break EditAction::Execute(cmd.to_string());
                }
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    break EditAction::Execute(cmd.to_string());
                }
                KeyCode::Char('e') => {
                    let _ = crossterm::terminal::disable_raw_mode();
                    let edit_prompt = format!("{}edit> {}", SetForegroundColor(Color::Yellow), ResetColor);
                    match rl.readline_with_initial(&edit_prompt, (cmd, "")) {
                        Ok(edited) => {
                            let trimmed = edited.trim().to_string();
                            if trimmed.is_empty() || trimmed == cmd {
                                break EditAction::Execute(cmd.to_string());
                            }
                            let _ = rl.add_history_entry(&trimmed);
                            break EditAction::Execute(trimmed);
                        }
                        Err(_) => break EditAction::Cancel,
                    }
                }
                KeyCode::Char('r') => {
                    let _ = crossterm::terminal::disable_raw_mode();
                    let refine_prompt = format!("{}refine> {}", SetForegroundColor(Color::Yellow), ResetColor);
                    match rl.readline(&refine_prompt) {
                        Ok(text) => {
                            let trimmed = text.trim().to_string();
                            if trimmed.is_empty() {
                                break EditAction::Cancel;
                            }
                            let _ = rl.add_history_entry(&trimmed);
                            break EditAction::Refine(trimmed);
                        }
                        Err(_) => break EditAction::Cancel,
                    }
                }
                _ => break EditAction::Cancel,
            }
        }
    };
    let _ = crossterm::terminal::disable_raw_mode();
    action
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

    let system = load_prompt(&config);

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

    // Test 12: #CHECK: prefix detection
    check("parse_check: basic", parse_check("#CHECK: ripgrep fd bat") == Some(vec!["ripgrep", "fd", "bat"]));
    check("parse_check: single", parse_check("#CHECK: jq") == Some(vec!["jq"]));
    check("parse_check: no prefix", parse_check("ls -la").is_none());
    check("parse_check: just prefix", parse_check("#CHECK:").is_none());

    // Test 13: parse_candidates
    let c = parse_candidates("ls -la ||| exa -la ||| eza -la");
    check("parse_candidates: 3 items", c.len() == 3);
    check("parse_candidates: first", c[0] == "ls -la");
    check("parse_candidates: second", c[1] == "exa -la");
    check("parse_candidates: third", c[2] == "eza -la");
    let c2 = parse_candidates("ls -la");
    check("parse_candidates: single", c2.len() == 1);
    check("parse_candidates: single value", c2[0] == "ls -la");
    let c3 = parse_candidates("  ls -la  |||  exa -la  ");
    check("parse_candidates: trims", c3[0] == "ls -la" && c3[1] == "exa -la");

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
    println!("Config priority: COMMA_* env > ,.config.json > claude settings");
    println!("Prompt file:     ~/.local/bin/,.prompt.md");
    println!();
    println!("API style (api_style):");
    println!("  openai       OpenAI-compatible (Cerebras, Groq, Ollama, vLLM, ...)");
    println!("  anthropic    Anthropic Messages API");
    println!("  (auto-detected from URL if omitted; anthropic URLs → anthropic, rest → openai)");
}

fn run_oneshot(config: &Config, system: &str, intent: &str, v: Verbosity) {
    let mut messages = vec![Message {
        role: "user".into(),
        content: intent.to_string(),
    }];
    let ph = collect_placeholders();
    let mut cache = ResponseCache::load(config.cache_size);

    print_info(&format!("{} ({})", config.model(), style_label(config.api_style())));
    if v.show_prompt() {
        print_debug(&format!("System prompt:\n{}", system));
        print_debug(&format!("User: {}", intent));
    }
    if v.show_debug() {
        print_debug(&format!("Cache: {} entries (max {})", cache.len(), config.cache_size));
    }

    let mut rl = Editor::<FileHelper, DefaultHistory>::new().ok();

    // Initial LLM call
    let mut spinner = Spinner::start(&format!("{} thinking...", config.model()));
    let result = call_llm_with_retry(config, system, &messages, v, &cache);
    spinner.stop();

    let (final_raw, resp) = match result {
        Ok(resp) => {
            print_usage(&resp.usage);
            let final_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache);
            (final_raw, resp)
        }
        Err(e) => {
            print_error(&e);
            cache.save();
            return;
        }
    };

    let mut current_raw = final_raw;
    let mut last_cache_key = resp.cache_key.clone();
    let mut last_cache_entry = CacheEntry::from(&resp);

    loop {
        let candidates: Vec<String> = parse_candidates(&current_raw)
            .into_iter()
            .map(|c| apply_placeholders(&c, &ph))
            .collect();

        // Show selector if multiple candidates, otherwise just print
        let cmd = if candidates.len() > 1 {
            match select_command(&candidates) {
                Some(i) => candidates[i].clone(),
                None => break,
            }
        } else {
            candidates[0].clone()
        };

        let action = match rl.as_mut() {
            Some(editor) => edit_or_execute(&cmd, editor),
            None => {
                // No editor (unlikely in oneshot), fall back to confirm
                if prompt_confirm("Execute?") {
                    EditAction::Execute(cmd)
                } else {
                    EditAction::Cancel
                }
            }
        };

        match action {
            EditAction::Execute(final_cmd) => {
                execute(&final_cmd);
                // Cache on execute
                if let Some(ref key) = last_cache_key {
                    cache.put(key.clone(), last_cache_entry.clone());
                }
                break;
            }
            EditAction::Refine(text) => {
                // Add assistant response + user refinement to conversation
                messages.push(Message {
                    role: "assistant".into(),
                    content: current_raw.clone(),
                });
                messages.push(Message {
                    role: "user".into(),
                    content: text,
                });

                let mut spinner = Spinner::start(&format!("{} thinking...", config.model()));
                let result = call_llm_with_retry(config, system, &messages, v, &cache);
                spinner.stop();

                match result {
                    Ok(resp) => {
                        print_usage(&resp.usage);
                        current_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache);
                        last_cache_key = resp.cache_key.clone();
                        last_cache_entry = CacheEntry::from(&resp);
                        // Loop back to show new candidates
                    }
                    Err(e) => {
                        print_error(&e);
                        // Remove the two messages we just added
                        messages.pop();
                        messages.pop();
                        // Loop back with previous candidates
                    }
                }
            }
            EditAction::Cancel => break,
        }
    }

    cache.save();
}

fn run_interactive(config: &Config, system: &str, v: Verbosity) {
    print_info(&format!(
        "{} ({}). Tab completes filenames. 'q' quit, 'x' exec/edit/refine, 'c' copy.",
        config.model(),
        style_label(config.api_style()),
    ));

    let ph = collect_placeholders();
    let mut cache = ResponseCache::load(config.cache_size);

    if v.show_debug() {
        print_debug(&format!("Cache: {} entries (max {})", cache.len(), config.cache_size));
    }

    let mut rl = Editor::<FileHelper, DefaultHistory>::new().ok();
    if let Some(ref mut editor) = rl {
        editor.set_helper(Some(FileHelper::new()));
        editor.set_completion_type(rustyline::CompletionType::List);
    }

    let mut messages: Vec<Message> = Vec::new();
    let mut current_cmd = String::new();
    let mut current_cache_key: Option<String> = None;
    let mut current_cache_entry: Option<CacheEntry> = None;

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
                    let action = match rl.as_mut() {
                        Some(editor) => edit_or_execute(&current_cmd, editor),
                        None => {
                            if prompt_confirm("Execute?") {
                                EditAction::Execute(current_cmd.clone())
                            } else {
                                EditAction::Cancel
                            }
                        }
                    };
                    match action {
                        EditAction::Execute(final_cmd) => {
                            execute(&final_cmd);
                            // Cache on execute
                            if let (Some(key), Some(entry)) = (current_cache_key.take(), current_cache_entry.take()) {
                                cache.put(key, entry);
                            }
                        }
                        EditAction::Refine(text) => {
                            // Push current cmd as assistant, refinement as user
                            messages.push(Message {
                                role: "assistant".into(),
                                content: current_cmd.clone(),
                            });
                            messages.push(Message {
                                role: "user".into(),
                                content: text,
                            });
                            if v.show_prompt() {
                                print_debug(&format!("Refine: {}", messages.last().unwrap().content));
                            }
                            let mut spinner = Spinner::start("thinking...");
                            let result = call_llm_with_retry(config, system, &messages, v, &cache);
                            spinner.stop();
                            match result {
                                Ok(resp) => {
                                    print_usage(&resp.usage);
                                    let final_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache);
                                    let candidates: Vec<String> = parse_candidates(&final_raw)
                                        .into_iter()
                                        .map(|c| apply_placeholders(&c, &ph))
                                        .collect();
                                    let cmd = if candidates.len() > 1 {
                                        match select_command(&candidates) {
                                            Some(i) => candidates[i].clone(),
                                            None => {
                                                messages.pop();
                                                messages.pop();
                                                continue;
                                            }
                                        }
                                    } else {
                                        candidates[0].clone()
                                    };
                                    current_cmd = cmd;
                                    current_cache_key = resp.cache_key.clone();
                                    current_cache_entry = Some(CacheEntry::from(&resp));
                                    messages.push(Message {
                                        role: "assistant".into(),
                                        content: final_raw,
                                    });
                                }
                                Err(e) => {
                                    print_error(&e);
                                    messages.pop();
                                    messages.pop();
                                }
                            }
                        }
                        EditAction::Cancel => {}
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

                if v.show_prompt() {
                    print_debug(&format!("User: {}", messages.last().unwrap().content));
                }
                let mut spinner = Spinner::start("thinking...");
                let result = call_llm_with_retry(config, system, &messages, v, &cache);
                spinner.stop();
                match result {
                    Ok(resp) => {
                        print_usage(&resp.usage);
                        let final_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache);
                        let candidates: Vec<String> = parse_candidates(&final_raw)
                            .into_iter()
                            .map(|c| apply_placeholders(&c, &ph))
                            .collect();

                        let cmd = if candidates.len() > 1 {
                            match select_command(&candidates) {
                                Some(i) => candidates[i].clone(),
                                None => {
                                    messages.pop();
                                    continue;
                                }
                            }
                        } else {
                            let c = candidates[0].clone();
                            print_cmd(&c);
                            c
                        };
                        current_cmd = cmd;
                        current_cache_key = resp.cache_key.clone();
                        current_cache_entry = Some(CacheEntry::from(&resp));
                        messages.push(Message {
                            role: "assistant".into(),
                            content: final_raw,
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
    cache.save();
}

fn style_label(style: ApiStyle) -> &'static str {
    match style {
        ApiStyle::OpenAI => "openai",
        ApiStyle::Anthropic => "anthropic",
    }
}

fn execute(cmd: &str) {
    let (command, _) = split_comment(cmd);
    print_info(&format!("Running: {}", command));
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
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
