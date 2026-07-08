use crossterm::style::{Color, ResetColor, SetForegroundColor};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::PathBuf;

// ── Config ──────────────────────────────────────────────────────────────────

/// Config read from `~/.local/bin/,.config.json` — all fields optional.
#[derive(Deserialize, Default)]
struct LocalConfig {
    base_url: Option<String>,
    auth_token: Option<String>,
    model: Option<String>,
}

/// Config read from `~/.claude/settings.json` `env` block.
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
}

fn home_dir() -> Result<String, String> {
    std::env::var("HOME").map_err(|_| "HOME not set".into())
}

/// Load config with priority: local > claude settings > defaults.
fn load_config() -> Result<Config, String> {
    let home = home_dir()?;

    // 1. Try local config
    let local_path = PathBuf::from(&home).join(".local/bin/,.config.json");
    let local: LocalConfig = match std::fs::read_to_string(&local_path) {
        Ok(data) => serde_json::from_str(&data)
            .map_err(|e| format!("Invalid {}: {}", local_path.display(), e))?,
        Err(_) => LocalConfig::default(),
    };

    // 2. Try Claude settings
    let claude_path = PathBuf::from(&home).join(".claude/settings.json");
    let claude_env: Option<ClaudeEnv> = match std::fs::read_to_string(&claude_path) {
        Ok(data) => {
            let settings: ClaudeSettings = serde_json::from_str(&data)
                .map_err(|e| format!("Invalid {}: {}", claude_path.display(), e))?;
            settings.env
        }
        Err(_) => None,
    };

    // Treat empty strings as None
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

    Ok(Config {
        base_url,
        auth_token,
        model,
    })
}

// ── Prompt ──────────────────────────────────────────────────────────────────

fn load_prompt() -> String {
    let home = home_dir().unwrap_or_default();
    let path = PathBuf::from(&home).join(".local/bin/,.prompt.md");
    std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_PROMPT.into())
}

const DEFAULT_PROMPT: &str = r#"You are a shell command generator. The user describes intent in natural language; you output the corresponding shell command.

Rules:
- Output exactly ONE shell command that can be executed directly. No explanations.
- The command should be concise, general-purpose, and correct for Linux.
- If the intent is ambiguous, output the most reasonable default.
- Prefer modern tools (e.g. ripgrep over grep, fd over find) when available.
- If the intent cannot be achieved in one command, output the closest command with a # comment noting the limitation.
- Output ONLY the command, nothing else. No markdown fences, no prose."#;

// ── API ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Option<Vec<ContentBlock>>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ApiError {
    message: Option<String>,
}

fn call_llm(config: &Config, system: &str, messages: &[Message]) -> Result<String, String> {
    let url = format!(
        "{}/v1/messages",
        config.base_url.trim_end_matches('/')
    );
    let body = ApiRequest {
        model: config.model.clone(),
        max_tokens: 1024,
        system: system.to_string(),
        messages: messages.to_vec(),
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP client: {}", e))?;
    let resp = client
        .post(&url)
        .header("x-api-key", &config.auth_token)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read body: {}", e))?;
    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, text));
    }
    let api_resp: ApiResponse =
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
    Ok(result.trim().to_string())
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

fn prompt_input() -> Option<String> {
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

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return;
    }

    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            print_error(&format!("Config: {}", e));
            std::process::exit(1);
        }
    };

    let system = load_prompt();

    if args.is_empty() {
        run_interactive(&config, &system);
    } else {
        let intent = args.join(" ");
        run_oneshot(&config, &system, &intent);
    }
}

fn print_help() {
    println!("Usage:");
    println!("  , <intent>   Generate shell command from natural language");
    println!("  ,            Interactive mode (refine commands with conversation)");
    println!("  , -h         Show this help");
    println!();
    println!("Interactive commands:");
    println!("  x / exec     Execute the current command");
    println!("  c / copy     Copy current command to clipboard");
    println!("  q / quit     Exit");
    println!();
    println!("Config priority: ~/.local/bin/,.config.json > ~/.claude/settings.json");
    println!("Prompt file:     ~/.local/bin/,.prompt.md");
}

fn run_oneshot(config: &Config, system: &str, intent: &str) {
    let messages = vec![Message {
        role: "user".into(),
        content: intent.to_string(),
    }];

    print_info(&format!("Model: {}", config.model));
    match call_llm(config, system, &messages) {
        Ok(cmd) => {
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

fn run_interactive(config: &Config, system: &str) {
    print_info(&format!(
        "Interactive mode (model: {}). Type 'q' to quit, 'x' to execute, 'c' to copy.",
        config.model
    ));

    let mut messages: Vec<Message> = Vec::new();
    let mut current_cmd = String::new();

    loop {
        match prompt_input() {
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
                match call_llm(config, system, &messages) {
                    Ok(cmd) => {
                        current_cmd = cmd.clone();
                        print_cmd(&current_cmd);
                        messages.push(Message {
                            role: "assistant".into(),
                            content: cmd,
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
