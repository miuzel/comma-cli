use std::path::Path;

use crate::llm::make_client;
use crate::ui::{print_error, print_info, Spinner};

// ── Version check & self-update ─────────────────────────────────────────────

fn get_latest_version() -> Result<(String, String), String> {
    let client = make_client()?;
    let resp = client
        .get("https://api.github.com/repos/miuzel/comma-cli/releases/latest")
        .header("User-Agent", format!("comma/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .map_err(|e| format!("GitHub API: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API: HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("GitHub API: {}", e))?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or("GitHub API: missing tag_name")?;
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
        .map_err(|e| format!("Download sha256sums.txt: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Download sha256sums.txt: HTTP {}", resp.status()));
    }
    let sums = resp.text().map_err(|e| format!("Download sha256sums.txt: {}", e))?;

    // Lines look like: `<sha256>  <archive-name>` (`*name` in binary mode)
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            if name.trim_start_matches('*') == archive_name { Some(hash.to_string()) } else { None }
        })
        .ok_or_else(|| format!("sha256sums.txt has no entry for {}", archive_name))?;

    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(format!(
            "Checksum mismatch for {} (expected {}, got {}) — aborting update",
            archive_name, expected, actual
        ));
    }
    print_info(&format!("Checksum verified ({}...)", &actual[..12]));
    Ok(())
}

pub fn do_update() {
    let current = env!("CARGO_PKG_VERSION");
    print_info(&format!("Checking for updates (current: {})...", current));

    let (latest, _tag) = match get_latest_version() {
        Ok(v) => v,
        Err(e) => { print_error(&e); return; }
    };

    if !version_newer(&latest, current) {
        print_info(&format!("Already up to date ({})", current));
        return;
    }

    println!("  Update available: {} → {}", current, latest);

    let platform = match detect_platform() {
        Some(p) => p,
        None => { print_error("Unsupported platform for auto-update"); return; }
    };

    // Determine binary path
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => { print_error(&format!("Cannot find binary path: {}", e)); return; }
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

    let mut spinner = Spinner::start(&format!("Downloading {}...", archive_name));
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
        Err(e) => { spinner.stop(); print_error(&format!("Download: {}", e)); return; }
    };
    if !resp.status().is_success() {
        spinner.stop();
        print_error(&format!("Download: HTTP {}", resp.status()));
        return;
    }
    let bytes = match resp.bytes() {
        Ok(b) => b,
        Err(e) => { spinner.stop(); print_error(&format!("Download: {}", e)); return; }
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
        print_error(&format!("Create temp dir: {}", e));
        return;
    }

    let archive_path = tmp_dir.join(&archive_name);
    if let Err(e) = std::fs::write(&archive_path, &bytes) {
        print_error(&format!("Write archive: {}", e));
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
            _ => { print_error("Failed to extract zip archive"); return; }
        }
    } else {
        // Use tar on Unix
        let status = std::process::Command::new("tar")
            .args(["xzf", archive_path.to_str().unwrap(), "-C", tmp_dir.to_str().unwrap()])
            .status();
        match status {
            Ok(s) if s.success() => tmp_dir.join("comma"),
            _ => { print_error("Failed to extract tar archive"); return; }
        }
    };

    if !extracted_binary.exists() {
        print_error("Binary not found in archive");
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
            print_error(&format!("Replace binary: {}", e));
            return;
        }
    } else {
        // Old exe renamed, copy new one into place
        if let Err(e) = std::fs::copy(&extracted_binary, &exe_path) {
            // Restore old exe on failure
            let _ = std::fs::rename(&old_path, &exe_path);
            print_error(&format!("Replace binary: {}", e));
            return;
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);

    print_info(&format!("Updated to {}", latest));
}
