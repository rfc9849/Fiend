use std::path::{Path, PathBuf};

pub const MAC_PLIST: &str = "/Library/LaunchDaemons/dev.fiend.daemon.plist";
pub const MAC_HELPER: &str = "/Library/PrivilegedHelperTools/dev.fiend.daemon";
pub const LINUX_UNIT: &str = "/etc/systemd/system/fiend.service";
pub const LINUX_HELPER: &str = "/usr/local/libexec/fiend/fiend-daemon";

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

fn installed_helper() -> Option<PathBuf> {
    let path = if cfg!(target_os = "macos") {
        MAC_HELPER
    } else if cfg!(target_os = "linux") {
        LINUX_HELPER
    } else {
        return None;
    };
    Some(PathBuf::from(path)).filter(|path| path.is_file())
}

pub fn needs_update() -> bool {
    let (Some(bundled), Some(helper)) = (daemon_binary(), installed_helper()) else {
        return false;
    };
    files_differ(&bundled, &helper)
}

fn files_differ(a: &Path, b: &Path) -> bool {
    let (Ok(meta_a), Ok(meta_b)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if meta_a.len() != meta_b.len() {
        return true;
    }
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(data_a), Ok(data_b)) => data_a != data_b,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_files_are_compared_by_content() {
        let dir = std::env::temp_dir().join(format!("fiend-service-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
        std::fs::write(&a, b"daemon one").unwrap();
        std::fs::write(&b, b"daemon one").unwrap();
        std::fs::write(&c, b"daemon two").unwrap();
        assert!(!files_differ(&a, &b));
        assert!(files_differ(&a, &c));
        assert!(!files_differ(&a, &dir.join("missing")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
