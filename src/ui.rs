use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use rustyline::completion::{Completer, FilenameCompleter};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};
use std::borrow::Cow;
use std::io::{self, Write};

use crate::danger::is_dangerous;

// ── Rustyline helper wrapper ────────────────────────────────────────────────

pub struct FileHelper {
    completer: FilenameCompleter,
}

impl FileHelper {
    pub fn new() -> Self {
        Self {
            completer: FilenameCompleter::new(),
        }
    }
}

impl Helper for FileHelper {}
impl Validator for FileHelper {}

impl Completer for FileHelper {
    type Candidate = <FilenameCompleter as Completer>::Candidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        self.completer.complete(line, pos, ctx)
    }
}

impl Hinter for FileHelper {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for FileHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Borrowed(hint)
    }
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }
}

// ── Verbosity ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct Verbosity(pub u8);

impl Verbosity {
    pub fn show_prompt(&self) -> bool {
        self.0 >= 1
    }
    pub fn show_debug(&self) -> bool {
        self.0 >= 2
    }
}

// ── Edit action ─────────────────────────────────────────────────────────────

pub enum EditAction {
    Execute(String),
    Refine(String),
    Cancel,
}

// ── Display helpers ─────────────────────────────────────────────────────────

/// Split "command # comment" into (command, Some(comment)) or (cmd, None).
/// Handles cases where # appears inside quotes or is the comment marker.
/// Check if a command is comment-only (no actual command, just a # comment)
pub fn is_comment_only(cmd: &str) -> bool {
    let (command, _) = split_comment(cmd);
    command.is_empty()
}

pub fn split_comment(raw: &str) -> (&str, Option<&str>) {
    // Find the first unquoted #
    let bytes = raw.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut prev = b'\0';
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double && prev != b'\\' => in_single = !in_single,
            b'"' if !in_single && prev != b'\\' => in_double = !in_double,
            b'#' if !in_single && !in_double => {
                let comment = raw[i + 1..].trim();
                if comment.is_empty() {
                    return (raw.trim(), None);
                }
                return (raw[..i].trim(), Some(comment));
            }
            _ => {}
        }
        prev = b;
    }
    (raw.trim(), None)
}

pub fn print_cmd(cmd: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let (command, comment) = split_comment(cmd);

    if is_dangerous(command) {
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
        command,
        ResetColor
    );
    if let Some(cmt) = comment {
        let _ = write!(
            out,
            "  {}# {}{}",
            SetForegroundColor(Color::DarkGrey),
            cmt,
            ResetColor
        );
    }
    let _ = writeln!(out);
}

/// Split LLM output by ||| delimiter into candidate commands.
pub fn parse_candidates(raw: &str) -> Vec<String> {
    let candidates: Vec<String> = raw
        .split("|||")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if candidates.is_empty() {
        vec![raw.trim().to_string()]
    } else {
        candidates
    }
}

/// Interactive selector for multiple candidates.
/// Returns the index of the selected candidate, or None if cancelled.
pub fn select_command(candidates: &[String]) -> Option<usize> {
    if candidates.len() <= 1 {
        return Some(0);
    }

    // Non-interactive (piped): just pick the first candidate
    if !atty::is(atty::Stream::Stdin) {
        print_cmd(&candidates[0]);
        return Some(0);
    }

    let mut selected: usize = 0;
    let len = candidates.len() as u16;

    // Print initial candidates. After drawing, the cursor always sits exactly
    // one row below the last candidate — whether or not the terminal scrolled
    // (no scroll: the cursor moved down `len` rows; scroll: it is clamped at
    // the bottom row with the `len` candidates immediately above it). So the
    // list reliably occupies the `len` rows directly above the cursor, and
    // redraws can use relative movement only (scroll-proof, unlike tracking
    // an absolute start row).
    draw_candidates(candidates, selected);
    let _ = io::stdout().flush();

    let _ = crossterm::terminal::enable_raw_mode();

    let result = loop {
        if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if selected > 0 {
                        selected -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected < candidates.len() - 1 {
                        selected += 1;
                    }
                }
                KeyCode::Tab => {
                    selected = (selected + 1) % candidates.len();
                }
                KeyCode::BackTab => {
                    selected = if selected == 0 {
                        candidates.len() - 1
                    } else {
                        selected - 1
                    };
                }
                KeyCode::Enter => {
                    let _ = crossterm::execute!(
                        io::stdout(),
                        crossterm::cursor::MoveUp(len),
                        crossterm::cursor::MoveToColumn(0),
                        crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
                    );
                    let _ = crossterm::terminal::disable_raw_mode();
                    return Some(selected);
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Esc | KeyCode::Char('q') => break None,
                _ => {}
            }
            // Move back up to the first candidate row, clear and redraw.
            // MoveToColumn(0) is needed because draw_candidates ends lines
            // with a bare '\n', which in raw mode leaves the cursor column
            // wherever the last line's text ended.
            let _ = crossterm::execute!(
                io::stdout(),
                crossterm::cursor::MoveUp(len),
                crossterm::cursor::MoveToColumn(0),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown),
            );
            draw_candidates(candidates, selected);
            let _ = io::stdout().flush();
        }
    };

    let _ = crossterm::terminal::disable_raw_mode();
    result
}

fn draw_candidates(candidates: &[String], selected: usize) {
    let mut out = io::stdout().lock();
    for (i, cmd) in candidates.iter().enumerate() {
        let (command, comment) = split_comment(cmd);
        let marker = if i == selected { "▸" } else { " " };
        let color = if is_dangerous(command) {
            Color::Red
        } else if i == selected {
            Color::Green
        } else {
            Color::DarkGrey
        };
        let _ = write!(out, "\r{}{} ", SetForegroundColor(Color::Cyan), marker);
        let _ = write!(out, "{}{}{}", SetForegroundColor(color), command, ResetColor);
        if let Some(cmt) = comment {
            let _ = write!(out, "  {}# {}{}", SetForegroundColor(Color::DarkGrey), cmt, ResetColor);
        }
        if is_dangerous(command) {
            let _ = write!(out, " {}⚠{}", SetForegroundColor(Color::Red), ResetColor);
        }
        let _ = writeln!(out);
    }
    let _ = out.flush();
}

// ── Spinner ─────────────────────────────────────────────────────────────────

pub struct Spinner {
    handle: Option<std::thread::JoinHandle<()>>,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Spinner {
    pub fn start(msg: &str) -> Self {
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let msg = msg.to_string();
        let running_clone = running.clone();

        let handle = std::thread::spawn(move || {
            let mut i = 0;
            while running_clone.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = crossterm::execute!(
                    io::stdout(),
                    crossterm::cursor::SavePosition,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                );
                let _ = write!(
                    io::stdout(),
                    "\r{}{} {}{}",
                    SetForegroundColor(Color::Cyan),
                    frames[i % frames.len()],
                    msg,
                    ResetColor,
                );
                let _ = io::stdout().flush();
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            // Clear the spinner line
            let _ = crossterm::execute!(
                io::stdout(),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            );
            let _ = write!(io::stdout(), "\r");
            let _ = io::stdout().flush();
        });

        Self {
            handle: Some(handle),
            running,
        }
    }

    pub fn stop(&mut self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn print_info(msg: &str) {
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

pub fn print_error(msg: &str) {
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

pub fn print_debug(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in msg.lines() {
        let _ = write!(
            out,
            "{}│{} {}",
            SetForegroundColor(Color::DarkGrey),
            ResetColor,
            line
        );
        let _ = writeln!(out);
    }
}

pub fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Largest char boundary at or below `max` — slicing mid-char would panic.
        let end = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= max)
            .last()
            .unwrap_or(0);
        &s[..end]
    }
}

pub fn prompt_confirm(msg: &str) -> bool {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(
        out,
        "{}{}{} [Enter/y/N] ",
        SetForegroundColor(Color::Yellow),
        msg,
        ResetColor
    );
    let _ = out.flush();
    drop(out);

    // Fallback to line-based input when stdin is not a TTY (piped)
    if !atty::is(atty::Stream::Stdin) {
        let mut input = String::new();
        return io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y");
    }

    let _ = crossterm::terminal::enable_raw_mode();
    let result = loop {
        if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
            match code {
                KeyCode::Enter => break true,
                KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                KeyCode::Char('n') | KeyCode::Char('N') => break false,
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break false,
                KeyCode::Esc => break false,
                _ => {}
            }
        }
    };
    let _ = crossterm::terminal::disable_raw_mode();
    result
}

pub fn edit_or_execute(cmd: &str, rl: &mut Editor<FileHelper, DefaultHistory>) -> EditAction {
    print_cmd(cmd);

    if !atty::is(atty::Stream::Stdin) {
        // Non-interactive stdin: never auto-execute — require an explicit "y"
        // via the line-based fallback in prompt_confirm.
        return if prompt_confirm("Execute?") {
            EditAction::Execute(cmd.to_string())
        } else {
            EditAction::Cancel
        };
    }

    let prompt_text = if is_dangerous(cmd) {
        "Execute this dangerous command? [Enter] exec / [e]dit / [r]efine / [Esc] cancel "
    } else {
        "Execute? [Enter] exec / [e]dit / [r]efine / [Esc] cancel "
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = write!(
        out,
        "{}{}{}",
        SetForegroundColor(Color::Yellow),
        prompt_text,
        ResetColor
    );
    let _ = out.flush();
    drop(out);

    let _ = crossterm::terminal::enable_raw_mode();
    let action = loop {
        if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
            match code {
                KeyCode::Enter => {
                    break EditAction::Execute(cmd.to_string());
                }
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    break EditAction::Execute(cmd.to_string());
                }
                KeyCode::Char('e') => {
                    let _ = crossterm::terminal::disable_raw_mode();
                    let edit_prompt = format!("{}edit> {}", SetForegroundColor(Color::Yellow), ResetColor);
                    match rl.readline_with_initial(&edit_prompt, (cmd, "")) {
                        Ok(edited) => {
                            let trimmed = edited.trim().to_string();
                            if trimmed.is_empty() || trimmed == cmd {
                                break EditAction::Execute(cmd.to_string());
                            }
                            let _ = rl.add_history_entry(&trimmed);
                            break EditAction::Execute(trimmed);
                        }
                        Err(_) => break EditAction::Cancel,
                    }
                }
                KeyCode::Char('r') => {
                    let _ = crossterm::terminal::disable_raw_mode();
                    let refine_prompt = format!("{}refine> {}", SetForegroundColor(Color::Yellow), ResetColor);
                    match rl.readline(&refine_prompt) {
                        Ok(text) => {
                            let trimmed = text.trim().to_string();
                            if trimmed.is_empty() {
                                break EditAction::Cancel;
                            }
                            let _ = rl.add_history_entry(&trimmed);
                            break EditAction::Refine(trimmed);
                        }
                        Err(_) => break EditAction::Cancel,
                    }
                }
                _ => break EditAction::Cancel,
            }
        }
    };
    let _ = crossterm::terminal::disable_raw_mode();
    action
}

pub fn prompt_input(rl: &mut Editor<FileHelper, DefaultHistory>) -> Option<String> {
    let prompt = format!("{}> {}", SetForegroundColor(Color::Cyan), ResetColor);
    match rl.readline(&prompt) {
        Ok(line) => {
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                let _ = rl.add_history_entry(&trimmed);
                Some(trimmed)
            }
        }
        Err(rustyline::error::ReadlineError::Interrupted)
        | Err(rustyline::error::ReadlineError::Eof) => None,
        Err(_) => None,
    }
}

pub fn prompt_input_fallback() -> Option<String> {
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

// ── Clipboard ───────────────────────────────────────────────────────────────

pub fn copy_to_clipboard(text: &str) {
    let tools: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
        // Windows built-in (clip.exe reads stdin); listed last since the
        // Unix tools won't exist on Windows anyway.
        ("clip", &[]),
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
    let hint = if cfg!(target_os = "windows") {
        "clip.exe is normally built in"
    } else {
        "install wl-clipboard, xclip, or xsel"
    };
    print_error(&format!("No clipboard tool found ({}).", hint));
}
