//! Opt-in REPL input-history persistence.
//!
//! The feature is off by default (`"history": false`, or the key absent): when
//! disabled, nothing is ever read from or written to disk, so no history file
//! is created — the entries are raw user intents, so they are treated as
//! private data (privacy by design). When enabled, each REPL session loads
//! `$XDG_STATE_HOME/comma/history` (`%APPDATA%\comma\history` on Windows) into
//! the prompt editor and rewrites the file on exit, capped at `MAX_HISTORY`
//! entries and readable by the owner only (0600).
//!
//! Only interactive prompt inputs are recorded here. Text typed in the
//! in-session `e` (edit) / `r` (refine) prompts is added to the rustyline
//! editor's in-memory history by `ui::edit_or_execute` but never reaches this
//! module, so it is not persisted.

use std::path::{Path, PathBuf};

/// Newest entries kept on disk (and seeded back into the editor).
pub const MAX_HISTORY: usize = 1000;

fn truncate_tail(entries: &[String]) -> &[String] {
    let start = entries.len().saturating_sub(MAX_HISTORY);
    &entries[start..]
}

/// Load persisted REPL inputs. A missing, empty, non-UTF-8 or truncated file
/// yields an empty list — a corrupt history must never break startup.
pub fn load(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(data) => truncate_tail(
            &data
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect::<Vec<String>>(),
        )
        .to_vec(),
        Err(_) => Vec::new(),
    }
}

/// Load only when the feature is enabled; a disabled or unresolvable path
/// (HOME missing) never touches the filesystem.
pub fn load_if_enabled(enabled: bool, path: Option<&Path>) -> Vec<String> {
    match (enabled, path) {
        (true, Some(p)) => load(p),
        _ => Vec::new(),
    }
}

/// Overwrite the history file with the newest `MAX_HISTORY` entries, one per
/// line. The file is private (0600), the parent dir is auto-created, and the
/// write is a temp-file + rename so a concurrent REPL exiting at the same time
/// leaves one complete file instead of an interleaved/truncated one. All
/// errors are swallowed: a read-only FS or missing HOME must not break a REPL
/// that is otherwise usable.
pub fn save(path: &Path, entries: &[String]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Entries are single-line by construction; strip any stray newline so the
    // one-entry-per-line format stays parseable.
    let mut body = String::new();
    for entry in truncate_tail(entries) {
        let line = entry.replace(['\n', '\r'], " ");
        let line = line.trim();
        if !line.is_empty() {
            body.push_str(line);
            body.push('\n');
        }
    }

    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    // Publish with a temp file + rename (short-circuit: a failed write is never
    // renamed). Written as a positive condition instead of an early `return` so
    // no platform ends up with a trailing `return` statement — on Windows the
    // `#[cfg(unix)]` block below is compiled out, which made an early return
    // the last statement of the function (`clippy::needless_return`, an error
    // under the CI's `-D warnings`).
    let published = write_private(&tmp, body.as_bytes()) && std::fs::rename(&tmp, path).is_ok();
    if !published {
        let _ = std::fs::remove_file(&tmp);
    }
    // Fix up a pre-existing file whose mode was looser (rename replaced the
    // inode, but an existing loose-mode temp file would survive the open).
    // Unix-only: Windows files inherit the user profile's ACLs instead.
    #[cfg(unix)]
    {
        if published {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Save only when the feature is enabled (see `save`).
pub fn save_if_enabled(enabled: bool, path: Option<&Path>, entries: &[String]) {
    if let (true, Some(p)) = (enabled, path) {
        save(p, entries);
    }
}

/// Create/write a file that is only readable by the owner from the moment it
/// exists (0600), so the contents are never briefly world-readable. On Windows
/// the flag does not apply (the file inherits the user profile's ACLs).
fn write_private(path: &Path, bytes: &[u8]) -> bool {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(path) {
        Ok(mut f) => f.write_all(bytes).is_ok(),
        Err(_) => false,
    }
}
