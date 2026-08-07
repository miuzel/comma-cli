//! Web search backends for the `#SEARCH:` protocol.
//! DuckDuckGo (default, no API key — scrapes the lite HTML endpoint), with
//! optional Brave / Tavily / SearXNG backends selected via `search.provider`.

use crate::config::SearchConfig;
use crate::llm::make_client;
use rust_i18n::t;

/// One search result. `page_text` carries richer per-result content when the
/// backend provides it natively (Tavily `raw_content`, Brave `extra_snippets`);
/// scraping backends leave it `None`.
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub page_text: Option<String>,
}

/// Max chars of per-result page content fed back to the LLM.
const PAGE_TEXT_MAX: usize = 3000;

/// Run a web search with the configured backend.
pub fn web_search(cfg: &SearchConfig, query: &str) -> Result<Vec<SearchHit>, String> {
    match cfg.provider() {
        // DuckDuckGo rate-limits aggressively; when it returns nothing
        // (anti-bot page or genuinely no results), fall back to Mojeek,
        // which is scraping-friendly and also keyless.
        "duckduckgo" => {
            let ddg = ddg_search(query, cfg.max_results());
            match ddg {
                Ok(hits) if !hits.is_empty() => Ok(hits),
                Ok(_) => {
                    crate::ui::print_info(&t!("search.ddg_fallback"));
                    mojeek_search(query, cfg.max_results())
                }
                Err(ddg_err) => match mojeek_search(query, cfg.max_results()) {
                    Ok(hits) if !hits.is_empty() => {
                        crate::ui::print_info(&t!("search.ddg_fallback"));
                        Ok(hits)
                    }
                    _ => Err(ddg_err),
                },
            }
        }
        "mojeek" => mojeek_search(query, cfg.max_results()),
        "brave" => brave_search(cfg, query),
        "tavily" => tavily_search(cfg, query),
        "searxng" => searxng_search(cfg, query),
        other => Err(t!("search.unknown_provider", "provider" => other).to_string()),
    }
}

/// Render hits as compact numbered text for feeding back to the LLM.
pub fn format_hits(hits: &[SearchHit]) -> String {
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n   {}\n", i + 1, h.title, h.url, h.snippet));
        if let Some(text) = &h.page_text {
            out.push_str(&format!("   Page content:\n   {}\n", text));
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Truncate backend-provided page content to `PAGE_TEXT_MAX` chars
/// (char-boundary safe); empty/whitespace-only becomes `None`.
pub(crate) fn clipped_page_text(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(crate::ui::truncate(t, PAGE_TEXT_MAX).to_string())
    }
}

/// Fetch an HTML page for the scraping backends. Prefers an external `curl`:
/// its TLS fingerprint passes bot checks (DDG anomaly page, Mojeek ALTCHA)
/// that reject reqwest/rustls. Falls back to reqwest when curl is missing.
fn fetch_html(url: &str) -> Result<String, String> {
    const UA: &str = "Mozilla/5.0 (X11; Linux x86_64)";
    const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
    const LANG: &str = "en-US,en;q=0.9";
    if let Some(body) = crate::context::run_cmd("curl", &[
        "-sL", "-m", "20", "-A", UA, "-H", &format!("Accept: {}", ACCEPT), "-H", &format!("Accept-Language: {}", LANG), url,
    ]) {
        if !body.is_empty() {
            return Ok(body);
        }
    }
    make_client()?
        .get(url)
        .header("User-Agent", UA)
        .header("Accept", ACCEPT)
        .header("Accept-Language", LANG)
        .send()
        .map_err(|e| t!("search.request_failed", "e" => e).to_string())?
        .text()
        .map_err(|e| t!("search.request_failed", "e" => e).to_string())
}

// ── DuckDuckGo lite (HTML scraping, no key) ─────────────────────────────────

fn ddg_search(query: &str, max: usize) -> Result<Vec<SearchHit>, String> {
    let url = format!("https://lite.duckduckgo.com/lite/?q={}", url_encode(query));
    let body = fetch_html(&url)?;
    Ok(parse_ddg_lite(&body, max))
}

/// Parse the lite.duckduckgo.com result page. Markup (verified 2026-08):
///   <a rel="nofollow" href="//duckduckgo.com/l/?uddg=<enc>&amp;rut=..." class='result-link'>Title</a>
///   <td class='result-snippet'>snippet (may contain <b> tags and entities)</td>
pub(crate) fn parse_ddg_lite(html: &str, max: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut pos = 0;
    while hits.len() < max {
        // Next result anchor: `class='result-link'` (or double quotes)
        let Some(cls) = find_class(html, pos, "result-link") else { break };
        // The href lives earlier in the same <a ...> tag
        let a_start = html[..cls].rfind("<a ").unwrap_or(0);
        let url = html[a_start..cls]
            .find("href=\"")
            .and_then(|h| {
                let rest = &html[a_start + h + 6..];
                rest.find('"').map(|end| rest[..end].to_string())
            })
            .map(|u| unwrap_ddg_redirect(&u))
            .unwrap_or_default();
        // Title: text between `>` (after the class attr) and `</a>`
        let Some(gt) = html[cls..].find('>').map(|i| cls + i) else { break };
        let Some(a_end) = html[gt..].find("</a>").map(|i| gt + i) else { break };
        let title = clean_html(&html[gt + 1..a_end]);
        // Snippet: the next result-snippet cell after this anchor
        let snippet = match find_class(html, a_end, "result-snippet") {
            Some(sc) => match html[sc..].find('>').map(|i| sc + i) {
                Some(sgt) => match html[sgt..].find("</td>").map(|i| sgt + i) {
                    Some(td_end) => clean_html(&html[sgt + 1..td_end]),
                    None => String::new(),
                },
                None => String::new(),
            },
            None => String::new(),
        };
        if !title.is_empty() {
            hits.push(SearchHit { title, url, snippet, page_text: None });
        }
        pos = a_end + 4;
    }
    hits
}

/// Find the next `class='<name>'` or `class="<name>"` occurrence from `pos`.
fn find_class(html: &str, pos: usize, name: &str) -> Option<usize> {
    let single = format!("class='{}'", name);
    let double = format!("class=\"{}\"", name);
    let s = html[pos..].find(&single).map(|i| pos + i);
    let d = html[pos..].find(&double).map(|i| pos + i);
    match (s, d) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// DDG wraps result URLs in a redirect: //duckduckgo.com/l/?uddg=<enc>&rut=...
/// Extract and decode the real URL; pass through anything else unchanged.
pub(crate) fn unwrap_ddg_redirect(url: &str) -> String {
    match url.find("uddg=") {
        Some(i) => {
            let rest = &url[i + 5..];
            let end = rest.find('&').unwrap_or(rest.len());
            percent_decode(&rest[..end])
        }
        None => url.to_string(),
    }
}

/// Strip HTML tags, decode common entities, collapse whitespace.
fn clean_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Mojeek (HTML scraping, no key, scraping-friendly) ───────────────────────

fn mojeek_search(query: &str, max: usize) -> Result<Vec<SearchHit>, String> {
    let url = format!("https://www.mojeek.com/search?q={}", url_encode(query));
    let body = fetch_html(&url)?;
    Ok(parse_mojeek(&body, max))
}

/// Parse the mojeek.com result page. Markup (verified 2026-08):
///   <h2><a class="title" title="URL" href="URL">Title</a></h2>
///   <p class="s">snippet (may contain <strong> tags)</p>
/// Unlike DDG lite, the href comes AFTER the class attribute here.
pub(crate) fn parse_mojeek(html: &str, max: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut pos = 0;
    while hits.len() < max {
        let Some(cls) = find_class(html, pos, "title") else { break };
        // Tag ends at the next `>`; href sits between the class attr and it
        let Some(gt) = html[cls..].find('>').map(|i| cls + i) else { break };
        let url = html[cls..gt]
            .find("href=\"")
            .and_then(|h| {
                let rest = &html[cls + h + 6..];
                rest.find('"').map(|end| rest[..end].to_string())
            })
            .unwrap_or_default();
        let Some(a_end) = html[gt..].find("</a>").map(|i| gt + i) else { break };
        let title = clean_html(&html[gt + 1..a_end]);
        // Snippet: the next <p class="s"> after this anchor
        let snippet = match html[a_end..].find("<p class=\"s\">").map(|i| a_end + i) {
            Some(sp) => match html[sp..].find("</p>").map(|i| sp + i) {
                Some(p_end) => clean_html(&html[sp + 13..p_end]),
                None => String::new(),
            },
            None => String::new(),
        };
        if !title.is_empty() {
            hits.push(SearchHit { title, url, snippet, page_text: None });
        }
        pos = a_end + 4;
    }
    hits
}

// ── Brave Search API (LLM Context endpoint) ─────────────────────────────────

fn brave_search(cfg: &SearchConfig, query: &str) -> Result<Vec<SearchHit>, String> {
    let key = cfg.api_key.as_deref().filter(|s| !s.is_empty())
        .ok_or_else(|| t!("search.missing_key", "provider" => "brave").to_string())?;
    // The LLM Context endpoint returns pre-extracted page content made for
    // LLM grounding — same key, included in every Search plan.
    let url = format!(
        "https://api.search.brave.com/res/v1/llm/context?q={}&max_urls={}",
        url_encode(query),
        cfg.max_results()
    );
    let body = make_client()?
        .get(&url)
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| t!("search.request_failed", "e" => e).to_string())?;
    let body = check_status(body)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| t!("search.parse_failed", "e" => e).to_string())?;
    Ok(parse_brave_llm_context(&json, cfg.max_results()))
}

/// Parse `grounding.generic[]`: each entry has url, title and `snippets` —
/// plain strings of extracted page content (text, tables, code). The first
/// snippet doubles as the summary line; all of them join into `page_text`.
pub(crate) fn parse_brave_llm_context(json: &serde_json::Value, max: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    if let Some(results) = json["grounding"]["generic"].as_array() {
        for r in results.iter().take(max) {
            let snippets: Vec<&str> = r["snippets"]
                .as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str()).collect())
                .unwrap_or_default();
            hits.push(SearchHit {
                title: r["title"].as_str().unwrap_or_default().to_string(),
                url: r["url"].as_str().unwrap_or_default().to_string(),
                snippet: snippets.first()
                    .map(|s| crate::ui::truncate(s, 200).to_string())
                    .unwrap_or_default(),
                page_text: clipped_page_text(&snippets.join("\n")),
            });
        }
    }
    hits
}

// ── Tavily Search API ───────────────────────────────────────────────────────

fn tavily_search(cfg: &SearchConfig, query: &str) -> Result<Vec<SearchHit>, String> {
    let key = cfg.api_key.as_deref().filter(|s| !s.is_empty())
        .ok_or_else(|| t!("search.missing_key", "provider" => "tavily").to_string())?;
    let payload = serde_json::json!({
        "api_key": key,
        "query": query,
        "max_results": cfg.max_results(),
        // raw_content is only populated on the advanced depth (2 credits/call).
        // Tavily fetches and cleans each result page server-side; one call
        // returns full content, no client-side fetching needed.
        "search_depth": "advanced",
        "include_raw_content": true,
    });
    let body = post_json("https://api.tavily.com/search", &payload)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| t!("search.parse_failed", "e" => e).to_string())?;
    let mut hits = Vec::new();
    if let Some(results) = json["results"].as_array() {
        for r in results.iter().take(cfg.max_results()) {
            // raw_content is null when Tavily could not extract the page.
            hits.push(SearchHit {
                title: r["title"].as_str().unwrap_or_default().to_string(),
                url: r["url"].as_str().unwrap_or_default().to_string(),
                snippet: r["content"].as_str().unwrap_or_default().to_string(),
                page_text: r["raw_content"].as_str().and_then(clipped_page_text),
            });
        }
    }
    Ok(hits)
}

// ── SearXNG (self-hosted) ───────────────────────────────────────────────────

fn searxng_search(cfg: &SearchConfig, query: &str) -> Result<Vec<SearchHit>, String> {
    let base = cfg.base_url.as_deref().filter(|s| !s.is_empty())
        .ok_or_else(|| t!("search.missing_base_url").to_string())?;
    let url = format!("{}/search?q={}&format=json", base.trim_end_matches('/'), url_encode(query));
    let body = make_client()?
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| t!("search.request_failed", "e" => e).to_string())?;
    let body = check_status(body)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| t!("search.parse_failed", "e" => e).to_string())?;
    let mut hits = Vec::new();
    if let Some(results) = json["results"].as_array() {
        for r in results.iter().take(cfg.max_results()) {
            hits.push(SearchHit {
                title: r["title"].as_str().unwrap_or_default().to_string(),
                url: r["url"].as_str().unwrap_or_default().to_string(),
                snippet: r["content"].as_str().unwrap_or_default().to_string(),
                page_text: None,
            });
        }
    }
    Ok(hits)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// POST a JSON body, preferring an external `curl`: its TLS fingerprint passes
/// bot checks that degrade reqwest/rustls responses (Tavily serves a degraded
/// Google-grounding pipeline to rustls clients — same reason `fetch_html`
/// prefers curl). Falls back to reqwest when curl is missing.
fn post_json(url: &str, payload: &serde_json::Value) -> Result<String, String> {
    if let Some(res) = curl_post_json(url, &payload.to_string()) {
        return res;
    }
    let resp = make_client()?
        .post(url)
        .json(payload)
        .send()
        .map_err(|e| t!("search.request_failed", "e" => e).to_string())?;
    check_status(resp)
}

/// `curl -s -X POST --data-binary @-`: body piped via stdin so API keys never
/// appear in the process list. `-w` appends the HTTP status on its own line.
/// None = curl missing or failed to run (caller falls back to reqwest).
fn curl_post_json(url: &str, body: &str) -> Option<Result<String, String>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("curl")
        .args(["-s", "-m", "20", "-X", "POST", "-H", "Content-Type: application/json",
               "--data-binary", "@-", "-w", "\n%{http_code}", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(body.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8(out.stdout).ok()?;
    let (body, status) = raw.rsplit_once('\n')?;
    if status.trim().starts_with('2') {
        Some(Ok(body.to_string()))
    } else {
        Some(Err(t!("search.api_error", "status" => status.trim(), "body" => crate::ui::truncate(body, 200)).to_string()))
    }
}

/// Error out on non-2xx with the response body as context.
fn check_status(resp: reqwest::blocking::Response) -> Result<String, String> {
    let status = resp.status();
    let body = resp.text().map_err(|e| t!("search.request_failed", "e" => e).to_string())?;
    if !status.is_success() {
        return Err(t!("search.api_error", "status" => status, "body" => crate::ui::truncate(&body, 200)).to_string());
    }
    Ok(body)
}

/// Percent-encode a query string (unreserved chars pass through; space becomes
/// `+` — `%20` trips bot detection on Mojeek/DDG, form-style `+` does not).
pub(crate) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Minimal percent-decoder (handles %XX and `+` as space).
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = &s[i + 1..i + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => { out.push(v); i += 3; }
                    Err(_) => { out.push(b'%'); i += 1; }
                }
            }
            b'+' => { out.push(b' '); i += 1; }
            b => { out.push(b); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}
