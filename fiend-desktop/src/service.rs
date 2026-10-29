use std::path::{Path, PathBuf};

pub const MAC_PLIST: &str = "/Library/LaunchDaemons/dev.fiend.daemon.plist";
pub const LINUX_UNIT: &str = "/etc/systemd/system/fiend.service";

pub fn is_installed() -> bool {
    if cfg!(target_os = "macos") {
        Path::new(MAC_PLIST).exists()
    } else if cfg!(target_os = "linux") {
        Path::new(LINUX_UNIT).exists()
            || Path::new("/usr/lib/systemd/system/fiend.service").exists()
    } else {
        false
    }
}

pub fn daemon_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "fiend-daemon.exe"
    } else {
        "fiend-daemon"
    };
    let candidate = exe.parent()?.join(name);
    candidate.is_file().then_some(candidate)
}

#[cfg(target_os = "macos")]
fn elevate(daemon: &Path, arguments: &str) -> std::io::Result<std::process::Output> {
    let command = format!(
        "'{}' {arguments}",
        daemon.display().to_string().replace('\'', "'\\''")
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        command.replace('\\', "\\\\").replace('"', "\\\"")
    );
    std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
}

#[cfg(target_os = "linux")]
fn elevate(daemon: &Path, arguments: &str) -> std::io::Result<std::process::Output> {
    let mut command = std::process::Command::new("pkexec");
    command.arg(daemon);
    command.args(arguments.split_whitespace());
    command.output()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn elevate(_: &Path, _: &str) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Service setup is not available on this system yet",
    ))
}

fn run(arguments: &str) -> Result<(), String> {
    let daemon = daemon_binary().ok_or("The daemon file is not next to the app")?;
    let output = elevate(&daemon, arguments).map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    if text.contains("User canceled") || text.contains("dismissed") {
        return Err("The password prompt was cancelled".into());
    }
    let line = text.lines().last().unwrap_or("The command failed").trim();
    Err(line.to_owned())
}

pub fn install() -> Result<(), String> {
    run("install")
}

pub fn uninstall(purge: bool) -> Result<(), String> {
    run(if purge {
        "uninstall --purge"
    } else {
        "uninstall"
    })
}
