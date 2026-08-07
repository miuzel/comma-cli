use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use rust_i18n::t;

// ── API style ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    OpenAI,
    OpenAIResponses,
    Anthropic,
}

impl ApiStyle {
    /// Auto-detect from URL. Defaults to OpenAI chat completions if not
    /// clearly Anthropic or the OpenAI Responses API.
    pub(crate) fn from_url(url: &str) -> Self {
        let lower = url.to_lowercase();
        if lower.contains("anthropic") {
            ApiStyle::Anthropic
        } else if lower.contains("responses") {
            ApiStyle::OpenAIResponses
        } else {
            ApiStyle::OpenAI
        }
    }

    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" | "open_ai" | "oai" => Some(ApiStyle::OpenAI),
            "responses" | "openai-responses" | "openai_responses" | "oai-responses" => {
                Some(ApiStyle::OpenAIResponses)
            }
            "anthropic" | "claude" => Some(ApiStyle::Anthropic),
            _ => None,
        }
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

pub const MAX_RETRIES: usize = 3;

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
    reasoning: Option<Reasoning>,
    max_output_tokens: Option<u32>,
}

#[derive(Clone)]
pub struct ModelEntry {
    pub base_url: String,
    pub auth_token: String,
    pub model: String,
    pub api_style: ApiStyle,
    /// Retry attempts for this model; always >= 1 (clamped at load time).
    pub retries: usize,
    /// Per-model reasoning override; falls back to config-level reasoning.
    pub reasoning: Option<Reasoning>,
    /// Per-model max_output_tokens override; falls back to config-level.
    pub max_output_tokens: Option<u32>,
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
    reasoning: Option<Reasoning>,
    max_output_tokens: Option<u32>,
    // Auto-update: true (default) enables weekly checks; false disables;
    // a number overrides the interval in days (0 = disabled).
    auto_update: Option<AutoUpdate>,
    // Language override (e.g., "en", "zh", "ja")
    lang: Option<String>,
    // Web search backend for the #SEARCH: protocol
    search: Option<SearchConfig>,
}

/// Web search configuration for the `#SEARCH:` protocol. Deserializes from
/// the optional `"search"` object in the config file:
///   "search": { "provider": "duckduckgo" }                     — keyless scraping (bot-detection prone)
///   "search": { "provider": "mojeek" }                         — keyless scraping
///   "search": { "provider": "brave", "api_key": "..." }
///   "search": { "provider": "tavily", "api_key": "..." }
///   "search": { "provider": "searxng", "base_url": "https://searx.example.com" }
/// Search is OFF by default (unset/empty provider): the keyless scraping
/// backends trigger aggressive anti-bot measures (DDG anomaly pages, Mojeek
/// ALTCHA) that can get the user's IP flagged, so #SEARCH only activates
/// when the user explicitly picks a backend.
#[derive(Deserialize, Clone, Default)]
pub struct SearchConfig {
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_results: Option<usize>,
}

impl SearchConfig {
    /// Backend name; "off" (disabled) when unset or empty.
    pub fn provider(&self) -> &str {
        self.provider.as_deref().filter(|s| !s.is_empty()).unwrap_or("off")
    }
    /// #SEARCH is compiled into the prompt only when a provider is chosen.
    pub fn enabled(&self) -> bool {
        self.provider() != "off"
    }
    pub fn max_results(&self) -> usize {
        self.max_results.unwrap_or(5).clamp(1, 10)
    }
}

/// Config-level auto-update setting. Accepts a bool (true/false) or a number
/// (interval in days). Deserializes from JSON:
///   "auto_update": false          — disabled
///   "auto_update": true           — enabled, default 7-day interval
///   "auto_update": 3              — enabled, check every 3 days
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
pub enum AutoUpdate {
    Bool(bool),
    Days(u64),
}

impl Default for AutoUpdate {
    fn default() -> Self {
        AutoUpdate::Bool(true)
    }
}

impl AutoUpdate {
    pub fn enabled(&self) -> bool {
        match self {
            AutoUpdate::Bool(b) => *b,
            AutoUpdate::Days(d) => *d > 0,
        }
    }

    pub fn interval_days(&self) -> u64 {
        match self {
            AutoUpdate::Bool(_) => 7,
            AutoUpdate::Days(d) => *d,
        }
    }
}

/// Reasoning / thinking configuration. Accepts either:
///   - a number (Anthropic token budget, e.g. `"reasoning": 2048`)
///   - a string effort level (e.g. `"reasoning": "low"`)
///     → Anthropic: mapped to token budget
///     → OpenAI: passed as `reasoning.effort` or `reasoning_effort`
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Reasoning {
    Tokens(u32),
    Effort(String),
}

impl Default for Reasoning {
    fn default() -> Self {
        Reasoning::Tokens(0)
    }
}

impl Reasoning {
    /// Token budget for Anthropic thinking.
    pub fn budget_tokens(&self) -> u32 {
        match self {
            Reasoning::Tokens(n) => *n,
            Reasoning::Effort(s) => match s.to_lowercase().as_str() {
                "none" | "" => 0,
                "low" => 1024,
                "medium" => 2048,
                "high" | "xhigh" | "max" => 4096,
                _ => 0,
            },
        }
    }

    /// Effort string for OpenAI APIs ("none", "low", "medium", "high").
    pub fn effort_str(&self) -> &str {
        match self {
            Reasoning::Tokens(0) => "none",
            Reasoning::Tokens(n) => {
                if *n <= 4096 { "low" }
                else if *n <= 16384 { "medium" }
                else { "high" }
            }
            Reasoning::Effort(s) => s,
        }
    }
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

pub struct Config {
    pub entries: Vec<ModelEntry>,
    pub prefer: HashMap<String, Vec<String>>,
    pub cache_size: usize,
    pub reasoning: Reasoning,
    pub max_output_tokens: Option<u32>,
    pub auto_update: AutoUpdate,
    pub lang: Option<String>,
    pub search: SearchConfig,
}

impl Config {
    fn primary(&self) -> &ModelEntry {
        &self.entries[0]
    }
    pub fn model(&self) -> &str {
        &self.primary().model
    }
    pub fn api_style(&self) -> ApiStyle {
        self.primary().api_style
    }

    /// Fuzzy-match a keyword against configured model names (case-insensitive
    /// substring). Returns a new Config with only the matched entry (fallbacks
    /// disabled) or an error listing available models.
    pub fn filter_by_model(&self, keyword: &str) -> Result<Self, String> {
        let kw = keyword.to_lowercase();
        let matched: Vec<&ModelEntry> = self
            .entries
            .iter()
            .filter(|e| e.model.to_lowercase().contains(&kw))
            .collect();

        match matched.len() {
            0 => {
                let available: Vec<&str> = self.entries.iter().map(|e| e.model.as_str()).collect();
                Err(t!("config.no_model_match", "keyword" => keyword, "available" => available.join(", ")).to_string())
            }
            _ => {
                // Pick the first match (preserves config order priority)
                let entry = matched[0].clone();
                Ok(Config {
                    entries: vec![entry],
                    prefer: self.prefer.clone(),
                    cache_size: self.cache_size,
                    reasoning: self.reasoning.clone(),
                    max_output_tokens: self.max_output_tokens,
                    auto_update: self.auto_update,
                    lang: self.lang.clone(),
                    search: self.search.clone(),
                })
            }
        }
    }
}

pub fn home_dir() -> Result<String, String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| t!("config.home_not_set").to_string())
}

/// Directory of the running executable. Portable installs keep the runtime
/// files (`,.config.json`, `,.prompt.md`, `,.cache.json`) next to the binary;
/// falls back to `~/.local/bin` when the exe path cannot be determined.
pub fn exe_dir(home: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(home).join(".local/bin"))
}

/// Resolve a user file: platform default location first, then next to the
/// executable (portable installs), then the legacy `~/.local/bin` path,
/// falling back to the platform default when none exists (where new
/// installs/writes go). The platform default is the XDG location on
/// Linux/macOS — `xdg_env`/`xdg_default` select the base dir:
/// `XDG_CONFIG_HOME`/`~/.config` for config and prompt,
/// `XDG_CACHE_HOME`/`~/.cache` for the cache — and `%APPDATA%\comma\` on
/// Windows.
pub fn xdg_or_legacy(home: &str, xdg_env: &str, xdg_default: &str, name: &str, legacy_name: &str) -> PathBuf {
    let exe_legacy = exe_dir(home).join(format!(",{}", legacy_name));
    let home_legacy = PathBuf::from(home).join(format!(".local/bin/,{}", legacy_name));
    let primary = if cfg!(windows) {
        match std::env::var("APPDATA") {
            Ok(dir) if !dir.is_empty() => PathBuf::from(dir).join(format!("comma/{}", name)),
            _ => PathBuf::from(home).join(format!("AppData/Roaming/comma/{}", name)),
        }
    } else {
        match std::env::var(xdg_env) {
            Ok(dir) if !dir.is_empty() => PathBuf::from(dir).join(format!("comma/{}", name)),
            _ => PathBuf::from(home).join(format!("{}/comma/{}", xdg_default, name)),
        }
    };
    if primary.exists() {
        primary
    } else if exe_legacy.exists() {
        exe_legacy
    } else if home_legacy.exists() {
        home_legacy
    } else {
        primary
    }
}

/// Path to the config file (see `xdg_or_legacy`).
pub fn config_path(home: &str) -> PathBuf {
    xdg_or_legacy(home, "XDG_CONFIG_HOME", ".config", "config.json", ".config.json")
}

/// Set `auto_update` in the config file at `path`, preserving all other keys.
/// A missing file is created with just the flag; invalid JSON is an error
/// (never clobber a config we can't parse).
pub fn write_auto_update_flag(path: &std::path::Path, enabled: bool) -> Result<(), String> {
    let mut json: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data)
            .map_err(|e| t!("config.invalid_file", "path" => path.display(), "e" => e).to_string())?,
        Err(_) => serde_json::json!({}),
    };
    if !json.is_object() {
        json = serde_json::json!({});
    }
    json["auto_update"] = serde_json::Value::Bool(enabled);
    let out = serde_json::to_string_pretty(&json)
        .map_err(|e| e.to_string())?;
    std::fs::write(path, out).map_err(|e| e.to_string())
}

/// Path to the response cache file: `$XDG_CACHE_HOME/comma/cache.json`
/// (default `~/.cache/comma/cache.json`), same fallback chain as the config.
pub fn cache_path(home: &str) -> PathBuf {
    xdg_or_legacy(home, "XDG_CACHE_HOME", ".cache", "cache.json", ".cache.json")
}

pub fn load_config() -> Result<Config, String> {
    let home = home_dir()?;

    // Read config files
    let local_path = config_path(&home);
    let local: LocalConfig = match std::fs::read_to_string(&local_path) {
        Ok(data) => serde_json::from_str(&data)
            .map_err(|e| t!("config.invalid_file", "path" => local_path.display(), "e" => e).to_string())?,
        Err(_) => LocalConfig::default(),
    };

    let claude_path = PathBuf::from(&home).join(".claude/settings.json");
    let claude_env: Option<ClaudeEnv> = match std::fs::read_to_string(&claude_path) {
        Ok(data) => {
            let settings: ClaudeSettings = serde_json::from_str(&data)
                .map_err(|e| t!("config.invalid_file", "path" => claude_path.display(), "e" => e).to_string())?;
            settings.env
        }
        Err(_) => None,
    };

    let non_empty = |o: Option<String>| o.filter(|s| !s.is_empty());
    let env_or = |key: &str| non_empty(std::env::var(key).ok());

    let prefer = local.prefer.unwrap_or_default();
    let cache_size = local.cache_size.unwrap_or(1000);
    let reasoning = local.reasoning.unwrap_or_default();
    let max_output_tokens = local.max_output_tokens;
    let auto_update = local.auto_update.unwrap_or_default();
    let lang = local.lang;
    let search = local.search.unwrap_or_default();

    // Build model entries
    // Priority: COMMA_* env > ,.config.json legacy > ,.config.json providers/models > claude settings
    let entries = if let Some(models) = local.models {
        // New providers/models format — provider fields are required, no claude fallback
        let providers = local.providers.unwrap_or_default();
        let mut entries = Vec::new();
        for (i, m) in models.iter().enumerate() {
            let p = providers.get(&m.provider)
                .ok_or(t!("config.provider_not_found", "name" => m.provider).to_string())?;
            let base_url = env_or("COMMA_BASE_URL")
                .or_else(|| non_empty(p.base_url.clone()))
                .ok_or(t!("config.provider_missing_url", "name" => m.provider).to_string())?;
            let auth_token = env_or("COMMA_API_KEY")
                .or_else(|| non_empty(p.auth_token.clone()))
                .ok_or(t!("config.provider_missing_token", "name" => m.provider, "name2" => m.provider).to_string())?;
            let api_style = env_or("COMMA_API_STYLE")
                .and_then(|s| ApiStyle::from_str(&s))
                .or_else(|| non_empty(p.api_style.clone()).and_then(|s| ApiStyle::from_str(&s)))
                .unwrap_or_else(|| ApiStyle::from_url(&base_url));
            // COMMA_MODEL overrides only the primary (first) entry's model;
            // fallback entries keep their configured model so the fallback
            // list is not collapsed into the same model repeated.
            let model = if i == 0 {
                env_or("COMMA_MODEL").unwrap_or_else(|| m.model.clone())
            } else {
                m.model.clone()
            };
            entries.push(ModelEntry {
                base_url,
                auth_token,
                model,
                api_style,
                // Clamp retries to at least 1: `retries: 0` would mean zero
                // attempts and a confusing "All models returned empty" error.
                retries: m.retries.unwrap_or(1).max(1),
                // Per-model reasoning override; falls back to config-level.
                reasoning: m.reasoning.clone(),
                max_output_tokens: m.max_output_tokens,
            });
        }
        if entries.is_empty() {
            return Err(t!("config.models_empty").to_string());
        }
        entries
    } else {
        // Legacy single-model format
        // Priority: COMMA_* env > ,.config.json > ANTHROPIC_* env > claude settings
        let base_url = env_or("COMMA_BASE_URL")
            .or_else(|| non_empty(local.base_url.clone()))
            .or_else(|| env_or("ANTHROPIC_BASE_URL"))
            .or_else(|| claude_env.as_ref().and_then(|e| e.base_url.clone()))
            .unwrap_or_else(|| "https://api.anthropic.com".into());
        let auth_token = env_or("COMMA_API_KEY")
            .or_else(|| non_empty(local.auth_token.clone()))
            .or_else(|| env_or("ANTHROPIC_API_KEY"))
            .or_else(|| claude_env.as_ref().and_then(|e| e.auth_token.clone()))
            .ok_or(t!("error.no_api_key", "path" => local_path.display()).to_string())?;
        let model = env_or("COMMA_MODEL")
            .or_else(|| non_empty(local.model.clone()))
            .or_else(|| env_or("ANTHROPIC_MODEL"))
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
            reasoning: None,
            max_output_tokens: None,
        }]
    };

    Ok(Config {
        entries,
        prefer,
        cache_size,
        reasoning,
        max_output_tokens,
        auto_update,
        lang,
        search,
    })
}
