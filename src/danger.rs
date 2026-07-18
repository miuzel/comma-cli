use crate::ui::split_comment;

// ── Danger detection ────────────────────────────────────────────────────────

/// Substring class: the command is flagged when it contains any of these
/// (case-insensitive). Whitespace runs are collapsed to a single space before
/// matching, so spacing variants like `rm  -rf  /` are caught as well.
const DANGER_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf /*",
    "dd if=",
    "of=/dev/sd",
    "of=/dev/nvme",
    "mkfs.",
    "wipefs",
    ":(){ :|:& };:",
    "chmod -R 777 /",
    "> /dev/sd",
    "> /dev/nvme",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "sudo rm",
    "git push --force",
    "git push -f",
    "DROP TABLE",
    "DROP DATABASE",
    "FORMAT ",
    "del /f /s /q",
    "rd /s /q",
];

/// Pipe-to-shell class: shells that make a pipeline dangerous when it feeds
/// into them (`curl x | sh`).
const PIPE_SHELLS: &[&str] = &["sh", "bash", "zsh"];

/// Pipe-to-shell class: `sudo`-prefixed shells (`curl x | sudo sh`).
const PIPE_SUDO_SHELLS: &[&str] = &["sh", "bash"];

/// Returns true when a pipe segment runs a shell: the first token is exactly
/// a shell, or `sudo` followed by a shell. `;` and `&` count as token
/// separators so `... | sh; ...` still flags. Exact token matching avoids
/// false positives like `| shuf`, `| sha256sum`, or `| shift`.
fn segment_runs_shell(segment: &str) -> bool {
    let mut tokens = segment
        .split(|c: char| c.is_whitespace() || c == ';' || c == '&')
        .filter(|t| !t.is_empty());
    match tokens.next() {
        Some(shell) if PIPE_SHELLS.contains(&shell) => true,
        Some("sudo") => matches!(tokens.next(), Some(t) if PIPE_SUDO_SHELLS.contains(&t)),
        _ => false,
    }
}

/// Danger detection uses two matching classes, both applied case-insensitively
/// to the whitespace-normalized command:
/// 1. Substring matching against `DANGER_PATTERNS` (broad, for fixed phrases).
/// 2. Pipe-to-shell matching: split on `|` and check each segment's first
///    token(s) exactly, so `| sh` is caught but `| shuf` is not.
pub fn is_dangerous(cmd: &str) -> bool {
    let (command, _) = split_comment(cmd);
    let lower = command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if DANGER_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
    {
        return true;
    }
    lower.split('|').any(segment_runs_shell)
}
