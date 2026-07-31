use std::path::Path;

use crate::config::{AutoUpdate, home_dir};
use crate::llm::make_client;
use crate::ui::{print_error, print_info, Spinner};
use rust_i18n::t;

// ── Version check & self-update ─────────────────────────────────────────────

fn get_latest_version() -> Result<(String, String), String> {
    let client = make_client()?;
    let resp = client
        .get("https://api.github.com/repos/miuzel/comma-cli/releases/latest")
        .header("User-Agent", format!("comma/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .map_err(|e| t!("update.github_api_error", "e" => e).to_string())?;
    if !resp.status().is_success() {
        return Err(t!("update.github_http_error", "status" => resp.status()).to_string());
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| t!("update.github_api_error", "e" => e).to_string())?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| t!("update.github_missing_tag").to_string())?;
    let version = tag.strip_prefix('v').unwrap_or(tag).to_string();
    Ok((version, tag.to_string()))
}

fn version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.').filter_map(|s| s.parse().ok()).collect()
    };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv > cv { return true; }
        if lv < cv { return false; }
    }
    false
}

fn detect_platform() -> Option<&'static str> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return None;
    };
    // Leak a small string to return &'static — acceptable for a few known values
    Some(Box::leak(format!("{}-{}", os, arch).into_boxed_str()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// Verify the downloaded archive against sha256sums.txt from the same release.
/// Fails on mismatch or missing entry — never install an unverified binary.
fn verify_archive(
    client: &reqwest::blocking::Client,
    archive_name: &str,
    bytes: &[u8],
    current: &str,
) -> Result<(), String> {
    let url = "https://github.com/miuzel/comma-cli/releases/latest/download/sha256sums.txt";
    let resp = client
        .get(url)
        .header("User-Agent", format!("comma/{}", current))
        .send()
        .map_err(|e| t!("update.checksum_download_error", "e" => e).to_string())?;
    if !resp.status().is_success() {
        return Err(t!("update.checksum_http_error", "status" => resp.status()).to_string());
    }
    let sums = resp.text().map_err(|e| t!("update.checksum_download_error", "e" => e).to_string())?;

    // Lines look like: `<sha256>  <archive-name>` (`*name` in binary mode)
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            if name.trim_start_matches('*') == archive_name { Some(hash.to_string()) } else { None }
        })
        .ok_or_else(|| t!("update.checksum_no_entry", "name" => archive_name).to_string())?;

    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(t!("update.checksum_mismatch", "name" => archive_name, "expected" => expected, "actual" => actual).to_string());
    }
    print_info(&t!("update.checksum_verified", "hash" => &actual[..12]));
    Ok(())
}

pub fn do_update() {
    let current = env!("CARGO_PKG_VERSION");
    print_info(&t!("update.checking", "v" => current));

    let (latest, _tag) = match get_latest_version() {
        Ok(v) => v,
        Err(e) => { print_error(&e); return; }
    };

    if !version_newer(&latest, current) {
        print_info(&t!("update.up_to_date", "v" => current));
        return;
    }

    println!("{}", t!("update.available", "from" => current, "to" => latest));

    let platform = match detect_platform() {
        Some(p) => p,
        None => { print_error(&t!("update.unsupported_platform")); return; }
    };

    // Determine binary path
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => { print_error(&t!("update.cannot_find_binary", e = e)); return; }
    };

    // Download platform archive
    let (archive_name, is_zip) = if cfg!(target_os = "windows") {
        (format!("comma-windows-x86_64.zip"), true)
    } else {
        (format!("comma-{}.tar.gz", platform), false)
    };
    let download_url = format!(
        "https://github.com/miuzel/comma-cli/releases/latest/download/{}",
        archive_name
    );

    let mut spinner = Spinner::start(&t!("update.downloading", "name" => archive_name));
    let client = match make_client() {
        Ok(c) => c,
        Err(e) => { spinner.stop(); print_error(&e); return; }
    };
    let resp = match client
        .get(&download_url)
        .header("User-Agent", format!("comma/{}", current))
        .send()
    {
        Ok(r) => r,
        Err(e) => { spinner.stop(); print_error(&t!("update.download_error", e = e)); return; }
    };
    if !resp.status().is_success() {
        spinner.stop();
        print_error(&t!("update.download_http_error", "status" => resp.status()));
        return;
    }
    let bytes = match resp.bytes() {
        Ok(b) => b,
        Err(e) => { spinner.stop(); print_error(&t!("update.download_error", e = e)); return; }
    };
    spinner.stop();

    // Verify integrity before touching the filesystem
    if let Err(e) = verify_archive(&client, &archive_name, &bytes, current) {
        print_error(&e);
        return;
    }

    // Extract binary from archive to temp dir (same filesystem as binary for rename)
    let tmp_dir = exe_path.parent().unwrap_or(Path::new(".")).join(".comma-update");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        print_error(&t!("update.create_temp_dir", "e" => e));
        return;
    }

    let archive_path = tmp_dir.join(&archive_name);
    if let Err(e) = std::fs::write(&archive_path, &bytes) {
        print_error(&t!("update.write_archive", "e" => e));
        return;
    }

    let extracted_binary = if is_zip {
        // Use PowerShell to extract on Windows
        let status = std::process::Command::new("powershell")
            .args(["-Command", &format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive_path.display(), tmp_dir.display()
            )])
            .status();
        match status {
            Ok(s) if s.success() => tmp_dir.join("comma.exe"),
            _ => { print_error(&t!("update.extract_zip_failed")); return; }
        }
    } else {
        // Use tar on Unix
        let status = std::process::Command::new("tar")
            .args(["xzf", archive_path.to_str().unwrap(), "-C", tmp_dir.to_str().unwrap()])
            .status();
        match status {
            Ok(s) if s.success() => tmp_dir.join("comma"),
            _ => { print_error(&t!("update.extract_tar_failed")); return; }
        }
    };

    if !extracted_binary.exists() {
        print_error(&t!("update.binary_not_found"));
        return;
    }

    // Replace binary
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&extracted_binary, std::fs::Permissions::from_mode(0o755));
    }
    // On Windows the running exe is locked. Rename it out of the way first.
    let old_path = exe_path.with_extension("old");
    let _ = std::fs::remove_file(&old_path); // clean up previous .old
    if let Err(_e) = std::fs::rename(&exe_path, &old_path) {
        // Rename of running exe failed, try direct copy (Unix or unlocked Windows)
        if let Err(e) = std::fs::copy(&extracted_binary, &exe_path) {
            print_error(&t!("update.replace_binary", "e" => e));
            return;
        }
    } else {
        // Old exe renamed, copy new one into place
        if let Err(e) = std::fs::copy(&extracted_binary, &exe_path) {
            // Restore old exe on failure
            let _ = std::fs::rename(&old_path, &exe_path);
            print_error(&t!("update.replace_binary", "e" => e));
            return;
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);

    print_info(&t!("update.updated", "v" => latest));
}

// ── Auto-update check ───────────────────────────────────────────────────────

/// Path to the file storing the last update-check timestamp (Unix epoch secs).
fn last_check_path() -> Option<std::path::PathBuf> {
    let home = home_dir().ok()?;
    Some(std::path::PathBuf::from(&home).join(".local/bin/,.last_update_check"))
}

/// Read the stored timestamp, or 0 if missing/unreadable.
fn read_last_check() -> u64 {
    let path = match last_check_path() {
        Some(p) => p,
        None => return 0,
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Write the current timestamp. Errors are silently ignored.
fn write_last_check() {
    let path = match last_check_path() {
        Some(p) => p,
        None => return,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(&path, now.to_string());
}

/// Check for updates if enough time has passed. Prints a one-line notice when
/// a newer version is available; does NOT auto-install.
///
/// Call this after the main work (command execution) is done.
pub fn check_and_notify(auto_update: AutoUpdate) {
    if !auto_update.enabled() {
        return;
    }

    let interval_secs = auto_update.interval_days() * 86400;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let last = read_last_check();
    if now.saturating_sub(last) < interval_secs {
        return;
    }

    // Timestamp check passed — do the actual version probe.
    write_last_check();

    let current = env!("CARGO_PKG_VERSION");
    let (latest, _tag) = match get_latest_version() {
        Ok(v) => v,
        Err(_) => return, // Silent: network errors are not user-facing here
    };

    if version_newer(&latest, current) {
        print_info(&t!("info.update_available", "from" => current, "to" => latest));
    }
}
