use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub const MAC_PLIST: &str = "/Library/LaunchDaemons/dev.fiend.daemon.plist";
#[cfg(target_os = "macos")]
pub const MAC_HELPER: &str = "/Library/PrivilegedHelperTools/dev.fiend.daemon";
#[cfg(target_os = "linux")]
pub const LINUX_UNIT: &str = "/etc/systemd/system/fiend.service";
#[cfg(target_os = "linux")]
pub const LINUX_HELPER: &str = "/usr/local/libexec/fiend/fiend-daemon";

#[cfg(windows)]
pub(crate) fn powershell_path() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

#[cfg(windows)]
fn windows_helper() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join("Fiend")
        .join("fiend-daemon.exe")
}

pub fn is_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        Path::new(MAC_PLIST).exists()
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(LINUX_UNIT).exists()
            || Path::new("/usr/lib/systemd/system/fiend.service").exists()
    }
    #[cfg(windows)]
    {
        windows_helper().is_file()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
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
    #[cfg(target_os = "macos")]
    let path = PathBuf::from(MAC_HELPER);
    #[cfg(target_os = "linux")]
    let path = PathBuf::from(LINUX_HELPER);
    #[cfg(windows)]
    let path = windows_helper();
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return None;
    #[allow(unreachable_code)]
    Some(path).filter(|path| path.is_file())
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

#[cfg(windows)]
fn elevate(daemon: &Path, arguments: &str) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;

    fn quoted(text: &str) -> String {
        format!("'{}'", text.replace('\'', "''"))
    }

    let script = format!(
        "$ErrorActionPreference='Stop'; try {{ $p = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -Wait -PassThru -WindowStyle Hidden; exit $p.ExitCode }} catch {{ Write-Error $_; exit 1 }}",
        quoted(&daemon.to_string_lossy()),
        quoted(arguments)
    );
    let mut command = std::process::Command::new(powershell_path());
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ]);
    command.creation_flags(0x0800_0000);
    command.output()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn elevate(_: &Path, _: &str) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Service setup is not available on this system yet",
    ))
}

#[cfg(windows)]
fn error_file() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "fiend-install-error-{}-{stamp}.txt",
        std::process::id()
    ))
}

fn run(arguments: &str) -> Result<(), String> {
    let daemon = daemon_binary().ok_or("The daemon file is not next to the app")?;
    #[cfg(windows)]
    let error_path = error_file();
    #[cfg(windows)]
    let arguments = format!("{arguments} --error-file \"{}\"", error_path.display());
    #[cfg(windows)]
    let arguments = arguments.as_str();
    let output = elevate(&daemon, arguments).map_err(|error| error.to_string())?;
    if output.status.success() {
        #[cfg(windows)]
        let _ = std::fs::remove_file(&error_path);
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    let lower = text.to_lowercase();
    if lower.contains("user canceled")
        || lower.contains("user cancelled")
        || lower.contains("canceled by the user")
        || lower.contains("cancelled by the user")
        || lower.contains("dismissed")
    {
        return Err("The administrator prompt was cancelled".into());
    }
    #[cfg(windows)]
    if let Ok(recorded) = std::fs::read_to_string(&error_path) {
        let _ = std::fs::remove_file(&error_path);
        let recorded = recorded.trim();
        if !recorded.is_empty() {
            return Err(recorded.chars().take(4096).collect());
        }
    }
    let line = text.lines().last().unwrap_or("The command failed").trim();
    Err(line.to_owned())
}

pub fn install() -> Result<(), String> {
    #[cfg(windows)]
    {
        let sid = fiend_ipc::current_user_sid().map_err(|error| error.to_string())?;
        run(&format!("install --user-sid {sid}"))
    }
    #[cfg(not(windows))]
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
