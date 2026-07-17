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

fn get_kernel() -> String {
    run_cmd("uname", &["-srmo"]).unwrap_or_else(|| "unknown".into())
}

fn get_arch() -> String {
    run_cmd("uname", &["-m"]).unwrap_or_else(|| "unknown".into())
}

fn get_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
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
    let user_pkgs = get_user_packages();
    if !user_pkgs.is_empty() {
        sections.push(format!("[User-installed packages: {}]", user_pkgs.join(", ")));
    }

    sections.join("\n")
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
    let distro = get_distro();
    let kernel = get_kernel();
    let arch = get_arch();
    let shell = get_shell();
    let home = home_dir().unwrap_or_default();
    let user = get_user();

    let cwd_raw = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    // Replace home path and username occurrences in CWD
    let cwd = cwd_raw
        .replace(&home, "{{HOME}}")
        .replace(&user, "{{USER}}");

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
