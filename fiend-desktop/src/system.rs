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
