use crossterm::style::{Color, ResetColor, SetForegroundColor};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

use crate::cache::{cache_key, ResponseCache};
use crate::config::{ApiStyle, Config, ModelEntry};
use crate::style_label;
use crate::ui::{print_debug, print_info, truncate, Verbosity};

// ── API ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Default, Debug)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read: u32,
    pub cache_creation: u32,
    pub total_tokens: u32,
    pub duration_ms: u64,
    pub from_cache: bool,
}

pub struct LlmResponse {
    pub content: String,
    pub usage: Usage,
    pub cache_key: Option<String>,
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

pub const RETRY_HINT: &str =
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

pub fn print_usage(u: &Usage) {
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

/// Call LLM with retry on empty response. Up to `retries` attempts per entry;
/// an empty response retries once with RETRY_HINT, consuming the next attempt.
pub fn call_llm_with_retry(
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
        // Per-entry message list; the hint pair is appended once after the
        // first empty response, and the hinted call is the next attempt.
        let mut msgs = messages.to_vec();
        let mut attempt = 0;
        while attempt < entry.retries {
            attempt += 1;
            match call_llm(entry, system, &msgs, v, cache, config.reasoning) {
                Ok(resp) if !resp.content.is_empty() => return Ok(resp),
                Ok(_) => {
                    // Empty response — retry with hint, consuming the next attempt
                    if attempt < entry.retries {
                        print_info(&format!(
                            "Empty response from {}, retrying ({}/{})...",
                            entry.model, attempt, entry.retries
                        ));
                        if msgs.len() == messages.len() {
                            msgs.push(Message {
                                role: "assistant".into(),
                                content: "(no response)".to_string(),
                            });
                            msgs.push(Message {
                                role: "user".into(),
                                content: RETRY_HINT.to_string(),
                            });
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

pub fn make_client() -> Result<reqwest::blocking::Client, String> {
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
        // API requires max_tokens > thinking.budget_tokens
        max_tokens: if reasoning > 0 { 1024 + reasoning } else { 1024 },
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
