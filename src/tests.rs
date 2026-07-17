use crate::config::MAX_RETRIES;
use crate::context::{apply_placeholders, collect_placeholders, gather_context};
use crate::llm::RETRY_HINT;
use crate::protocol::{parse_check, parse_explore};
use crate::ui::parse_candidates;

// ── Built-in self-test suite (`--test`) ─────────────────────────────────────

pub fn run_tests() {
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
