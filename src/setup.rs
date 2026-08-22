//! `, --setup`: interactive configuration wizard. Manages LLM providers
//! (add/edit/delete/reorder — the `models` array order is the fallback order)
//! and the web-search backend, then writes `config.json` back with a
//! timestamped backup of the previous file.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use rust_i18n::t;
use rustyline::history::DefaultHistory;
use rustyline::Editor;
use serde_json::{json, Map, Value};

use crate::config::{config_path, home_dir, Reasoning};
use crate::ui::{print_error, print_info, prompt_confirm, FileHelper};

// ── Data model ──────────────────────────────────────────────────────────────

/// One editable LLM provider+model pair (a `models[]` entry joined with its
/// `providers` entry).
#[derive(Clone, PartialEq, Debug)]
pub struct SetupEntry {
    pub provider: String,
    pub base_url: String,
    pub auth_token: String,
    pub api_style: Option<String>,
    pub model: String,
    pub retries: usize,
    pub reasoning: Option<Reasoning>,
    pub max_output_tokens: Option<u32>,
}

/// Editable view of the `search` object. `provider == "off"` disables search
/// (the whole `search` key is dropped on save).
#[derive(Clone, PartialEq, Debug)]
pub struct SetupSearch {
    pub provider: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_results: Option<usize>,
}

impl SetupSearch {
    pub fn from_json(json: &Value) -> Self {
        let s = &json["search"];
        SetupSearch {
            provider: s["provider"].as_str().filter(|p| !p.is_empty()).unwrap_or("off").to_string(),
            api_key: s["api_key"].as_str().filter(|v| !v.is_empty()).map(|v| v.to_string()),
            base_url: s["base_url"].as_str().filter(|v| !v.is_empty()).map(|v| v.to_string()),
            max_results: s["max_results"].as_u64().map(|n| n as usize),
        }
    }

    pub fn to_json(&self) -> Option<Value> {
        if self.provider == "off" {
            return None;
        }
        let mut obj = Map::new();
        obj.insert("provider".into(), json!(self.provider));
        if let Some(k) = &self.api_key {
            obj.insert("api_key".into(), json!(k));
        }
        if let Some(u) = &self.base_url {
            obj.insert("base_url".into(), json!(u));
        }
        if let Some(n) = self.max_results {
            obj.insert("max_results".into(), json!(n));
        }
        Some(Value::Object(obj))
    }
}

/// Parse the config JSON into editable entries. The multi-provider
/// `providers`+`models` format is read directly; the legacy single-model
/// format (top-level base_url/auth_token/model) becomes one "default" entry.
/// Entries whose provider is missing from `providers` are kept with empty
/// fields so the wizard can fix them.
pub fn json_to_entries(json: &Value) -> Vec<SetupEntry> {
    let mut entries = Vec::new();
    if let Some(models) = json["models"].as_array() {
        for m in models {
            let provider = m["provider"].as_str().unwrap_or_default().to_string();
            let p = &json["providers"][&provider];
            entries.push(SetupEntry {
                provider,
                base_url: p["base_url"].as_str().unwrap_or_default().to_string(),
                auth_token: p["auth_token"].as_str().unwrap_or_default().to_string(),
                api_style: p["api_style"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
                model: m["model"].as_str().unwrap_or_default().to_string(),
                retries: m["retries"].as_u64().unwrap_or(1).max(1) as usize,
                reasoning: serde_json::from_value(m["reasoning"].clone()).ok(),
                max_output_tokens: m["max_output_tokens"].as_u64().map(|n| n as u32),
            });
        }
    } else if json["base_url"].is_string() || json["auth_token"].is_string() || json["model"].is_string() {
        // Legacy single-model format — upgraded to providers+models on save.
        entries.push(SetupEntry {
            provider: "default".to_string(),
            base_url: json["base_url"].as_str().unwrap_or_default().to_string(),
            auth_token: json["auth_token"].as_str().unwrap_or_default().to_string(),
            api_style: json["api_style"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
            model: json["model"].as_str().unwrap_or_default().to_string(),
            retries: 1,
            reasoning: None,
            max_output_tokens: None,
        });
    }
    entries
}

/// Reasoning back to its config JSON form (number for a token budget, string
/// for an effort level).
fn reasoning_to_json(r: &Reasoning) -> Value {
    match r {
        Reasoning::Tokens(n) => json!(n),
        Reasoning::Effort(s) => json!(s),
    }
}

/// Rebuild the config JSON from the edited entries and search settings,
/// preserving every unrelated key (prefer, cache_size, full_prompt, custom
/// keys, ...). Legacy single-model top-level keys are removed once the
/// multi-provider format is written.
pub fn entries_to_json(entries: &[SetupEntry], search: &SetupSearch, existing: &Value) -> Value {
    let mut obj = existing.as_object().cloned().unwrap_or_default();
    for key in ["base_url", "auth_token", "model", "api_style"] {
        obj.remove(key);
    }

    let mut providers = Map::new();
    for e in entries {
        let mut p = Map::new();
        p.insert("base_url".into(), json!(e.base_url));
        p.insert("auth_token".into(), json!(e.auth_token));
        if let Some(s) = &e.api_style {
            p.insert("api_style".into(), json!(s));
        }
        providers.insert(e.provider.clone(), Value::Object(p));
    }
    let models: Vec<Value> = entries.iter().map(|e| {
        let mut m = Map::new();
        m.insert("provider".into(), json!(e.provider));
        m.insert("model".into(), json!(e.model));
        if e.retries > 1 {
            m.insert("retries".into(), json!(e.retries));
        }
        if let Some(r) = &e.reasoning {
            m.insert("reasoning".into(), reasoning_to_json(r));
        }
        if let Some(n) = e.max_output_tokens {
            m.insert("max_output_tokens".into(), json!(n));
        }
        Value::Object(m)
    }).collect();
    obj.insert("providers".into(), Value::Object(providers));
    obj.insert("models".into(), json!(models));

    match search.to_json() {
        Some(s) => { obj.insert("search".into(), s); }
        None => { obj.remove("search"); }
    }
    Value::Object(obj)
}

/// Swap an entry with its neighbor; returns false at the list edges.
pub fn move_item<T>(v: &mut [T], i: usize, up: bool) -> bool {
    let j = if up {
        if i == 0 { return false; }
        i - 1
    } else {
        if i + 1 >= v.len() { return false; }
        i + 1
    };
    v.swap(i, j);
    true
}

// ── Timestamped backup ──────────────────────────────────────────────────────

/// Days-from-epoch to (year, month, day), Howard Hinnant's civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYYMMDD-HHMMSS` in UTC, no external date crate.
pub fn utc_timestamp(now_secs: u64) -> String {
    let (y, mo, d) = civil_from_days((now_secs / 86400) as i64);
    let s = now_secs % 86400;
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, mo, d, s / 3600, (s % 3600) / 60, s % 60)
}

/// Copy `path` to `<name>.<UTC timestamp>.bak` next to it. Returns the backup
/// path, or None when there is nothing to back up.
pub fn backup_with_timestamp(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("config.json");
    let backup = path.with_file_name(format!("{}.{}.bak", name, utc_timestamp(now)));
    std::fs::copy(path, &backup).map_err(|e| e.to_string())?;
    Ok(Some(backup))
}

// ── Saving ──────────────────────────────────────────────────────────────────

/// Back up the existing file (timestamped), then atomically write the new
/// config (temp file + rename, parent dir auto-created).
pub fn save_config(path: &Path, json: &Value) -> Result<Option<PathBuf>, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let backup = backup_with_timestamp(path)?;
    let out = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
    let tmp = path.with_file_name(format!(
        ".tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("config.json")
    ));
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(backup)
}

// ── Interactive UI ──────────────────────────────────────────────────────────

/// Minimal raw-mode menu, modeled on `select_command` but without command
/// specifics. Returns the chosen index, or None on Esc/q/Ctrl-C and on
/// non-interactive stdin. Menu items must be single-line.
fn menu_select(title: &str, items: &[String]) -> Option<usize> {
    if items.is_empty() || !atty::is(atty::Stream::Stdin) {
        return None;
    }
    if !title.is_empty() {
        print_info(title);
    }
    let mut selected: usize = 0;
    let draw = |items: &[String], selected: usize| {
        // Each line starts with '\r': in raw mode a bare '\n' leaves the
        // column where the previous line ended, staircasing the menu.
        let mut out = io::stdout().lock();
        for (i, item) in items.iter().enumerate() {
            if i == selected {
                let _ = write!(out, "\r> {}\n", item);
            } else {
                let _ = write!(out, "\r  {}\n", item);
            }
        }
        let _ = out.flush();
    };
    draw(items, selected);
    let rows = items.len() as u16;

    let _ = crossterm::terminal::enable_raw_mode();
    let result = loop {
        if let Ok(Event::Key(KeyEvent { code, modifiers, kind, .. })) = event::read() {
            // Act on Press only (Windows also reports Release/Repeat).
            if kind != KeyEventKind::Press {
                continue;
            }
            match code {
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < items.len() {
                        selected += 1;
                    }
                }
                KeyCode::Tab => selected = (selected + 1) % items.len(),
                KeyCode::BackTab => {
                    selected = if selected == 0 { items.len() - 1 } else { selected - 1 };
                }
                KeyCode::Enter => break Some(selected),
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Esc | KeyCode::Char('q') => break None,
                _ => continue,
            }
            let _ = crossterm::execute!(
                io::stdout(),
                crossterm::cursor::MoveUp(rows),
                crossterm::cursor::MoveToColumn(0),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
            );
            draw(items, selected);
        }
    };
    let _ = crossterm::execute!(
        io::stdout(),
        crossterm::cursor::MoveUp(rows),
        crossterm::cursor::MoveToColumn(0),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
    );
    let _ = crossterm::terminal::disable_raw_mode();
    result
}

/// One line of text via rustyline. Empty input keeps `current`; the returned
/// Option is None only when the user aborts (Ctrl-C/Ctrl-D).
fn text_prompt(rl: &mut Editor<FileHelper, DefaultHistory>, label: &str, current: Option<&str>)
    -> Option<String>
{
    let prompt = match current {
        Some(c) if !c.is_empty() => format!("  {} [{}]: ", label, c),
        _ => format!("  {}: ", label),
    };
    match rl.readline(&prompt) {
        Ok(line) => {
            let line = line.trim();
            if line.is_empty() {
                Some(current.unwrap_or_default().to_string())
            } else {
                Some(line.to_string())
            }
        }
        Err(_) => None,
    }
}

/// Show only the head and last two chars of a secret when listing it.
pub fn mask_secret(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let n = s.chars().count();
    if n <= 6 {
        // Too short to show anything safely.
        return "…".to_string();
    }
    let head: String = s.chars().take(4).collect();
    let tail: String = s.chars().skip(n - 2).collect();
    format!("{}…{}", head, tail)
}

fn entry_label(e: &SetupEntry) -> String {
    format!("{} — {} ({})", e.provider, e.model, e.base_url)
}

// ── Wizard sections ─────────────────────────────────────────────────────────

fn llm_section(entries: &mut Vec<SetupEntry>) {
    loop {
        println!("{}", t!("setup.llm_title"));
        if entries.is_empty() {
            println!("  {}", t!("setup.llm_empty"));
        } else {
            for (i, e) in entries.iter().enumerate() {
                println!("  {}. {}", i + 1, entry_label(e));
            }
        }
        let items = vec![
            t!("setup.llm_add").to_string(),
            t!("setup.llm_edit").to_string(),
            t!("setup.llm_delete").to_string(),
            t!("setup.llm_up").to_string(),
            t!("setup.llm_down").to_string(),
            t!("setup.back").to_string(),
        ];
        match menu_select("", &items) {
            Some(0) => add_entry(entries),
            Some(1) => edit_entry(entries),
            Some(2) => delete_entry(entries),
            Some(3) => reorder_entry(entries, true),
            Some(4) => reorder_entry(entries, false),
            _ => return,
        }
    }
}

fn pick_entry(entries: &[SetupEntry]) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let items: Vec<String> = entries.iter().map(entry_label).collect();
    menu_select(&t!("setup.pick_entry"), &items)
}

fn add_entry(entries: &mut Vec<SetupEntry>) {
    let mut rl = match Editor::<FileHelper, DefaultHistory>::new() {
        Ok(rl) => rl,
        Err(_) => return,
    };
    let provider = match text_prompt(&mut rl, &t!("setup.prompt_provider"), None) {
        Some(p) if !p.is_empty() => p,
        _ => return,
    };
    if entries.iter().any(|e| e.provider == provider) {
        print_error(&t!("setup.provider_exists", "name" => provider));
        return;
    }
    let base_url = match text_prompt(&mut rl, &t!("setup.prompt_base_url"), None) {
        Some(u) if !u.is_empty() => u,
        _ => return,
    };
    let auto_style = match crate::config::ApiStyle::from_url(&base_url) {
        crate::config::ApiStyle::OpenAI => "openai",
        crate::config::ApiStyle::OpenAIResponses => "responses",
        crate::config::ApiStyle::Anthropic => "anthropic",
    };
    let auth_token = match text_prompt(&mut rl, &t!("setup.prompt_auth_token"), None) {
        Some(k) if !k.is_empty() => k,
        _ => return,
    };
    let model = match text_prompt(&mut rl, &t!("setup.prompt_model"), None) {
        Some(m) if !m.is_empty() => m,
        _ => return,
    };
    let style_label = t!("setup.prompt_api_style", "auto" => auto_style).to_string();
    let api_style = text_prompt(&mut rl, &style_label, None)
        .filter(|s| !s.is_empty());
    let retries = text_prompt(&mut rl, &t!("setup.prompt_retries"), None)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    entries.push(SetupEntry {
        provider,
        base_url,
        auth_token,
        api_style,
        model,
        retries,
        reasoning: None,
        max_output_tokens: None,
    });
}

fn edit_entry(entries: &mut [SetupEntry]) {
    let Some(i) = pick_entry(entries) else { return };
    let mut rl = match Editor::<FileHelper, DefaultHistory>::new() {
        Ok(rl) => rl,
        Err(_) => return,
    };
    let e = &mut entries[i];
    // Field names are the literal config keys — left untranslated on purpose.
    if let Some(v) = text_prompt(&mut rl, "base_url", Some(&e.base_url)) {
        e.base_url = v;
    } else { return; }
    if let Some(v) = text_prompt(&mut rl, "auth_token", Some(&mask_secret(&e.auth_token))) {
        if !v.contains('…') {
            e.auth_token = v;
        }
    } else { return; }
    if let Some(v) = text_prompt(&mut rl, "model", Some(&e.model)) {
        e.model = v;
    } else { return; }
    if let Some(v) = text_prompt(&mut rl, "api_style", e.api_style.as_deref()) {
        e.api_style = if v.is_empty() { None } else { Some(v) };
    } else { return; }
    if let Some(v) = text_prompt(&mut rl, "retries", Some(&e.retries.to_string())) {
        if let Ok(n) = v.parse() {
            e.retries = n;
        }
    }
}

fn delete_entry(entries: &mut Vec<SetupEntry>) {
    let Some(i) = pick_entry(entries) else { return };
    if prompt_confirm(&t!("setup.delete_confirm", "name" => entries[i].provider)) {
        entries.remove(i);
    }
}

fn reorder_entry(entries: &mut [SetupEntry], up: bool) {
    let Some(i) = pick_entry(entries) else { return };
    move_item(entries, i, up);
}

fn search_section(search: &mut SetupSearch) {
    let backends = ["off", "duckduckgo", "mojeek", "brave", "tavily", "searxng"];
    let items: Vec<String> = backends.iter()
        .map(|b| if *b == search.provider { format!("{} ●", b) } else { b.to_string() })
        .collect();
    let title = t!("setup.search_title").to_string();
    let Some(i) = menu_select(&title, &items) else { return };
    let chosen = backends[i];
    search.provider = chosen.to_string();

    let mut rl = match Editor::<FileHelper, DefaultHistory>::new() {
        Ok(rl) => rl,
        Err(_) => return,
    };
    match chosen {
        "brave" | "tavily" => {
            let label = t!("setup.prompt_api_key", "provider" => chosen).to_string();
            if let Some(v) = text_prompt(&mut rl, &label, search.api_key.as_deref().map(mask_secret).as_deref()) {
                if !v.contains('…') && !v.is_empty() {
                    search.api_key = Some(v);
                }
            }
        }
        "searxng" => {
            let label = t!("setup.prompt_search_url").to_string();
            if let Some(v) = text_prompt(&mut rl, &label, search.base_url.as_deref()) {
                search.base_url = if v.is_empty() { None } else { Some(v) };
            }
        }
        _ => {}
    }
    if chosen != "off" {
        let current = search.max_results.unwrap_or(5).to_string();
        let label = t!("setup.prompt_max_results", "current" => current.as_str()).to_string();
        if let Some(v) = text_prompt(&mut rl, &label, None) {
            if v.is_empty() {
                return;
            }
            match v.parse::<usize>() {
                Ok(n) => search.max_results = Some(n),
                Err(_) => print_error(&t!("setup.invalid_number", "current" => current)),
            }
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Run the setup wizard. Returns Ok(true) when a config was saved.
pub fn run_setup() -> Result<bool, String> {
    let home = home_dir()?;
    let path = config_path(&home);
    if !atty::is(atty::Stream::Stdin) {
        return Err(t!("setup.requires_tty", "path" => path.display()).to_string());
    }

    let existing: Value = match std::fs::read_to_string(&path) {
        Ok(data) => {
            let v: Value = serde_json::from_str(&data)
                .map_err(|e| t!("config.invalid_file", "path" => path.display(), "e" => e).to_string())?;
            if !v.is_object() {
                return Err(t!("config.invalid_file", "path" => path.display(), "e" => "not an object").to_string());
            }
            v
        }
        Err(_) => json!({}),
    };

    let mut entries = json_to_entries(&existing);
    let mut search = SetupSearch::from_json(&existing);

    loop {
        let items = vec![
            t!("setup.menu_llm").to_string(),
            t!("setup.menu_search").to_string(),
            t!("setup.menu_save").to_string(),
            t!("setup.menu_discard").to_string(),
        ];
        match menu_select(&t!("setup.menu_title"), &items) {
            Some(0) => llm_section(&mut entries),
            Some(1) => search_section(&mut search),
            Some(2) => {
                if entries.is_empty() {
                    print_error(&t!("setup.no_providers_warn"));
                }
                let json = entries_to_json(&entries, &search, &existing);
                match save_config(&path, &json) {
                    Ok(backup) => {
                        if let Some(b) = backup {
                            print_info(&t!("setup.backup_created", "path" => b.display()));
                        }
                        print_info(&t!("setup.saved", "path" => path.display()));
                        return Ok(true);
                    }
                    Err(e) => return Err(t!("setup.save_failed", "path" => path.display(), "e" => e).to_string()),
                }
            }
            Some(3) | None
                if prompt_confirm(&t!("setup.discard_confirm")) => {
                    return Ok(false);
                }
            _ => {}
        }
    }
}
