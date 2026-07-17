use crate::ui::split_comment;

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

pub fn is_dangerous(cmd: &str) -> bool {
    let (command, _) = split_comment(cmd);
    let lower = command.to_lowercase();
    DANGER_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}
