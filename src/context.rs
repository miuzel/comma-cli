use crate::config::home_dir;

// ── System context ──────────────────────────────────────────────────────────

pub fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn get_distro() -> String {
    // Try /etc/os-release
    if let Some(content) = read_file("/etc/os-release") {
        let name = content
            .lines()
            .find(|l| l.starts_with("PRETTY_NAME="))
            .and_then(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string());
        if let Some(n) = name {
            return n;
        }
    }
    // Try lsb_release
    run_cmd("lsb_release", &["-ds"]).unwrap_or_else(|| "Linux (unknown distro)".into())
}

/// Kernel string and architecture from a single `uname -srm` spawn.
/// `-o` is intentionally omitted: it's a GNU extension that fails on macOS.
fn get_kernel_arch() -> (String, String) {
    let kernel = run_cmd("uname", &["-srm"]).unwrap_or_else(|| "unknown".into());
    // Arch is the last field of the `uname -srm` output.
    let arch = kernel
        .split_whitespace()
        .last()
        .unwrap_or("unknown")
        .to_string();
    (kernel, arch)
}

/// Shell dialect the model should generate for.
/// `COMMA_EVAL_SHELL` wins when set and non-empty: the eval wrapper (README §
/// Shell integration) declares it, because in eval mode the command runs in
/// the wrapper's shell — e.g. PowerShell on Windows, where the SHELL-less
/// default (cmd.exe) would generate the wrong dialect. Otherwise respect
/// SHELL: Git Bash/MSYS users on Windows have it set, and for them POSIX
/// commands are correct. Otherwise Windows commands run via `cmd /C`, so
/// report cmd.exe rather than a Unix shell.
pub fn get_shell() -> String {
    if let Ok(s) = std::env::var("COMMA_EVAL_SHELL") {
        if !s.is_empty() {
            return s;
        }
    }
    std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "windows") {
            "cmd.exe".into()
        } else {
            "/bin/sh".into()
        }
    })
}

fn get_user() -> String {
    run_cmd("whoami", &[])
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "user".into())
}

fn get_hostname() -> String {
    run_cmd("hostname", &[]).unwrap_or_else(|| "localhost".into())
}

fn get_packages() -> String {
    let mut sections: Vec<String> = Vec::new();

    // Detect package manager
    let managers: &[&str] = &["apt", "dnf", "yum", "pacman", "apk", "xbps-install", "zypper", "eopkg"];
    let pkg_mgr = managers.iter().find(|m| run_cmd("which", &[m]).is_some());
    if let Some(mgr) = pkg_mgr {
        sections.push(format!("[Package manager: {}]", mgr));
    }

    // List user-installed packages (non-auto, not part of base system)
    // This is much smaller than listing all PATH executables.
    // Capped so large systems don't blow up the system prompt.
    const MAX_USER_PACKAGES: usize = 200;
    let user_pkgs = get_user_packages();
    let pkg_list: String = user_pkgs.iter().cloned().collect::<Vec<_>>().join(" ");
    if !user_pkgs.is_empty() {
        let mut list = user_pkgs
            .iter()
            .take(MAX_USER_PACKAGES)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if user_pkgs.len() > MAX_USER_PACKAGES {
            list.push_str(&format!(", ... ({} more)", user_pkgs.len() - MAX_USER_PACKAGES));
        }
        sections.push(format!("[User-installed packages: {}]", list));
    }

    // Scan user-local bin directories for standalone executables not
    // already listed by the package manager (e.g. kimi in ~/.kimi-code/bin/).
    let user_bins = get_user_binaries(&pkg_list);
    if !user_bins.is_empty() {
        let list: Vec<&str> = user_bins.iter().map(|s| s.as_str()).collect();
        sections.push(format!("[User binaries: {}]", list.join(", ")));
    }

    sections.join("\n")
}

/// Standalone executables installed outside the system package manager.
/// Scans well-known user-local bin directories and reports tools not already
/// listed by the package manager — e.g. `kimi` in `~/.kimi-code/bin/`.
const LOCAL_BIN_DIRS: &[&str] = &[
    ".local/bin",
    ".cargo/bin",
    "bin",
    ".kimi-code/bin",
    ".opencode/bin",
    ".bun/bin",
    ".local/share/pnpm/bin",
];

fn get_user_binaries(pkg_list: &str) -> Vec<String> {
    let home = match home_dir() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let mut bins: Vec<String> = Vec::new();
    for dir_name in LOCAL_BIN_DIRS {
        let dir = std::path::Path::new(&home).join(dir_name);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                // Skip hidden files, backups, and shell scripts
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if name.starts_with('.') || name.ends_with(".bak") || name.ends_with(".sh") {
                    continue;
                }
                // Skip if already reported by package manager
                if pkg_list.contains(&name) {
                    continue;
                }
                // Check it's actually executable (or likely intended to be)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let ok = std::fs::metadata(&path)
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false);
                    if !ok {
                        continue;
                    }
                }
                if !bins.contains(&name) {
                    bins.push(name);
                }
            }
        }
    }
    bins.sort();
    bins
}

/// Get packages explicitly installed by the user (not auto-installed deps).
fn get_user_packages() -> Vec<String> {
    // Try apt-mark showmanual (Debian/Ubuntu)
    if let Some(output) = run_cmd("apt-mark", &["showmanual"]) {
        let pkgs: Vec<String> = output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if !pkgs.is_empty() {
            return pkgs;
        }
    }
    // Try dnf/yum (RHEL/Fedora)
    if let Some(output) = run_cmd("dnf", &["repoquery", "--userinstalled", "--qf", "%{name}"]) {
        let pkgs: Vec<String> = output.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if !pkgs.is_empty() {
            return pkgs;
        }
    }
    // Try pacman (Arch)
    if let Some(output) = run_cmd("pacman", &["-Qe"]) {
        let pkgs: Vec<String> = output
            .lines()
            .filter_map(|l| l.split_whitespace().next().map(|s| s.to_string()))
            .collect();
        if !pkgs.is_empty() {
            return pkgs;
        }
    }
    Vec::new()
}

/// Non-private system context sent to the API.
/// Sanitizes CWD to avoid leaking username/home path.
pub fn gather_context() -> String {
    gather_context_inner()
}

fn gather_context_inner() -> String {
    let distro = get_distro();
    let (kernel, arch) = get_kernel_arch();
    let shell = get_shell();
    let home = home_dir().unwrap_or_default();
    let user = get_user();

    let cwd_raw = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    // Replace home path and username occurrences in CWD. Skip empty values:
    // str::replace with an empty needle inserts the replacement between
    // every character.
    let mut cwd = cwd_raw;
    if !home.is_empty() {
        cwd = cwd.replace(&home, "{{HOME}}");
    }
    if !user.is_empty() {
        cwd = cwd.replace(&user, "{{USER}}");
    }

    let packages = get_packages();

    format!(
        "Distro: {}\nKernel: {}\nArch: {}\nShell: {}\nCWD: {}\n\nInstalled packages & tools:\n{}",
        distro, kernel, arch, shell, cwd, packages
    )
}

/// Private placeholders — never sent to the API, only substituted locally.
pub struct Placeholders {
    pub user: String,
    pub hostname: String,
    pub home: String,
}

pub fn collect_placeholders() -> Placeholders {
    Placeholders {
        user: get_user(),
        hostname: get_hostname(),
        home: home_dir().unwrap_or_else(|_| "~".into()),
    }
}

/// Replace {{USER}}, {{HOSTNAME}}, {{HOME}} in LLM output with real values.
pub fn apply_placeholders(cmd: &str, ph: &Placeholders) -> String {
    cmd.replace("{{USER}}", &ph.user)
        .replace("{{HOSTNAME}}", &ph.hostname)
        .replace("{{HOME}}", &ph.home)
}
