mod cache;
mod config;
mod context;
mod danger;
mod history;
mod i18n;
mod llm;
mod prompt;
mod protocol;
mod pty;
mod search;
mod setup;
mod tests;
mod ui;
mod update;

#[macro_use]
extern crate rust_i18n;
i18n!("locales", fallback = "en");

use rustyline::Editor;
use rustyline::config::Configurer;
use rustyline::history::DefaultHistory;
use std::io::{self, IsTerminal};

use crate::cache::{CacheEntry, ResponseCache};
use crate::config::{ApiStyle, Config, Reasoning, history_path, home_dir, load_config};
use crate::context::{Placeholders, apply_placeholders, collect_placeholders, mask_placeholders};
use crate::llm::{Message, call_llm_with_retry, print_usage};
use crate::prompt::load_prompt;
use crate::protocol::process_response;
use crate::tests::run_tests;
use crate::ui::{
    EditAction, FileHelper, Spinner, Verbosity, copy_to_clipboard, edit_or_execute, is_bare_cd,
    is_comment_only, parse_candidates, print_cmd, print_debug, print_error, print_info,
    prompt_confirm, prompt_input, prompt_input_fallback, select_command, split_comment,
};
use crate::update::{check_and_notify, do_update};

// ── Main logic ──────────────────────────────────────────────────────────────

fn main() {
    // Initialize i18n early from environment (before config is loaded)
    i18n::init_from_env();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // Only LEADING args (before the first non-flag arg) are treated as flags;
    // everything after the first positional is intent text, verbatim. `--`
    // explicitly ends the flag run. Unrecognized `-...` words start the intent.
    let mut flags: Vec<&str> = Vec::new();
    let mut model_keyword: Option<String> = None;
    let mut reasoning_keyword: Option<String> = None;
    let mut rest: &[String] = &[];
    let mut i = 0;
    while i < args.len() {
        let s = args[i].as_str();
        let is_flag = matches!(
            s,
            "-h" | "--help"
                | "-V"
                | "--version"
                | "--update"
                | "--test"
                | "--setup"
                | "--default-prompt"
                | "-f"
                | "--nocache"
                | "--model"
                | "--reasoning"
                | "-r"
        ) || (s.starts_with("-v") && s.chars().skip(1).all(|c| c == 'v'));
        if s == "--" {
            rest = &args[i + 1..];
            break;
        } else if s == "--model" {
            // Consume next arg as the model keyword
            i += 1;
            if i < args.len() {
                model_keyword = Some(args[i].clone());
            } else {
                print_error(&t!("error.model_requires_keyword"));
                std::process::exit(1);
            }
        } else if s == "--reasoning" || s == "-r" {
            // Consume next arg as the reasoning level
            i += 1;
            if i < args.len() {
                reasoning_keyword = Some(args[i].clone());
            } else {
                print_error(&t!("error.reasoning_requires_keyword"));
                std::process::exit(1);
            }
        } else if is_flag {
            flags.push(s);
        } else {
            rest = &args[i..];
            break;
        }
        i += 1;
    }

    if flags.iter().any(|a| *a == "-V" || *a == "--version") {
        println!(
            "{}",
            t!("general.version", "v" => env!("CARGO_PKG_VERSION"))
        );
        return;
    }

    if flags.contains(&"--update") {
        do_update();
        return;
    }

    if flags.iter().any(|a| *a == "-h" || *a == "--help") {
        print_help();
        return;
    }

    if flags.contains(&"--test") {
        run_tests();
        return;
    }

    if flags.contains(&"--default-prompt") {
        println!("{}", crate::prompt::DEFAULT_PROMPT);
        return;
    }

    if flags.contains(&"--setup") {
        match setup::run_setup() {
            Ok(_) => {}
            Err(e) => {
                print_error(&e);
                std::process::exit(1);
            }
        }
        return;
    }

    let force_refresh = flags.iter().any(|a| *a == "-f" || *a == "--nocache");

    // Count -v flags (supports -v, -vv, -vvv) among leading flags only
    let verbosity = Verbosity(
        flags
            .iter()
            .filter(|a| a.starts_with("-v") && a.chars().skip(1).all(|c| c == 'v'))
            .map(|a| a.len() as u8 - 1)
            .sum(),
    );

    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            // No usable provider configured: launch the setup wizard
            // interactively and retry; non-TTY keeps the plain error.
            if std::io::stdin().is_terminal() {
                print_info(&t!("setup.auto_setup"));
                match setup::run_setup() {
                    Ok(true) => match load_config() {
                        Ok(c) => c,
                        Err(e2) => {
                            print_error(&t!("error.config_load", "e" => e2));
                            std::process::exit(1);
                        }
                    },
                    Ok(false) => std::process::exit(0),
                    Err(e2) => {
                        print_error(&e2);
                        std::process::exit(1);
                    }
                }
            } else {
                print_error(&t!("error.config_load", "e" => e));
                std::process::exit(1);
            }
        }
    };

    // Initialize i18n based on config and environment
    i18n::init(&config);

    // If --model is set, fuzzy-match and disable fallbacks
    let mut config = if let Some(ref keyword) = model_keyword {
        match config.filter_by_model(keyword) {
            Ok(c) => c,
            Err(e) => {
                print_info(&t!("info.model_fallback", "e" => e));
                config
            }
        }
    } else {
        config
    };

    // If --reasoning/-r is set, override the config reasoning level
    if let Some(ref level) = reasoning_keyword {
        config.reasoning = Reasoning::Effort(level.clone());
    }

    let system = load_prompt(&config);

    if rest.is_empty() {
        if !std::io::stdin().is_terminal() {
            // Piped stdin: read intent from stdin and run one-shot
            if let Some(intent) = read_stdin_intent() {
                run_oneshot(&config, &system, &intent, verbosity, false, force_refresh)
            }
        } else {
            run_interactive(&config, &system, verbosity, false, force_refresh);
        }
    } else if rest.len() == 1 && rest[0] == "!" && !std::io::stdin().is_terminal() {
        // Scriptable auto-confirm escape hatch: echo 'intent' | , !
        if let Some(intent) = read_stdin_intent() {
            run_oneshot(&config, &system, &intent, verbosity, true, force_refresh)
        }
    } else {
        let intent = rest.join(" ");
        // Check for auto-confirm flag: , install fenster !
        let (intent, auto_confirm) = if intent.ends_with('!') {
            (intent[..intent.len() - 1].trim().to_string(), true)
        } else {
            (intent, false)
        };
        run_oneshot(
            &config,
            &system,
            &intent,
            verbosity,
            auto_confirm,
            force_refresh,
        );
    }
}

/// Read a one-shot intent from piped stdin (first line, trimmed).
/// Returns None on read failure or empty input.
fn read_stdin_intent() -> Option<String> {
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok()?;
    let intent = input.trim();
    if intent.is_empty() {
        None
    } else {
        Some(intent.to_string())
    }
}

fn print_help() {
    println!("{}", t!("help.usage"));
    println!("{}", t!("help.intent_desc"));
    println!("{}", t!("help.interactive_desc"));
    println!("{}", t!("help.help_desc"));
    println!("{}", t!("help.version_desc"));
    println!("{}", t!("help.update_desc"));
    println!("{}", t!("help.setup_desc"));
    println!("{}", t!("help.test_desc"));
    println!("{}", t!("help.default_prompt_desc"));
    println!("{}", t!("help.force_desc"));
    println!("{}", t!("help.verbose_desc"));
    println!("{}", t!("help.very_verbose_desc"));
    println!("{}", t!("help.model_desc"));
    println!();
    println!("{}", t!("help.interactive_commands"));
    println!("{}", t!("help.exec_desc"));
    println!("{}", t!("help.refine_desc"));
    println!("{}", t!("help.copy_desc"));
    println!("{}", t!("help.quit_desc"));
    println!("{}", t!("help.confirm_desc"));
    println!("{}", t!("help.tab_desc"));
    println!();
    println!("{}", t!("help.execution_prompt"));
    println!("{}", t!("help.execution_actions"));
    println!();
    println!("{}", t!("help.config_priority"));
    let prm_path = crate::config::home_dir()
        .map(|h| crate::prompt::prompt_path(&h))
        .unwrap_or_default();
    println!("{}", t!("help.prompt_file", "path" => prm_path.display()));
    println!();
    let cfg_path = crate::config::home_dir()
        .map(|h| crate::config::config_path(&h))
        .unwrap_or_default();
    println!("{}", t!("help.config_file", "path" => cfg_path.display()));
    println!("{}", t!("help.auto_update_desc"));
    println!("{}", t!("help.auto_refine_desc"));
    println!();
    println!("{}", t!("help.api_style"));
    println!("{}", t!("help.openai_desc"));
    println!("{}", t!("help.anthropic_desc"));
    println!("{}", t!("help.api_style_auto"));
}

fn run_oneshot(
    config: &Config,
    system: &str,
    intent: &str,
    v: Verbosity,
    auto_confirm: bool,
    force_refresh: bool,
) {
    let mut messages = vec![Message {
        role: "user".into(),
        content: intent.to_string(),
    }];
    let ph = collect_placeholders();
    let mut cache = ResponseCache::load(config.cache_size);
    if force_refresh {
        cache.clear();
        print_info(&t!("info.cache_refreshed"));
    }

    print_info(&format!(
        "{} ({})",
        config.model(),
        style_label(config.api_style())
    ));
    if v.show_prompt() {
        print_debug(&format!("System prompt:\n{}", system));
        print_debug(&format!("User: {}", intent));
    }
    if v.show_debug() {
        print_debug(&format!(
            "Cache: {} entries (max {})",
            cache.len(),
            config.cache_size
        ));
    }

    let mut rl = Editor::<FileHelper, DefaultHistory>::new().ok();

    // Initial LLM call
    let mut spinner = Spinner::start(&t!("interactive.thinking", "m" => config.model()));
    let result = call_llm_with_retry(config, system, &messages, v, &cache, Some(&spinner));
    spinner.stop();

    let (final_raw, resp) = match result {
        Ok(resp) => {
            print_usage(&resp.usage);
            let final_raw = process_response(
                config,
                system,
                &messages,
                &resp.content,
                &ph,
                v,
                &cache,
                auto_confirm,
            );
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
    // Cache the final processed command, not a raw #CHECK:/#EXPLORE: probe
    last_cache_entry.content = current_raw.clone();

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
            // Show the command (and any danger warning) before executing
            print_cmd(&cmd);
            EditAction::Execute(cmd)
        } else {
            match rl.as_mut() {
                Some(editor) => edit_or_execute(&cmd, editor),
                None => {
                    // No editor (unlikely in oneshot), fall back to confirm
                    if prompt_confirm(&t!("ui.execute_confirm")) {
                        EditAction::Execute(cmd)
                    } else {
                        EditAction::Cancel
                    }
                }
            }
        };

        match action {
            EditAction::Execute(final_cmd) => {
                execute(&final_cmd, false);
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

                let mut spinner =
                    Spinner::start(&t!("interactive.thinking", "m" => config.model()));
                let result =
                    call_llm_with_retry(config, system, &messages, v, &cache, Some(&spinner));
                spinner.stop();

                match result {
                    Ok(resp) => {
                        print_usage(&resp.usage);
                        current_raw = process_response(
                            config,
                            system,
                            &messages,
                            &resp.content,
                            &ph,
                            v,
                            &cache,
                            auto_confirm,
                        );
                        last_cache_key = resp.cache_key.clone();
                        last_cache_entry = CacheEntry::from(&resp);
                        // Cache the final processed command, not a raw probe
                        last_cache_entry.content = current_raw.clone();
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
    check_and_notify(config.auto_update);
}

// ── REPL next-step hint ─────────────────────────────────────────────────────

/// The one-line "what can I do next" hint printed right after a freshly
/// generated command in the REPL. The hotkey letters are passed in as
/// placeholders (not baked into the locale text) so a translation can never
/// lose or mangle them. REPL-only: the one-shot and piped-stdin paths never
/// call this, so non-TTY output is unchanged.
fn print_cmd_hint() {
    print_info(&t!(
        "interactive.cmd_hint",
        "exec" => "x",
        "copy" => "c",
        "quit" => "q"
    ));
}

fn run_interactive(
    config: &Config,
    system: &str,
    v: Verbosity,
    auto_confirm: bool,
    force_refresh: bool,
) {
    print_info(&t!(
        "interactive.welcome",
        m = config.model(),
        s = style_label(config.api_style()),
    ));

    let ph = collect_placeholders();
    let mut cache = ResponseCache::load(config.cache_size);
    if force_refresh {
        cache.clear();
        print_info(&t!("info.cache_refreshed"));
    }

    if v.show_debug() {
        print_debug(&format!(
            "Cache: {} entries (max {})",
            cache.len(),
            config.cache_size
        ));
    }

    // REPL input history (opt-in, off by default). When `history` is false the
    // helpers never touch the disk, so no history file is created and `↑`
    // simply has no entries. Only prompt inputs are recorded here — the
    // edit/refine text that `edit_or_execute` adds to the editor stays
    // in memory and is never persisted.
    let history_file = home_dir().ok().map(|h| history_path(&h));
    let mut session_history = history::load_if_enabled(config.history, history_file.as_deref());

    let mut rl = Editor::<FileHelper, DefaultHistory>::new().ok();
    if let Some(ref mut editor) = rl {
        editor.set_helper(Some(FileHelper::new()));
        editor.set_completion_type(rustyline::CompletionType::List);
        // Seed the prompt history so `↑` recalls inputs from previous
        // sessions; empty when the opt-in `history` key is off.
        for line in &session_history {
            let _ = editor.add_history_entry(line.as_str());
        }
    }

    let mut messages: Vec<Message> = Vec::new();
    let mut current_cmd = String::new();
    // Raw LLM reply behind current_cmd (placeholders NOT substituted) —
    // pushed as assistant content on refine so real paths never reach the API
    let mut current_raw = String::new();
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
                // Record REPL inputs (not the q/quit/exit command itself) for
                // the opt-in on-disk history; saved once on exit below.
                session_history.push(input.clone());

                if input == "x" || input == "exec" {
                    if current_cmd.is_empty() {
                        print_error(&t!("error.no_command_execute"));
                        continue;
                    }
                    let action = match rl.as_mut() {
                        Some(editor) => edit_or_execute(&current_cmd, editor),
                        None => {
                            if prompt_confirm(&t!("ui.execute_confirm")) {
                                EditAction::Execute(current_cmd.clone())
                            } else {
                                EditAction::Cancel
                            }
                        }
                    };
                    match action {
                        EditAction::Execute(final_cmd) => {
                            // Capture stdout/stderr only when an automatic
                            // refine may need them; with `auto_refine: false`
                            // the old streaming `.status()` path is kept.
                            let outcome = execute(&final_cmd, config.auto_refine);
                            // Cache on execute
                            if let (Some(key), Some(entry)) =
                                (current_cache_key.take(), current_cache_entry.take())
                            {
                                cache.put(key, entry);
                            }
                            // At most one automatic refine per executed
                            // command: this branch runs once per `x`, and the
                            // refine turn itself never executes anything.
                            if should_auto_refine(outcome.as_ref(), config.auto_refine) {
                                let code = outcome.as_ref().and_then(|o| o.code).unwrap_or(-1);
                                print_info(&t!("interactive.auto_refine_notice", "code" => code));
                                let body = auto_refine_body(
                                    &final_cmd,
                                    code,
                                    outcome.as_ref().map(|o| o.output.as_str()).unwrap_or(""),
                                    &ph,
                                );
                                if let Some(res) = do_refine(
                                    config,
                                    system,
                                    &mut messages,
                                    &ph,
                                    v,
                                    auto_confirm,
                                    &cache,
                                    &current_raw,
                                    &body,
                                ) {
                                    current_cmd = res.cmd;
                                    current_raw = res.raw;
                                    current_cache_key = res.cache_key;
                                    current_cache_entry = Some(res.entry);
                                    print_cmd(&current_cmd);
                                    print_cmd_hint();
                                }
                            }
                        }
                        EditAction::Refine(text) => {
                            if let Some(res) = do_refine(
                                config,
                                system,
                                &mut messages,
                                &ph,
                                v,
                                auto_confirm,
                                &cache,
                                &current_raw,
                                &text,
                            ) {
                                current_cmd = res.cmd;
                                current_raw = res.raw;
                                current_cache_key = res.cache_key;
                                current_cache_entry = Some(res.entry);
                                print_cmd(&current_cmd);
                                print_cmd_hint();
                            }
                        }
                        EditAction::Cancel => {}
                    }
                    continue;
                }

                // Direct refine entry point at the main prompt: `/refine TEXT`
                // (alias `/r TEXT`), so refining no longer requires `x` first.
                if let Some(text) = parse_refine_command(&input) {
                    let text = text.trim();
                    if current_raw.is_empty() {
                        print_error(&t!("error.no_command_refine"));
                    } else if text.is_empty() {
                        print_error(&t!("error.refine_requires_text"));
                    } else if let Some(res) = do_refine(
                        config,
                        system,
                        &mut messages,
                        &ph,
                        v,
                        auto_confirm,
                        &cache,
                        &current_raw,
                        text,
                    ) {
                        current_cmd = res.cmd;
                        current_raw = res.raw;
                        current_cache_key = res.cache_key;
                        current_cache_entry = Some(res.entry);
                        print_cmd(&current_cmd);
                        print_cmd_hint();
                    }
                    continue;
                }

                if input == "c" || input == "copy" {
                    if current_cmd.is_empty() {
                        print_error(&t!("error.no_command_copy"));
                    } else {
                        copy_to_clipboard(&current_cmd);
                        print_info(&t!("info.copied"));
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
                let mut spinner = Spinner::start(&t!("interactive.thinking_short"));
                let result =
                    call_llm_with_retry(config, system, &messages, v, &cache, Some(&spinner));
                spinner.stop();
                match result {
                    Ok(resp) => {
                        print_usage(&resp.usage);
                        let final_raw = process_response(
                            config,
                            system,
                            &messages,
                            &resp.content,
                            &ph,
                            v,
                            &cache,
                            auto_confirm,
                        );
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
                            candidates[0].clone()
                        };

                        // If command is comment-only, just display and don't store
                        // it: there is no command for `x` to act on, so the
                        // next-step hint would be a lie here.
                        if is_comment_only(&cmd) {
                            print_cmd(&cmd);
                            messages.push(Message {
                                role: "assistant".into(),
                                content: final_raw,
                            });
                            continue;
                        }

                        print_cmd(&cmd);
                        print_cmd_hint();
                        current_cmd = cmd;
                        current_raw = final_raw.clone();
                        current_cache_key = resp.cache_key.clone();
                        let mut entry = CacheEntry::from(&resp);
                        // Cache the final processed command, not a raw probe
                        entry.content = final_raw.clone();
                        current_cache_entry = Some(entry);
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
    // Single exit point (q/quit/exit all `break` here): persist the REPL
    // history when the feature is on. No-op when it is off.
    history::save_if_enabled(config.history, history_file.as_deref(), &session_history);
    check_and_notify(config.auto_update);
}

pub fn style_label(style: ApiStyle) -> &'static str {
    match style {
        ApiStyle::OpenAI => "openai",
        ApiStyle::OpenAIResponses => "responses",
        ApiStyle::Anthropic => "anthropic",
    }
}

// ── Refine ──────────────────────────────────────────────────────────────────

/// New command produced by one refine turn.
struct RefineOutcome {
    /// LLM reply with placeholders still unsubstituted (the next refine's
    /// assistant turn must stay raw so real paths never reach the API).
    raw: String,
    /// Selected candidate, placeholders applied (what the user would execute).
    cmd: String,
    cache_key: Option<String>,
    entry: CacheEntry,
}

/// The two conversation turns a refine pushes, in order: the *raw* reply
/// behind the current command as the assistant turn (never the substituted
/// one — this is the project's privacy invariant), then the refinement text
/// as the user turn. The automatic refine uses the same convention with a
/// generated refinement text.
fn refine_turns(raw_reply: &str, refine_text: &str) -> [Message; 2] {
    [
        Message {
            role: "assistant".into(),
            content: raw_reply.to_string(),
        },
        Message {
            role: "user".into(),
            content: refine_text.to_string(),
        },
    ]
}

/// Run one refine turn and return the new command, or `None` when the call
/// failed or the user cancelled candidate selection. On failure both pushed
/// turns are rolled back (mirroring the existing refine branch), so a Ctrl-C
/// or a network error cannot poison `messages`.
#[allow(clippy::too_many_arguments)]
fn do_refine(
    config: &Config,
    system: &str,
    messages: &mut Vec<Message>,
    ph: &Placeholders,
    v: Verbosity,
    auto_confirm: bool,
    cache: &ResponseCache,
    raw_reply: &str,
    refine_text: &str,
) -> Option<RefineOutcome> {
    messages.extend(refine_turns(raw_reply, refine_text));
    if v.show_prompt() {
        print_debug(&format!("Refine: {}", refine_text));
    }
    let mut spinner = Spinner::start(&t!("interactive.thinking_short"));
    let result = call_llm_with_retry(config, system, messages, v, cache, Some(&spinner));
    spinner.stop();
    match result {
        Ok(resp) => {
            print_usage(&resp.usage);
            let final_raw = process_response(
                config,
                system,
                messages,
                &resp.content,
                ph,
                v,
                cache,
                auto_confirm,
            );
            let candidates: Vec<String> = parse_candidates(&final_raw)
                .into_iter()
                .map(|c| apply_placeholders(&c, ph))
                .collect();
            let cmd = if candidates.len() > 1 {
                match select_command(&candidates) {
                    Some(i) => candidates[i].clone(),
                    None => {
                        messages.pop();
                        messages.pop();
                        return None;
                    }
                }
            } else {
                candidates[0].clone()
            };
            let mut entry = CacheEntry::from(&resp);
            // Cache the final processed command, not a raw probe
            entry.content = final_raw.clone();
            messages.push(Message {
                role: "assistant".into(),
                content: final_raw.clone(),
            });
            Some(RefineOutcome {
                raw: final_raw,
                cmd,
                cache_key: resp.cache_key.clone(),
                entry,
            })
        }
        Err(e) => {
            print_error(&e);
            // Roll back the two turns we just added
            messages.pop();
            messages.pop();
            None
        }
    }
}

/// `/refine TEXT` and its `/r TEXT` alias at the main REPL prompt. Returns the
/// text after the command (empty when nothing follows); `None` when the input
/// is not a refine command at all. Slash-prefixed so a natural-language intent
/// is never swallowed: `/refinex ...` is not a refine command.
pub(crate) fn parse_refine_command(input: &str) -> Option<&str> {
    let rest = input
        .strip_prefix("/refine")
        .or_else(|| input.strip_prefix("/r"))?;
    if rest.is_empty() {
        Some("")
    } else if rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

// ── Auto-refine after a failed command ──────────────────────────────────────

/// Upper bound on the captured output sent to the API (in characters). Long
/// output is truncated head+tail because the interesting part (the error) is
/// usually at the end.
pub(crate) const AUTO_REFINE_MAX_OUTPUT_CHARS: usize = 2000;

/// Strip ANSI escape sequences and unprintable control characters from
/// captured command output so the summary cannot smuggle terminal control
/// codes into the API request.
fn sanitize_output(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                // CSI: ESC [ ... final byte 0x40..=0x7e
                Some('[') => {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c2) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL, or string terminator ESC \
                Some(']') => {
                    chars.next();
                    while let Some(c2) = chars.next() {
                        if c2 == '\u{7}' {
                            break;
                        }
                        if c2 == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Any other escape: drop the escape and its single parameter
                _ => {
                    chars.next();
                }
            }
            continue;
        }
        match c {
            '\r' => cleaned.push('\n'),
            '\n' | '\t' => cleaned.push(c),
            c if c.is_control() => {}
            c => cleaned.push(c),
        }
    }
    // Collapse the CRLF/multiple newlines the \r mapping may produce.
    let mut out = String::with_capacity(cleaned.len());
    let mut last_was_newline = false;
    for c in cleaned.chars() {
        if c == '\n' {
            if last_was_newline {
                continue;
            }
            last_was_newline = true;
        } else {
            last_was_newline = false;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// Truncate to `max` characters, keeping both ends (head + tail) with a marker
/// in between. The result never exceeds `max` characters.
fn truncate_output(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let head = max / 4;
    let mut tail = max.saturating_sub(head + 40);
    let mut marker = String::new();
    // The marker is part of the budget and its length depends on the omitted
    // count, which in turn depends on the budget — settle in a couple of passes.
    for _ in 0..3 {
        let omitted = count.saturating_sub(head + tail);
        marker = format!("\n… [{} chars omitted] …\n", omitted);
        let next = max.saturating_sub(head + marker.chars().count());
        if next == tail {
            break;
        }
        tail = next;
    }
    let mut out: String = text.chars().take(head).collect();
    out.push_str(&marker);
    out.extend(text.chars().skip(count - tail));
    out
}

/// Sanitize, mask and truncate captured output for the auto-refine payload.
/// Masking happens BEFORE truncation: cutting a real path in half would still
/// leak a (partial) private value.
pub(crate) fn auto_refine_output(raw: &str, ph: &Placeholders) -> String {
    truncate_output(
        &mask_placeholders(&sanitize_output(raw), ph),
        AUTO_REFINE_MAX_OUTPUT_CHARS,
    )
}

/// The user turn sent for an automatic refine: the failed command, its exit
/// code and the (masked, truncated) output summary. Empty output is sent as a
/// command + exit code only.
pub(crate) fn auto_refine_body(
    cmd: &str,
    code: i32,
    raw_output: &str,
    ph: &Placeholders,
) -> String {
    let cmd = mask_placeholders(cmd, ph);
    let output = auto_refine_output(raw_output, ph);
    if output.is_empty() {
        t!("interactive.auto_refine_body_empty", "cmd" => cmd, "code" => code).to_string()
    } else {
        t!(
            "interactive.auto_refine_body",
            "cmd" => cmd,
            "code" => code,
            "output" => output
        )
        .to_string()
    }
}

/// Should a finished command start an automatic refine?
///
/// "Failed" means a non-zero exit code — including a signal death or a spawn
/// failure, where `code()` is `None` — and never a guess about the output
/// text. The outcome is `None` in `COMMA_EVAL_FILE` mode (nothing was executed
/// here), which never triggers. The REPL calls this once per `execute()` call,
/// so one executed command triggers at most one automatic refine.
pub(crate) fn should_auto_refine(outcome: Option<&ExecOutcome>, enabled: bool) -> bool {
    match outcome {
        Some(o) => enabled && o.code != Some(0),
        None => false,
    }
}

// ── Command execution ───────────────────────────────────────────────────────

/// Result of running a confirmed command in a child shell.
pub(crate) struct ExecOutcome {
    /// Exit code of the child process; `None` when it was killed by a signal
    /// (or could not be spawned at all).
    pub code: Option<i32>,
    /// Combined stdout+stderr, raw and untruncated (capture mode only).
    pub output: String,
}

/// Run the confirmed command. When `COMMA_EVAL_FILE` is set (shell
/// integration, see README), the comment-stripped command is appended to
/// that file — one per line — instead of being spawned: the wrapper shell
/// function evals the file in the *current* shell, so `cd`/`export` take
/// effect there (a child process could never change the parent shell's cwd).
/// In interactive mode each execution appends one line; the wrapper evals
/// them in order when the session exits. Without the variable, the command
/// runs in a child shell matching the dialect reported in the system context
/// (`$SHELL` on Unix, `cmd /C` on Windows without a POSIX `SHELL`).
///
/// Returns `None` in `COMMA_EVAL_FILE` mode: nothing was executed here (the
/// wrapper shell evals the appended line later), so there is no exit code or
/// output and no automatic refine can fire.
///
/// `capture` selects how the child is run:
/// - `false` — `.status()`: stdio is inherited, so output streams live and
///   progress bars and TTY-dependent programs behave as before; nothing is
///   captured, so no auto-refine summary is available.
/// - `true` — the output must be captured for a possible auto-refine summary.
///   On Unix the child runs on a pty (see `crate::pty`): output is relayed to
///   our terminal as it arrives *and* accumulated, so it stays live and the
///   child keeps full TTY semantics (colors, progress bars, `vim`/`less`); our
///   terminal is in raw mode for the duration and is restored unconditionally.
///   Windows (and a Unix host where no pty can be allocated) falls back to
///   streaming pipes: still live, but the child sees pipes, not a TTY — that
///   documented degradation is reported, never silent.
pub(crate) fn execute(cmd: &str, capture: bool) -> Option<ExecOutcome> {
    let (command, _) = split_comment(cmd);
    print_info(&t!("info.running", "cmd" => command));

    if let Ok(path) = std::env::var("COMMA_EVAL_FILE")
        && !path.is_empty()
    {
        use std::io::Write;
        let line = command.lines().next().unwrap_or("");
        let result = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "{}", line));
        if let Err(e) = result {
            print_error(&t!("error.failed_write_eval", "path" => path, "e" => e));
        }
        return None;
    }

    if is_bare_cd(command) {
        print_info(&t!("info.cd_subprocess"));
    }

    let (prog, args) = crate::context::shell_command();
    let mut child = std::process::Command::new(&prog);
    child.args(&args).arg(command);

    if !capture {
        let status = child.status();
        match status {
            Ok(s) => {
                if !s.success() {
                    print_error(&t!("error.exit_code", "code" => s.code().unwrap_or(-1)));
                }
                Some(ExecOutcome {
                    code: s.code(),
                    output: String::new(),
                })
            }
            Err(e) => {
                print_error(&t!("error.failed_execute", "e" => e));
                Some(ExecOutcome {
                    code: None,
                    output: String::new(),
                })
            }
        }
    } else {
        // The output has to be captured for a possible auto-refine summary.
        // `pty::run_captured` streams it live (on a pty where possible).
        match crate::pty::run_captured(child) {
            Ok(run) => {
                if run.code != Some(0) {
                    print_error(&t!("error.exit_code", "code" => run.code.unwrap_or(-1)));
                }
                Some(ExecOutcome {
                    code: run.code,
                    output: run.output,
                })
            }
            Err(e) => {
                print_error(&t!("error.failed_execute", "e" => e));
                Some(ExecOutcome {
                    code: None,
                    output: String::new(),
                })
            }
        }
    }
}
