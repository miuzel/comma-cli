// ── i18n support ─────────────────────────────────────────────────────────────

use crate::config::Config;

/// Initialize i18n from environment variables only (before config is loaded).
/// Priority: COMMA_LANG env > LANG/LC_ALL env > default "en"
pub fn init_from_env() {
    let lang = detect_lang_from_env();
    rust_i18n::set_locale(&lang);
}

/// Initialize i18n with the appropriate locale.
/// Priority: config.lang > COMMA_LANG env > LANG/LC_ALL env > default "en"
pub fn init(config: &Config) {
    let lang = detect_lang(config);
    rust_i18n::set_locale(&lang);
}

/// Detect the language from environment variables only (before config is loaded).
fn detect_lang_from_env() -> String {
    // 1. COMMA_LANG environment variable
    if let Ok(lang) = std::env::var("COMMA_LANG") {
        if !lang.is_empty() {
            return normalize_lang(&lang);
        }
    }

    // 2. System locale (LANG, LC_ALL, LC_MESSAGES)
    for var in &["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(locale) = std::env::var(var) {
            if !locale.is_empty() {
                // Extract language code from locale string (e.g., "zh_CN.UTF-8" -> "zh")
                let lang = locale.split('.').next().unwrap_or(&locale);
                let lang = lang.split('_').next().unwrap_or(lang);
                return normalize_lang(lang);
            }
        }
    }

    // 3. Default to English
    "en".to_string()
}

/// Detect the language to use based on config and environment variables.
fn detect_lang(config: &Config) -> String {
    // 1. Config file lang field (highest priority)
    if let Some(ref lang) = config.lang {
        if !lang.is_empty() {
            return normalize_lang(lang);
        }
    }

    // 2. Fall back to environment-based detection
    detect_lang_from_env()
}

/// Normalize language code to supported languages.
/// Maps variations like "zh-CN", "zh_CN", "zh-Hans" to "zh", etc.
fn normalize_lang(lang: &str) -> String {
    let lower = lang.to_lowercase();
    match lower.as_str() {
        "zh" | "zh-cn" | "zh_cn" | "zh-hans" | "zh-hant" | "chinese" => "zh".to_string(),
        "ja" | "jp" | "japanese" => "ja".to_string(),
        "ko" | "kr" | "korean" => "ko".to_string(),
        "fr" | "french" => "fr".to_string(),
        "de" | "german" => "de".to_string(),
        "es" | "spanish" => "es".to_string(),
        "pt" | "portuguese" => "pt".to_string(),
        "ru" | "russian" => "ru".to_string(),
        "en" | "english" | "c" | "posix" => "en".to_string(),
        _ => {
            // Try prefix match (locale variants like "fr_FR", "pt-BR", "de_DE")
            for prefix in ["zh", "ja", "ko", "fr", "de", "es", "pt", "ru"] {
                if lower.starts_with(prefix) {
                    return prefix.to_string();
                }
            }
            "en".to_string()
        }
    }
}
