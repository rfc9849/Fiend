pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('\\', "\\\\").replace('"', "\\\""),
            title.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args([title, body])
            .spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (title, body);
    }
}

pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let launcher = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("Fiend")
        .set_app_path(&exe.to_string_lossy())
        .build()
        .map_err(|error| error.to_string())?;
    if enabled {
        launcher.enable().map_err(|error| error.to_string())
    } else if launcher.is_enabled().unwrap_or(false) {
        launcher.disable().map_err(|error| error.to_string())
    } else {
        Ok(())
    }
}
