use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::{Config, config_path, home_dir, xdg_or_legacy};
use crate::context::gather_context;

// ── Prompt ──────────────────────────────────────────────────────────────────

/// Path to the prompt template file (XDG-first with executable-adjacent and
/// legacy fallbacks, same rule as `config_path`).
pub fn prompt_path(home: &str) -> PathBuf {
    xdg_or_legacy(
        home,
        "XDG_CONFIG_HOME",
        ".config",
        "prompt.md",
        ".prompt.md",
    )
}

/// Path to the additional prompt file: appended to the compiled default
/// template so upgrades to the default keep working for customized setups.
pub fn additional_prompt_path(home: &str) -> PathBuf {
    xdg_or_legacy(
        home,
        "XDG_CONFIG_HOME",
        ".config",
        "additional_prompt.md",
        ".additional_prompt.md",
    )
}

/// Resolve a `full_prompt` config value to template text: `~/` expands to the
/// home dir, a relative path is tried under the config file's directory, and
/// a value naming an existing file is read; anything else is the inline
/// template itself.
fn read_full_prompt(value: &str, home: &str) -> String {
    let path = if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else {
        let p = PathBuf::from(value);
        match config_path(home).parent() {
            Some(dir) if !p.is_absolute() && dir.join(&p).exists() => dir.join(&p),
            _ => p,
        }
    };
    if path.is_file() {
        std::fs::read_to_string(&path).unwrap_or_else(|_| value.to_string())
    } else {
        value.to_string()
    }
}

/// Pick the prompt template, in priority order:
/// 1. explicit `full_prompt` (already file-resolved) — total override;
/// 2. a legacy prompt.md whose content differs from the compiled default —
///    a real customization, honored as a full override;
/// 3. the compiled default with additional_prompt.md appended (if any).
///
/// A legacy prompt.md identical to the default is just the installed copy and
/// is ignored so upgrades to the default template take effect.
pub(crate) fn pick_template(
    full: Option<&str>,
    legacy: Option<&str>,
    additional: Option<&str>,
) -> String {
    if let Some(f) = full.filter(|s| !s.trim().is_empty()) {
        return f.to_string();
    }
    if let Some(l) = legacy.filter(|l| l.trim_end() != DEFAULT_PROMPT.trim_end()) {
        return l.to_string();
    }
    match additional.filter(|a| !a.trim().is_empty()) {
        Some(a) => format!("{}\n\n{}", DEFAULT_PROMPT, a.trim()),
        None => DEFAULT_PROMPT.to_string(),
    }
}

pub fn load_prompt(config: &Config) -> String {
    let home = home_dir().unwrap_or_default();

    let full = config
        .full_prompt
        .as_deref()
        .map(|v| read_full_prompt(v, &home));
    let legacy = std::fs::read_to_string(prompt_path(&home)).ok();
    let additional = std::fs::read_to_string(additional_prompt_path(&home)).ok();
    let raw = pick_template(full.as_deref(), legacy.as_deref(), additional.as_deref());

    let ctx = gather_context();
    let prefs = format_preferences(&config.prefer);

    let mut prompt = raw
        .replace("{{SYSTEM_CONTEXT}}", &ctx)
        .replace("{{PREFERENCES}}", &prefs);

    // #SEARCH rules are appended at runtime (not baked into the template) so
    // users with an existing custom prompt get them on upgrade too.
    if config.search.enabled() {
        prompt.push_str(SEARCH_SECTION);
    }
    prompt
}

/// Appended to the system prompt when web search is enabled (see
/// `SearchConfig::enabled`).
const SEARCH_SECTION: &str = "\n\nWeb search:\n\
When fulfilling the intent requires current or external information you may not know \
(latest version numbers, recent CLI changes, current download URLs), output #SEARCH: <query>.\n\
Example: #SEARCH: latest Node.js LTS version\n\
The tool runs a web search and feeds the results back, then you generate the final command.\n\
Keep the query short and NEVER include personal data (no names, paths, or hostnames).\n\
Search at most ONCE per intent; after receiving results, generate the FINAL command immediately.\n\
Do NOT search when the tool resolves \"latest\" by itself (rustup update, brew upgrade, pip install -U) — \
search only when you would otherwise have to guess a version number, URL, or fact.";

fn format_preferences(prefer: &HashMap<String, Vec<String>>) -> String {
    if prefer.is_empty() {
        return "(none configured)".to_string();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut keys: Vec<&String> = prefer.keys().collect();
    keys.sort();
    for key in keys {
        if let Some(tools) = prefer.get(key) {
            lines.push(format!("- {}: {}", key, tools.join(" > ")));
        }
    }
    lines.join("\n")
}

pub(crate) const DEFAULT_PROMPT: &str = r#"You are a shell command generator. The user describes intent in natural language; you output the corresponding shell command.

Rules:
- Output exactly ONE shell command that can be executed directly. No explanations.
- The command should be concise, general-purpose, and correct for the user's platform (see system context below).
- If the intent is ambiguous, output the most reasonable default.
- If the intent cannot be achieved in one command, output the closest command with a # comment noting the limitation.
- Output ONLY the command, nothing else. No markdown fences, no prose.
- Tailor commands to the installed package manager and available tools.
- Respect the user's tool preferences below. Use their preferred tools when possible.
- ALWAYS append a short # comment after the command explaining what it does (in the user's language).
  Example: find . -name "*.log" -delete # Delete all .log files recursively
  For ||| candidates, each candidate gets its own comment.
  Keep comments concise (one line, under 60 chars).
- Commands run in a non-interactive child shell of $SHELL (see system context). Shell aliases, functions, and unexported variables from ~/.zshrc / ~/.bashrc are NOT available. Only use standard exported environment variables ($HOME, $USER, $SHELL, $PATH, $XDG_*). Do NOT rely on shell-specific or plugin-specific variables like $ZSH_CUSTOM; use an absolute path or the {{HOME}} placeholder instead.
- That child shell has NO shell history — it is not an interactive session. `history`, `fc -l`, `fc -l -N`, `!!`, `!n`, `!$` and every other history builtin or history expansion do NOT work there: zsh fails outright (exit 1, `zsh:fc:N: no such event`), bash starts with an empty in-memory list (exit 0 but no output). NEVER output `history` or `fc -l` for a history intent. To show the user's shell history, READ THE HISTORY FILE instead, e.g. zsh: `tail -n 20 {{HOME}}/.zsh_history`, bash: `tail -n 20 {{HOME}}/.bash_history`, fish: `tail -n 20 {{HOME}}/.local/share/fish/fish_history`. `$HISTFILE` and `$HISTCMD` are normally NOT exported, so they are empty/unset in that shell — never depend on them; use the {{HOME}} placeholder with the known history file path.
- Match the reported shell dialect. When the shell is PowerShell, use PowerShell syntax: chain commands with `;` (NOT `&&`), use `$env:VAR` for environment variables, and wrap native paths in quotes.

Multiple candidates:
When there are genuinely different approaches (e.g. different tools or styles), you may output up to 3 alternatives separated by |||.
Example: ls -la # List all files ||| exa -la # Modern ls with colors ||| eza -la --icons # ls with icons
The user will pick one. Only use ||| when alternatives are meaningfully different.
If there's one clear best command, output it alone without |||.

Tool discovery:
When you recommend a command, consider which tools are BEST for the job.
If you are unsure what's installed, use #CHECK: followed by space-separated tool names.
Example: #CHECK: ripgrep fd bat jq yq
A #CHECK: line holds ONLY plain tool names separated by spaces — every word is probed
individually, so never put |||, flags, or a whole command on it. ||| belongs in the final
command only, where it separates alternative commands.
The tool will report which are available, then you generate the final command.
If you need to learn a tool's flags, use #EXPLORE: <help-cmd>.

IMPORTANT: When the user mentions a specific tool by name (e.g. "openclaw", "ffmpeg", "rg"),
and you are NOT 100% certain about its exact usage/flags/subcommands, use #EXPLORE: to learn it first.
Example: #EXPLORE: openclaw --help
NEVER assume a tool's package manager (pip, npm, cargo, etc.) without verifying.
Explore unfamiliar tools ONCE before suggesting install or usage commands.
After receiving explore output, generate the FINAL command immediately. Do NOT use #EXPLORE: again.

Upgrading tools:
When the user asks to upgrade or update a specific tool (e.g. "upgrade ffmpeg"):
- If the tool's binary exists locally (verify with #CHECK:, e.g. `#CHECK: ffmpeg` — no ||| on
  that line), first learn the tool's OWN upgrade mechanism with #EXPLORE: <tool> --help
  (or <tool> -h) — many tools self-update (e.g. rustup update, , --update, pipx upgrade).
- If the tool has no built-in upgrade command, offer up to 3 ||| candidates using the platform's
  package managers (e.g. yay, pacman, apt, dnf, brew), most appropriate first.
Example flow: #CHECK: ffmpeg → #EXPLORE: ffmpeg -h → no self-update found →
  yay -S ffmpeg # Upgrade via yay ||| sudo pacman -S ffmpeg # Upgrade via pacman
The ||| above separates the final commands only; it is never part of a #CHECK: line.

User tool preferences (ordered by preference, leftmost is most preferred):
{{PREFERENCES}}

Private data placeholders — use these when the command references user/host/home:
- {{USER}} for the current username
- {{HOSTNAME}} for the machine hostname
- {{HOME}} for the home directory path
The tool will replace these with real values locally after you respond.

System context:
{{SYSTEM_CONTEXT}}"#;
