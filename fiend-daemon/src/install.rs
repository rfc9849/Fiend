#[cfg(windows)]
use std::ffi::OsStr;
use std::io;
use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::Command;
#[cfg(windows)]
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceDependency,
    ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo,
    ServiceStartType, ServiceState, ServiceType,
};
#[cfg(windows)]
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

#[cfg(any(target_os = "macos", windows))]
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

#[cfg(not(windows))]
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

#[cfg(not(windows))]
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

#[cfg(not(windows))]
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
fn mac_service_loaded() -> bool {
    Command::new("launchctl")
        .args(["print", &format!("system/{LABEL}")])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(target_os = "macos")]
fn mac_stop_service() {
    run_quiet("launchctl", &["bootout", &format!("system/{LABEL}")]);
    for _ in 0..50 {
        if !mac_service_loaded() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(target_os = "macos")]
fn mac_start_service() -> io::Result<()> {
    let mut last = None;
    for _ in 0..5 {
        match run("launchctl", &["bootstrap", "system", MAC_PLIST]) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(_) if mac_service_loaded() => {
                last = None;
                break;
            }
            Err(error) => {
                last = Some(error);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    if let Some(error) = last {
        return Err(error);
    }
    run_quiet("launchctl", &["enable", &format!("system/{LABEL}")]);
    run_quiet("launchctl", &["kickstart", &format!("system/{LABEL}")]);
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn install() -> io::Result<()> {
    require_root()?;
    let exe = std::env::current_exe()?;
    mac_stop_service();
    install_file(&exe, Path::new(MAC_HELPER), 0o755)?;
    write_root_file(
        Path::new(MAC_PLIST),
        &mac_plist(Path::new(MAC_HELPER)),
        0o644,
    )?;
    mac_start_service()?;
    println!("Service installed.");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall(purge: bool) -> io::Result<()> {
    require_root()?;
    mac_stop_service();
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

#[cfg(windows)]
const ERROR_FILE_PREFIX: &str = "fiend-install-error-";

#[cfg(windows)]
fn error_file_allowed(path: &Path) -> bool {
    path.is_absolute()
        && path.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name.starts_with(ERROR_FILE_PREFIX) && name.ends_with(".txt")
        })
}

#[cfg(windows)]
pub fn write_error(path: Option<&Path>, error: &io::Error) {
    use std::io::Write;

    let Some(path) = path.filter(|path| error_file_allowed(path)) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        let _ = file.write_all(error.to_string().as_bytes());
    }
}

#[cfg(windows)]
fn require_elevated() -> io::Result<()> {
    if crate::windows::is_elevated()? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Run this command as administrator.",
        ))
    }
}

#[cfg(windows)]
fn service_error(error: windows_service::Error) -> io::Error {
    match error {
        windows_service::Error::Winapi(error) => error,
        other => io::Error::other(other),
    }
}

#[cfg(windows)]
fn service_missing(error: &windows_service::Error) -> bool {
    matches!(
        error,
        windows_service::Error::Winapi(error)
            if error.kind() == io::ErrorKind::NotFound
                || error.raw_os_error() == Some(1060)
    )
}

#[cfg(windows)]
fn marked_for_delete(error: &io::Error) -> bool {
    error.raw_os_error() == Some(1072)
}

#[cfg(windows)]
fn service_manager() -> io::Result<ServiceManager> {
    ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(service_error)
}

#[cfg(windows)]
fn open_service(manager: &ServiceManager, access: ServiceAccess) -> io::Result<Option<Service>> {
    match manager.open_service(crate::windows::SERVICE_NAME, access) {
        Ok(service) => Ok(Some(service)),
        Err(error) if service_missing(&error) => Ok(None),
        Err(error) => Err(service_error(error)),
    }
}

#[cfg(windows)]
fn stop_service(service: &Service) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = service.query_status().map_err(service_error)?;
        match status.current_state {
            ServiceState::Stopped => return Ok(()),
            ServiceState::StopPending => {}
            _ => {
                if let Err(error) = service.stop()
                    && !matches!(
                        error,
                        windows_service::Error::Winapi(ref error)
                            if matches!(error.raw_os_error(), Some(1061 | 1062))
                    )
                {
                    return Err(service_error(error));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "The service did not stop.",
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
fn wait_deleted(manager: &ServiceManager) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match manager.open_service(crate::windows::SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
            Ok(_) => {}
            Err(error) if service_missing(&error) => return Ok(()),
            Err(error) => {
                let error = service_error(error);
                if !marked_for_delete(&error) {
                    return Err(error);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "The old service is still in use. Restart the computer and try again.",
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(windows)]
fn remove_service(manager: &ServiceManager) -> io::Result<()> {
    match open_service(manager, ServiceAccess::ALL_ACCESS) {
        Ok(Some(service)) => {
            stop_service(&service)?;
            if let Err(error) = service.delete()
                && !service_missing(&error)
            {
                let error = service_error(error);
                if !marked_for_delete(&error) {
                    return Err(error);
                }
            }
        }
        Ok(None) => {}
        Err(error) if marked_for_delete(&error) => {}
        Err(error) => return Err(error),
    }
    wait_deleted(manager)
}

#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

#[cfg(windows)]
pub fn install(user_sid: Option<&str>) -> io::Result<()> {
    require_elevated()?;
    let exe = std::env::current_exe()?;
    let wintun = crate::windows::source_wintun().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "wintun.dll is not next to fiend-daemon.exe.",
        )
    })?;
    let manager = service_manager()?;
    remove_service(&manager)?;
    std::fs::create_dir_all(crate::windows::install_dir())?;
    let helper = crate::windows::helper_path();
    if !same_path(&exe, &helper) {
        crate::windows::copy_file(&exe, &helper)?;
    }
    crate::windows::install_wintun(&wintun)?;
    crate::windows::write_allowed_sid(user_sid)?;
    let info = ServiceInfo {
        name: OsStr::new(crate::windows::SERVICE_NAME).to_owned(),
        display_name: OsStr::new("Fiend VPN").to_owned(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: helper,
        launch_arguments: vec![OsStr::new("--service").to_owned()],
        dependencies: ["Tcpip", "Dnscache"]
            .map(|name| ServiceDependency::Service(OsStr::new(name).to_owned()))
            .to_vec(),
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(
            &info,
            ServiceAccess::START
                | ServiceAccess::STOP
                | ServiceAccess::QUERY_STATUS
                | ServiceAccess::CHANGE_CONFIG,
        )
        .map_err(service_error)?;
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(5),
                },
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(15),
                },
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(60),
                },
            ]),
        })
        .map_err(service_error)?;
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(service_error)?;
    service.start::<&OsStr>(&[]).map_err(service_error)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = service.query_status().map_err(service_error)?;
        match status.current_state {
            ServiceState::Running => break,
            ServiceState::Stopped => {
                return Err(io::Error::other(format!(
                    "The service stopped at startup: {:?}",
                    status.exit_code
                )));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "The service did not start.",
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("Service installed.");
    Ok(())
}

#[cfg(windows)]
pub fn uninstall(purge: bool) -> io::Result<()> {
    require_elevated()?;
    let manager = service_manager()?;
    remove_service(&manager)?;
    let state = crate::windows::store_dir();
    let _ = crate::platform::native(&state).recover();
    let _ = crate::platform::windows::remove_nrpt();
    let _ = crate::windows::remove_file(&crate::windows::helper_path());
    let _ = crate::windows::remove_file(&crate::windows::wintun_path());
    let _ = std::fs::remove_dir(crate::windows::install_dir());
    if purge {
        let _ = std::fs::remove_dir_all(&state);
    }
    println!("Service removed.");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn install() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Service setup is not available on this system yet.",
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn only_named_error_files_are_written() {
        let dir = std::env::temp_dir();
        assert!(error_file_allowed(&dir.join("fiend-install-error-1-2.txt")));
        assert!(!error_file_allowed(&dir.join("other.txt")));
        assert!(!error_file_allowed(Path::new("fiend-install-error-1.txt")));
        assert!(!error_file_allowed(&dir.join("fiend-install-error-1.dll")));
    }
}
