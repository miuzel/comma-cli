mod cache;
mod config;
mod context;
mod danger;
mod llm;
mod prompt;
mod protocol;
mod tests;
mod ui;
mod update;

use rustyline::config::Configurer;
use rustyline::history::DefaultHistory;
use rustyline::Editor;
use std::io;

use crate::cache::{CacheEntry, ResponseCache};
use crate::config::{load_config, ApiStyle, Config};
use crate::context::{apply_placeholders, collect_placeholders};
use crate::llm::{call_llm_with_retry, print_usage, Message};
use crate::prompt::load_prompt;
use crate::protocol::process_response;
use crate::tests::run_tests;
use crate::ui::{
    copy_to_clipboard, edit_or_execute, is_comment_only, parse_candidates, print_cmd, print_debug,
    print_error, print_info, prompt_confirm, prompt_input, prompt_input_fallback, select_command,
    split_comment, EditAction, FileHelper, Spinner, Verbosity,
};
use crate::update::do_update;

// ── Main logic ──────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("comma {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--update") {
        do_update();
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
        if !atty::is(atty::Stream::Stdin) {
            // Piped stdin: read intent from stdin and run one-shot
            let mut input = String::new();
            io::stdin().read_line(&mut input).ok();
            let intent = input.trim();
            if intent.is_empty() {
                return;
            }
            run_oneshot(&config, &system, intent, verbosity, false);
        } else {
            run_interactive(&config, &system, verbosity, false);
        }
    } else {
        let intent = args.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
        // Check for auto-confirm flag: , install fenster !
        let (intent, auto_confirm) = if intent.ends_with('!') {
            (intent[..intent.len()-1].trim().to_string(), true)
        } else {
            (intent, false)
        };
        run_oneshot(&config, &system, &intent, verbosity, auto_confirm);
    }
}

fn print_help() {
    println!("Usage:");
    println!("  , <intent>   Generate shell command from natural language");
    println!("  ,            Interactive mode (refine commands with conversation)");
    println!("  , -h         Show this help");
    println!("  , --update   Check for updates and self-update");
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

fn run_oneshot(config: &Config, system: &str, intent: &str, v: Verbosity, auto_confirm: bool) {
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
            let final_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache, auto_confirm);
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
            if auto_confirm {
                candidates[0].clone()
            } else {
                match select_command(&candidates) {
                    Some(i) => candidates[i].clone(),
                    None => break,
                }
            }
        } else {
            candidates[0].clone()
        };

        // If command is comment-only (no actual command), just display and exit
        if is_comment_only(&cmd) {
            print_cmd(&cmd);
            break;
        }

        let action = if auto_confirm {
            EditAction::Execute(cmd)
        } else {
            match rl.as_mut() {
                Some(editor) => edit_or_execute(&cmd, editor),
                None => {
                    // No editor (unlikely in oneshot), fall back to confirm
                    if prompt_confirm("Execute?") {
                        EditAction::Execute(cmd)
                    } else {
                        EditAction::Cancel
                    }
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
                        current_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache, auto_confirm);
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

fn run_interactive(config: &Config, system: &str, v: Verbosity, auto_confirm: bool) {
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
                                    let final_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache, auto_confirm);
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
                                    print_cmd(&current_cmd);
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
                        let final_raw = process_response(config, system, &messages, &resp.content, &ph, v, &cache, auto_confirm);
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
                            c
                        };

                        // If command is comment-only, just display and don't store
                        if is_comment_only(&cmd) {
                            print_cmd(&cmd);
                            messages.push(Message {
                                role: "assistant".into(),
                                content: final_raw,
                            });
                            continue;
                        }

                        print_cmd(&cmd);
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

pub fn style_label(style: ApiStyle) -> &'static str {
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
