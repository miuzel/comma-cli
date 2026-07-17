use crate::cache::ResponseCache;
use crate::config::Config;
use crate::context::{apply_placeholders, run_cmd, Placeholders};
use crate::llm::{call_llm_with_retry, Message};
use crate::ui::{
    print_cmd, print_debug, print_error, print_info, prompt_confirm, split_comment, truncate,
    Verbosity,
};

// ── #CHECK: tool availability query ─────────────────────────────────────────

const CHECK_PREFIX: &str = "#CHECK:";

const CHECK_HINT: &str = "\
Here is which tools are available on this system. \
Now generate the best shell command using what's actually installed. \
Output ONLY the final command. Do NOT prefix with #CHECK: or #EXPLORE:.";

/// If raw starts with `#CHECK:`, extract the tool names.
pub fn parse_check(raw: &str) -> Option<Vec<&str>> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix(CHECK_PREFIX)?.trim();
    if rest.is_empty() {
        return None;
    }
    // Strip # comment before parsing tool names
    let (tool_str, _) = split_comment(rest);
    let tools: Vec<&str> = tool_str.split_whitespace().collect();
    if tools.is_empty() {
        None
    } else {
        Some(tools)
    }
}

/// Check which tools are available, return a report string.
fn check_tools(tools: &[&str]) -> String {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for tool in tools {
        if run_cmd("which", &[tool]).is_some() {
            found.push(*tool);
        } else {
            missing.push(*tool);
        }
    }
    let mut parts = Vec::new();
    if !found.is_empty() {
        parts.push(format!("Available: {}", found.join(", ")));
    }
    if !missing.is_empty() {
        parts.push(format!("Not found: {}", missing.join(", ")));
    }
    parts.join("\n")
}

/// If the model returned `#CHECK: t1 t2 t3`, check availability,
/// feed results back to the LLM, and return the real command.
fn check_then_generate(
    config: &Config,
    system: &str,
    messages: &[Message],
    raw: &str,
    v: Verbosity,
    cache: &ResponseCache,
) -> Result<Option<String>, String> {
    let tools = match parse_check(raw) {
        Some(t) => t,
        None => return Ok(None),
    };

    print_info(&format!("Checking tools: {}", tools.join(", ")));
    let report = check_tools(&tools);
    print_info(&report);

    let mut ext = messages.to_vec();
    ext.push(Message {
        role: "assistant".into(),
        content: raw.to_string(),
    });
    ext.push(Message {
        role: "user".into(),
        content: format!("{}\n\nTool availability:\n{}", CHECK_HINT, report),
    });

    let resp = call_llm_with_retry(config, system, &ext, v, cache)?;
    Ok(Some(resp.content))
}

// ── Exploration: #EXPLORE: prefix ───────────────────────────────────────────

const EXPLORE_PREFIX: &str = "#EXPLORE:";

const EXPLORE_HINT: &str = "\
The command output is shown above. You have already explored this tool. \
DO NOT use #EXPLORE: or #CHECK: again. \
Now generate the FINAL shell command the user originally wanted. \
Output ONLY the command, nothing else.";

/// If raw starts with `#EXPLORE:`, extract the command after the prefix.
pub fn parse_explore(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    trimmed.strip_prefix(EXPLORE_PREFIX).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// Run a command, capture stdout+stderr (up to 4096 chars).
fn run_and_capture(cmd: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| format!("Failed to run: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut result = stdout;
    if !stderr.is_empty() {
        result.push_str("\n[stderr]\n");
        result.push_str(&stderr);
    }
    Ok(truncate(&result, 4096).to_string())
}

/// Chain: #CHECK → #EXPLORE → final command.
/// #CHECK can loop (to handle #CHECK after #EXPLORE), but #EXPLORE runs only once.
pub fn process_response(
    config: &Config,
    system: &str,
    messages: &[Message],
    raw: &str,
    ph: &Placeholders,
    v: Verbosity,
    cache: &ResponseCache,
    auto_confirm: bool,
) -> String {
    let mut current = raw.to_string();
    let mut explored = false;

    for _ in 0..5 {
        let after_check = match check_then_generate(config, system, messages, &current, v, cache) {
            Ok(Some(cmd)) => cmd,
            Ok(None) => current.clone(),
            Err(e) => {
                print_error(&format!("Check: {}", e));
                current.clone()
            }
        };

        if explored {
            // Already explored once, stop here
            return after_check;
        }

        match explore_then_generate(config, system, messages, &after_check, ph, v, cache, auto_confirm) {
            Ok(Some(cmd)) => {
                explored = true;
                current = cmd;
            }
            Ok(None) => {
                // Explore was attempted (or not applicable), mark as explored
                if parse_explore(&after_check).is_some() {
                    explored = true;
                }
                if after_check == current {
                    return current; // No change from either step
                }
                current = after_check;
            }
            Err(e) => {
                print_error(&format!("Explore: {}", e));
                return after_check;
            }
        }
    }
    current
}

/// If the model returned `#EXPLORE: <cmd>`, run it with user permission,
/// feed output back to the LLM, and return the real command.
/// Returns Ok(None) if user declines or no #EXPLORE: prefix.
fn explore_then_generate(
    config: &Config,
    system: &str,
    messages: &[Message],
    raw: &str,
    ph: &Placeholders,
    v: Verbosity,
    cache: &ResponseCache,
    auto_confirm: bool,
) -> Result<Option<String>, String> {
    // Handle multiple #EXPLORE candidates separated by |||
    let candidates: Vec<&str> = raw.split("|||")
        .map(|s| s.trim())
        .filter(|s| parse_explore(s).is_some())
        .collect();

    let explore_cmds: Vec<&str> = if candidates.len() > 1 {
        // Multiple explore candidates — show them and ask to run all
        print_info("Model wants to explore:");
        for c in &candidates {
            print_cmd(parse_explore(c).unwrap_or(c));
        }
        if !auto_confirm && !prompt_confirm("Run all to learn usage?") {
            return Ok(None);
        }
        candidates.iter()
            .map(|c| parse_explore(c).unwrap_or(c))
            .collect()
    } else {
        match parse_explore(raw) {
            Some(cmd) => vec![cmd],
            None => return Ok(None),
        }
    };

    // Run all explore commands and collect outputs
    let mut all_output = String::new();
    for cmd_str in &explore_cmds {
        let cmd = apply_placeholders(cmd_str, ph);
        print_info(&format!("Exploring: {}", cmd));
        match run_and_capture(&cmd) {
            Ok(output) => {
                if !output.trim().is_empty() {
                    // Show output to user
                    print_debug(&output);
                    if !all_output.is_empty() {
                        all_output.push_str("\n\n");
                    }
                    all_output.push_str(&format!("$ {}\n{}", cmd, output));
                }
            }
            Err(e) => {
                print_error(&format!("Explore failed: {}", e));
            }
        }
    }

    if all_output.trim().is_empty() {
        print_info("No output from explore commands.");
        return Ok(None);
    }

    if v.show_debug() {
        print_debug(&format!(
            "Captured ({} chars):\n{}",
            all_output.len(),
            truncate(&all_output, 1000)
        ));
    }

    print_info("Learning from output...");

    // Feed help output back: original messages + assistant(#EXPLORE: cmd) + user(hint + output)
    let mut ext = messages.to_vec();
    ext.push(Message {
        role: "assistant".into(),
        content: raw.to_string(),
    });
    ext.push(Message {
        role: "user".into(),
        content: format!("{}\n\nCommand output:\n```\n{}\n```", EXPLORE_HINT, all_output),
    });

    let resp = call_llm_with_retry(config, system, &ext, v, cache)?;
    Ok(Some(resp.content))
}
