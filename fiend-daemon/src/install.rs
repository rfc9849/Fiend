use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "macos")]
use crate::platform::Platform as _;

pub const LABEL: &str = "dev.fiend.daemon";
pub const MAC_HELPER: &str = "/Library/PrivilegedHelperTools/dev.fiend.daemon";
pub const MAC_PLIST: &str = "/Library/LaunchDaemons/dev.fiend.daemon.plist";
pub const MAC_STORE: &str = "/Library/Application Support/Fiend";
pub const MAC_LOG: &str = "/var/log/fiend-daemon.log";
pub const LINUX_UNIT: &str = "/etc/systemd/system/fiend.service";
pub const LINUX_HELPER: &str = "/usr/local/libexec/fiend/fiend-daemon";
pub const LINUX_STORE: &str = "/var/lib/fiend";
pub const SOCKET_DIR: &str = "/var/run/fiend";

pub fn mac_plist(helper: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>{MAC_LOG}</string>
    <key>StandardOutPath</key>
    <string>{MAC_LOG}</string>
</dict>
</plist>
"#,
        xml_escape(&helper.to_string_lossy())
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn linux_unit(helper: &Path) -> String {
    format!(
        "[Unit]\n\
Description=Fiend VPN daemon\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
ExecStart={} run\n\
Restart=on-failure\n\
RestartSec=2\n\
\n\
[Install]\n\
WantedBy=multi-user.target\n",
        helper.display()
    )
}

fn require_root() -> io::Result<()> {
    let output = Command::new("id").arg("-u").output()?;
    if String::from_utf8_lossy(&output.stdout).trim() == "0" {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Run this command as root.",
        ))
    }
}

fn run(program: &str, arguments: &[&str]) -> io::Result<()> {
    let output = Command::new(program).args(arguments).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} {}: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn run_quiet(program: &str, arguments: &[&str]) {
    let _ = Command::new(program).args(arguments).output();
}

#[cfg(unix)]
fn install_file(source: &Path, destination: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755))?;
        std::os::unix::fs::chown(parent, Some(0), Some(0))?;
    }
    let temporary = destination.with_extension("new");
    std::fs::copy(source, &temporary)?;
    std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode))?;
    std::os::unix::fs::chown(&temporary, Some(0), Some(0))?;
    std::fs::rename(&temporary, destination)
}

#[cfg(unix)]
fn write_root_file(path: &Path, content: &str, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    std::os::unix::fs::chown(path, Some(0), Some(0))
}

#[cfg(target_os = "macos")]
pub fn install() -> io::Result<()> {
    require_root()?;
    let exe = std::env::current_exe()?;
    run_quiet("launchctl", &["bootout", &format!("system/{LABEL}")]);
    install_file(&exe, Path::new(MAC_HELPER), 0o755)?;
    write_root_file(
        Path::new(MAC_PLIST),
        &mac_plist(Path::new(MAC_HELPER)),
        0o644,
    )?;
    run("launchctl", &["bootstrap", "system", MAC_PLIST])?;
    run_quiet("launchctl", &["enable", &format!("system/{LABEL}")]);
    run_quiet("launchctl", &["kickstart", &format!("system/{LABEL}")]);
    println!("Service installed.");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall(purge: bool) -> io::Result<()> {
    require_root()?;
    run_quiet("launchctl", &["bootout", &format!("system/{LABEL}")]);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let state = PathBuf::from(MAC_STORE);
    let _ = crate::platform::native(&state).recover();
    for path in [MAC_PLIST, MAC_HELPER, MAC_LOG] {
        remove_file(Path::new(path))?;
    }
    let _ = std::fs::remove_dir_all(SOCKET_DIR);
    if purge {
        let _ = std::fs::remove_dir_all(&state);
    }
    println!("Service removed.");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install() -> io::Result<()> {
    require_root()?;
    let exe = std::env::current_exe()?;
    let packaged = exe.starts_with("/usr/bin") || exe.starts_with("/usr/libexec");
    let helper = if packaged {
        exe.clone()
    } else {
        install_file(&exe, Path::new(LINUX_HELPER), 0o755)?;
        PathBuf::from(LINUX_HELPER)
    };
    run_quiet("groupadd", &["--system", "fiend"]);
    let user = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|uid| {
            std::fs::read_to_string("/etc/passwd")
                .ok()
                .and_then(|text| crate::peer::parse_username(&text, uid.parse().ok()?))
        })
        .or_else(|| std::env::var("SUDO_USER").ok());
    if let Some(user) = user {
        run_quiet("usermod", &["-aG", "fiend", &user]);
    }
    write_root_file(Path::new(LINUX_UNIT), &linux_unit(&helper), 0o644)?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "--now", "fiend.service"])?;
    println!("Service installed. Sign out and in again to use the fiend group.");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall(purge: bool) -> io::Result<()> {
    require_root()?;
    run_quiet("systemctl", &["disable", "--now", "fiend.service"]);
    remove_file(Path::new(LINUX_UNIT))?;
    run_quiet("systemctl", &["daemon-reload"]);
    remove_file(Path::new(LINUX_HELPER))?;
    let _ = std::fs::remove_dir_all(SOCKET_DIR);
    if purge {
        let _ = std::fs::remove_dir_all(LINUX_STORE);
    }
    println!("Service removed.");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Service setup is not available on this system yet.",
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall(_: bool) -> io::Result<()> {
    install()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remove_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_plist_runs_the_helper_at_load() {
        let text = mac_plist(Path::new("/Library/PrivilegedHelperTools/dev.fiend.daemon"));
        assert!(text.contains("<string>dev.fiend.daemon</string>"));
        assert!(text.contains("<string>/Library/PrivilegedHelperTools/dev.fiend.daemon</string>"));
        assert!(text.contains("<string>run</string>"));
        assert!(text.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(text.contains("<key>KeepAlive</key>\n    <true/>"));
    }

    #[test]
    fn plist_escapes_the_path() {
        assert!(mac_plist(Path::new("/a&b/<c>")).contains("/a&amp;b/&lt;c&gt;"));
    }

    #[test]
    fn systemd_unit_starts_at_boot_and_restarts() {
        let text = linux_unit(Path::new("/usr/bin/fiend-daemon"));
        assert!(text.contains("ExecStart=/usr/bin/fiend-daemon run"));
        assert!(text.contains("WantedBy=multi-user.target"));
        assert!(text.contains("Restart=on-failure"));
    }
}
